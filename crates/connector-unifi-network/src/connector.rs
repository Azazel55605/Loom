use std::sync::Mutex;

use async_trait::async_trait;
use futures_util::future::join_all;
use loom_core::connector::{
    details::set_detail, ActionResult, ActionWidgetType, ConnectorAction, ConnectorError,
    ConnectorMetadata, ConnectorStatus, DataPointDescriptor, DataPointValueType, DisplayField,
    DisplayWidgetType, HealthState, NetworkTarget, SubTarget, WidgetBinding, WidgetLayout,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{UniFiNetworkClient, UniFiNetworkConfig, UniFiNetworkError};

pub const TYPE_ID: &str = "unifi-network";
pub const DISPLAY_NAME: &str = "UniFi Network";
pub const ICON: &str = "brand:unifi";

pub const DATA_POINT_DEVICE_COUNT: &str = "deviceCount";
pub const DATA_POINT_ONLINE_DEVICE_COUNT: &str = "onlineDeviceCount";
pub const DATA_POINT_CLIENT_COUNT: &str = "clientCount";
pub const DATA_POINT_STATE: &str = "state";
pub const DATA_POINT_MODEL: &str = "model";
pub const DATA_POINT_UPTIME: &str = "uptime";
pub const ACTION_RESTART: &str = "restart";

const PAGE_LIMIT: usize = 200;

/// One configured local UniFi Network console and selected site.
pub struct UniFiNetworkConnector {
    config: UniFiNetworkConfig,
    client: UniFiNetworkClient,
    site: SiteOverview,
    known_devices: Mutex<Vec<DeviceOverview>>,
}

impl UniFiNetworkConnector {
    /// Validates configuration and proves the API key can see the selected site.
    pub async fn from_config_value(value: Value) -> Result<Self, ConnectorError> {
        let config = UniFiNetworkConfig::from_value(value)?;
        let client =
            UniFiNetworkClient::connect(&config.host, &config.api_key, config.allow_insecure_cert)
                .map_err(connector_error)?;
        let sites: Page<SiteOverview> = client
            .get(&format!("sites?limit={PAGE_LIMIT}"))
            .await
            .map_err(connector_error)?;
        let site = resolve_site(&sites.data, &config.site).ok_or_else(|| {
            let available = sites
                .data
                .iter()
                .map(|site| format!("{} ({})", site.name, site.internal_reference))
                .collect::<Vec<_>>()
                .join(", ");
            ConnectorError::invalid_config(if available.is_empty() {
                format!("site `{}` was not returned by the console", config.site)
            } else {
                format!(
                    "site `{}` was not found; available sites: {available}",
                    config.site
                )
            })
        })?;

        Ok(Self {
            config,
            client,
            site,
            known_devices: Mutex::new(Vec::new()),
        })
    }

    async fn read_status(&self) -> Result<PollReadings, UniFiNetworkError> {
        let devices = self.list_all_devices().await?;
        self.remember_devices(devices.clone());

        // This deliberately fetches detail for every known device, whether or
        // not somebody has placed that target on a dashboard. That is simple
        // and honest for typical homelab device counts; the client's shared
        // semaphore caps the fan-out at ten rather than betting the console's
        // middleware can absorb an unbounded future installation.
        let statistics = join_all(devices.iter().map(|device| {
            let path = format!(
                "sites/{}/devices/{}/statistics/latest",
                self.site.id, device.id
            );
            async move { self.client.get::<DeviceStatistics>(&path).await }
        }));
        let clients_path = format!("sites/{}/clients?limit=1", self.site.id);
        let clients = self.client.get::<CountPage>(&clients_path);
        let (statistics, clients) = tokio::join!(statistics, clients);
        let clients = clients?;

        Ok(PollReadings {
            summary: map_site_summary(&devices, clients.total_count),
            devices: devices
                .into_iter()
                .zip(statistics)
                .map(|(device, statistics)| DeviceReading {
                    device,
                    uptime_seconds: statistics.ok().and_then(|value| value.uptime_sec),
                })
                .collect(),
        })
    }

    async fn list_all_devices(&self) -> Result<Vec<DeviceOverview>, UniFiNetworkError> {
        let mut offset = 0usize;
        let mut devices = Vec::new();
        loop {
            let page: Page<DeviceOverview> = self
                .client
                .get(&format!(
                    "sites/{}/devices?offset={offset}&limit={PAGE_LIMIT}",
                    self.site.id
                ))
                .await?;
            let total_count = page.total_count;
            let count = page.data.len();
            devices.extend(page.data);
            if count == 0 || devices.len() >= total_count {
                break;
            }
            offset += count;
        }
        // The published response requires an id. Ignore a malformed row
        // instead of manufacturing the unusable target id `device:`.
        devices.retain(|device| !device.id.trim().is_empty());
        Ok(devices)
    }

    fn remember_devices(&self, devices: Vec<DeviceOverview>) {
        *self
            .known_devices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = devices;
    }

    async fn list_sub_targets_live(&self) -> Result<Vec<SubTarget>, ConnectorError> {
        let devices = self.list_all_devices().await.map_err(connector_error)?;
        let targets = devices.iter().map(device_sub_target).collect();
        self.remember_devices(devices);
        Ok(targets)
    }
}

#[async_trait]
impl loom_core::connector::Connector for UniFiNetworkConnector {
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
        match self.read_status().await {
            Ok(readings) => {
                let mut details = summary_details(&readings.summary);
                let mut status = ConnectorStatus::new(HealthState::Healthy, Value::Null)
                    .with_target_health(String::new(), HealthState::Healthy);
                for reading in readings.devices {
                    let target_id = device_target_id(&reading.device.id);
                    let state = reading.device.state.as_deref().unwrap_or("UNKNOWN");
                    set_detail(
                        &mut details,
                        Some(&target_id),
                        DATA_POINT_STATE,
                        json!(state),
                    );
                    set_detail(
                        &mut details,
                        Some(&target_id),
                        DATA_POINT_MODEL,
                        json!(device_model(&reading.device)),
                    );
                    set_detail(
                        &mut details,
                        Some(&target_id),
                        DATA_POINT_UPTIME,
                        json!(format_uptime(reading.uptime_seconds)),
                    );
                    status = status.with_target_health(target_id, health_for_device_state(state));
                }
                status.details = details;
                Ok(status)
            }
            Err(error) => {
                let mut details = Value::Object(Map::new());
                set_detail(&mut details, None, "error", json!(error.to_string()));
                Ok(ConnectorStatus::new(HealthState::Down, details)
                    .with_target_health(String::new(), HealthState::Down))
            }
        }
    }

    async fn actions(&self) -> Vec<ConnectorAction> {
        let Ok(devices) = self.list_all_devices().await else {
            return Vec::new();
        };
        self.remember_devices(devices.clone());
        devices
            .into_iter()
            .map(|device| restart_action(&device_target_id(&device.id)))
            .collect()
    }

    async fn execute_action(
        &self,
        action_id: &str,
        target_id: Option<&str>,
        _params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        if action_id != ACTION_RESTART {
            return Err(ConnectorError::invalid_action(action_id));
        }
        let Some(device_id) = target_id.and_then(device_id_from_target) else {
            return Err(ConnectorError::invalid_action(action_id));
        };
        let devices = self.list_all_devices().await.map_err(connector_error)?;
        let Some(device) = devices.iter().find(|device| device.id == device_id) else {
            return Err(ConnectorError::invalid_action(action_id));
        };
        let label = device_label(device);
        self.remember_devices(devices.clone());
        self.client
            .post_json(
                &format!("sites/{}/devices/{device_id}/actions", self.site.id),
                json!({ "action": "RESTART" }),
            )
            .await
            .map_err(connector_error)?;
        Ok(ActionResult::ok(format!(
            "Restart requested for {label}. The device will briefly disconnect attached clients; restarting a gateway may temporarily disconnect the network itself."
        )))
    }

    fn supports_sub_targets(&self) -> bool {
        true
    }

    async fn list_sub_targets(&self) -> Result<Vec<SubTarget>, ConnectorError> {
        self.list_sub_targets_live().await
    }

    fn config_schema(&self) -> Value {
        crate::config_schema()
    }

    fn metadata(&self) -> ConnectorMetadata {
        ConnectorMetadata {
            id: TYPE_ID.to_owned(),
            name: DISPLAY_NAME.to_owned(),
            icon: Some(ICON.to_owned()),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            min_size: (3, 2),
        }
    }

    fn display_fields(&self) -> Vec<DisplayField> {
        vec![
            DisplayField::new("Console", self.config.host.clone()),
            DisplayField::new("Site", self.site.name.clone()),
        ]
    }

    fn data_points(&self) -> Vec<DataPointDescriptor> {
        let devices = self
            .known_devices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        let mut descriptors = vec![
            DataPointDescriptor::new(
                DATA_POINT_DEVICE_COUNT,
                "Devices",
                DataPointValueType::Number,
            ),
            DataPointDescriptor::new(
                DATA_POINT_ONLINE_DEVICE_COUNT,
                "Online devices",
                DataPointValueType::Number,
            ),
            DataPointDescriptor::new(
                DATA_POINT_CLIENT_COUNT,
                "Connected clients",
                DataPointValueType::Number,
            ),
        ];
        descriptors.extend(devices.iter().flat_map(device_data_points));
        descriptors
    }

    fn default_layout(&self) -> WidgetLayout {
        WidgetLayout::new(vec![
            WidgetBinding::display(DATA_POINT_DEVICE_COUNT, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_ONLINE_DEVICE_COUNT, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_CLIENT_COUNT, DisplayWidgetType::StatTile),
        ])
    }

    fn default_layout_for(&self, target_id: Option<&str>) -> WidgetLayout {
        match target_id.and_then(device_id_from_target) {
            Some(_) => WidgetLayout::new(vec![
                WidgetBinding::display(DATA_POINT_STATE, DisplayWidgetType::StatusDot).with_config(
                    json!({
                        "colorMap": {
                            "ONLINE": "healthy",
                            "PENDING_ADOPTION": "degraded",
                            "UPDATING": "degraded",
                            "GETTING_READY": "degraded",
                            "ADOPTING": "degraded",
                            "DELETING": "degraded",
                            "OFFLINE": "down",
                            "CONNECTION_INTERRUPTED": "down",
                            "ISOLATED": "down",
                            "UNKNOWN": "unknown"
                        }
                    }),
                ),
                WidgetBinding::display(DATA_POINT_MODEL, DisplayWidgetType::StatTile),
                WidgetBinding::display(DATA_POINT_UPTIME, DisplayWidgetType::StatTile),
                WidgetBinding::action(ACTION_RESTART, ActionWidgetType::Button),
            ]),
            None => self.default_layout(),
        }
    }

    fn network_target(&self) -> Option<NetworkTarget> {
        self.config.network_target()
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    total_count: usize,
    data: Vec<T>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SiteOverview {
    id: String,
    internal_reference: String,
    name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceOverview {
    // Although the published schema marks `state` required, real consoles can
    // temporarily return it absent or null while a device record is settling.
    // Such a row still contributes to the total, but is not claimed online.
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceStatistics {
    #[serde(default)]
    uptime_sec: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CountPage {
    // Client rows are deliberately not decoded here. The endpoint's
    // polymorphic rows vary by wired/wireless/VPN type and the host-level
    // connector needs only the documented collection total.
    total_count: usize,
}

#[derive(Debug, PartialEq, Eq)]
struct SiteSummary {
    device_count: usize,
    online_device_count: usize,
    client_count: usize,
}

struct PollReadings {
    summary: SiteSummary,
    devices: Vec<DeviceReading>,
}

struct DeviceReading {
    device: DeviceOverview,
    uptime_seconds: Option<u64>,
}

fn resolve_site(sites: &[SiteOverview], requested: &str) -> Option<SiteOverview> {
    sites
        .iter()
        .find(|site| {
            site.id == requested
                || site.internal_reference.eq_ignore_ascii_case(requested)
                || site.name.eq_ignore_ascii_case(requested)
        })
        .cloned()
}

fn map_site_summary(devices: &[DeviceOverview], client_count: usize) -> SiteSummary {
    let online_device_count = devices
        .iter()
        .filter(|device| device.state.as_deref() == Some("ONLINE"))
        .count();
    SiteSummary {
        device_count: devices.len(),
        online_device_count,
        client_count,
    }
}

fn device_target_id(device_id: &str) -> String {
    format!("device:{device_id}")
}

fn device_id_from_target(target_id: &str) -> Option<&str> {
    target_id
        .strip_prefix("device:")
        .filter(|device_id| !device_id.is_empty())
}

fn device_label(device: &DeviceOverview) -> String {
    let model = device_model(device);
    device
        .name
        .as_deref()
        .map(str::trim)
        .filter(|name| {
            !name.is_empty()
                && !name.eq_ignore_ascii_case("device")
                && !name.eq_ignore_ascii_case("unifi device")
                && !name.eq_ignore_ascii_case(&model)
        })
        .map(str::to_owned)
        .unwrap_or(model)
}

fn device_model(device: &DeviceOverview) -> String {
    device
        .model
        .as_deref()
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| "Unknown model".to_owned())
}

fn device_sub_target(device: &DeviceOverview) -> SubTarget {
    SubTarget::new(device_target_id(&device.id), device_label(device)).of_kind("device")
}

fn device_data_points(device: &DeviceOverview) -> Vec<DataPointDescriptor> {
    let target_id = device_target_id(&device.id);
    vec![
        DataPointDescriptor::new(DATA_POINT_STATE, "State", DataPointValueType::String)
            .for_target(&target_id),
        DataPointDescriptor::new(DATA_POINT_MODEL, "Model", DataPointValueType::String)
            .for_target(&target_id),
        DataPointDescriptor::new(DATA_POINT_UPTIME, "Uptime", DataPointValueType::String)
            .for_target(target_id),
    ]
}

fn restart_action(target_id: &str) -> ConnectorAction {
    ConnectorAction::simple(ACTION_RESTART, "Restart device")
        .for_target(target_id)
        .with_description(
            "Restarting disconnects attached clients briefly. Restarting a gateway may temporarily disconnect the network itself.",
        )
        .disruptive()
        .snapshotting([DATA_POINT_STATE])
}

fn health_for_device_state(state: &str) -> HealthState {
    match state {
        "ONLINE" => HealthState::Healthy,
        "PENDING_ADOPTION" | "UPDATING" | "GETTING_READY" | "ADOPTING" | "DELETING" => {
            HealthState::Degraded
        }
        "OFFLINE" | "CONNECTION_INTERRUPTED" | "ISOLATED" => HealthState::Down,
        _ => HealthState::Unknown,
    }
}

fn format_uptime(seconds: Option<u64>) -> String {
    let Some(seconds) = seconds else {
        return "Unavailable".to_owned();
    };
    let days = seconds / 86_400;
    let hours = seconds % 86_400 / 3_600;
    let minutes = seconds % 3_600 / 60;
    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

fn summary_details(summary: &SiteSummary) -> Value {
    let mut details = Value::Object(Map::new());
    set_detail(
        &mut details,
        None,
        DATA_POINT_DEVICE_COUNT,
        json!(summary.device_count),
    );
    set_detail(
        &mut details,
        None,
        DATA_POINT_ONLINE_DEVICE_COUNT,
        json!(summary.online_device_count),
    );
    set_detail(
        &mut details,
        None,
        DATA_POINT_CLIENT_COUNT,
        json!(summary.client_count),
    );
    details
}

fn connector_error(error: UniFiNetworkError) -> ConnectorError {
    match error {
        UniFiNetworkError::ConnectionFailed(reason) => ConnectorError::unreachable(reason),
        UniFiNetworkError::AuthFailed(reason) => ConnectorError::AuthFailed { reason },
        UniFiNetworkError::ApiError { status, message } => ConnectorError::Internal(format!(
            "UniFi Network API returned HTTP {status}: {message}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use loom_core::connector::Connector;

    #[test]
    fn official_site_shape_resolves_by_id_reference_or_name() {
        let page: Page<SiteOverview> = serde_json::from_value(json!({
            "offset": 0,
            "limit": 25,
            "count": 1,
            "totalCount": 1,
            "data": [{
                "id": "5fc12861-86aa-4e14-bb0b-5a4d98b3c003",
                "internalReference": "default",
                "name": "Default"
            }]
        }))
        .expect("official site page shape");

        for requested in ["5fc12861-86aa-4e14-bb0b-5a4d98b3c003", "default", "DEFAULT"] {
            assert_eq!(
                resolve_site(&page.data, requested).map(|site| site.id),
                Some("5fc12861-86aa-4e14-bb0b-5a4d98b3c003".to_owned())
            );
        }
    }

    #[test]
    fn official_device_and_client_shapes_map_to_host_counts() {
        let devices: Page<DeviceOverview> = serde_json::from_value(json!({
            "offset": 0,
            "limit": 200,
            "count": 3,
            "totalCount": 3,
            "data": [
                {"id":"11111111-1111-1111-1111-111111111111","name":"Gateway","model":"UDMPRO","state":"ONLINE","macAddress":"00:00:00:00:00:01","ipAddress":"192.0.2.1","features":["gateway"],"interfaces":["ports"]},
                {"id":"22222222-2222-2222-2222-222222222222","name":"Switch","model":"USW","state":"UPDATING","macAddress":"00:00:00:00:00:02","ipAddress":"192.0.2.2","features":["switching"],"interfaces":["ports"]},
                {"id":"33333333-3333-3333-3333-333333333333","name":"AP","model":"U7PRO","state":"ONLINE","macAddress":"00:00:00:00:00:03","ipAddress":"192.0.2.3","features":["accessPoint"],"interfaces":["radios"]}
            ]
        }))
        .expect("official device page shape");
        let clients: CountPage = serde_json::from_value(json!({
            "offset": 0,
            "limit": 1,
            "count": 1,
            "totalCount": 14,
            "data": [{"type":"WIRED","id":"44444444-4444-4444-4444-444444444444","name":"Example","access":{"type":"DEFAULT"}}]
        }))
        .expect("official client page shape");

        assert_eq!(
            map_site_summary(&devices.data, clients.total_count),
            SiteSummary {
                device_count: 3,
                online_device_count: 2,
                client_count: 14,
            }
        );
    }

    #[test]
    fn temporarily_null_device_state_does_not_break_the_whole_poll() {
        let devices: Page<DeviceOverview> = serde_json::from_value(json!({
            "totalCount": 2,
            "data": [
                {"id":"11111111-1111-1111-1111-111111111111","name":null,"model":null,"state":"ONLINE"},
                {"id":"22222222-2222-2222-2222-222222222222","name":null,"model":null,"state":null}
            ]
        }))
        .expect("real consoles may expose incomplete device metadata transiently");

        assert_eq!(
            map_site_summary(&devices.data, 0),
            SiteSummary {
                device_count: 2,
                online_device_count: 1,
                client_count: 0,
            }
        );
    }

    #[test]
    fn descriptors_include_host_counts_before_devices_are_discovered() {
        let config = UniFiNetworkConfig::from_value(json!({
            "host": "https://console.example.com",
            "apiKey": "not-a-real-key"
        }))
        .expect("config");
        let connector = UniFiNetworkConnector {
            client: UniFiNetworkClient::connect(&config.host, &config.api_key, false)
                .expect("client"),
            config,
            site: SiteOverview {
                id: "5fc12861-86aa-4e14-bb0b-5a4d98b3c003".to_owned(),
                internal_reference: "default".to_owned(),
                name: "Default".to_owned(),
            },
            known_devices: Mutex::new(Vec::new()),
        };

        assert_eq!(connector.metadata().id, TYPE_ID);
        assert_eq!(connector.data_points().len(), 3);
        assert!(connector.supports_sub_targets());
        assert!(connector.resource_kinds(None).is_empty());
        assert!(connector.setup_guide().is_none());
    }

    #[test]
    fn official_device_states_map_to_health_without_guessing() {
        for (state, expected) in [
            ("ONLINE", HealthState::Healthy),
            ("PENDING_ADOPTION", HealthState::Degraded),
            ("UPDATING", HealthState::Degraded),
            ("GETTING_READY", HealthState::Degraded),
            ("ADOPTING", HealthState::Degraded),
            ("DELETING", HealthState::Degraded),
            ("OFFLINE", HealthState::Down),
            ("CONNECTION_INTERRUPTED", HealthState::Down),
            ("ISOLATED", HealthState::Down),
            ("A_FUTURE_STATE", HealthState::Unknown),
        ] {
            assert_eq!(health_for_device_state(state), expected, "state {state}");
        }
    }

    #[test]
    fn device_targets_prefer_a_real_name_and_fall_back_to_model() {
        let named = DeviceOverview {
            id: "device-one".to_owned(),
            name: Some("Workshop AP".to_owned()),
            model: Some("U7 Pro".to_owned()),
            state: Some("ONLINE".to_owned()),
        };
        let generic = DeviceOverview {
            id: "device-two".to_owned(),
            name: Some("Device".to_owned()),
            model: Some("USW Lite".to_owned()),
            state: Some("OFFLINE".to_owned()),
        };

        assert_eq!(device_sub_target(&named).id, "device:device-one");
        assert_eq!(device_sub_target(&named).label, "Workshop AP");
        assert_eq!(device_sub_target(&named).kind, "device");
        assert_eq!(device_sub_target(&generic).label, "USW Lite");
    }

    #[test]
    fn device_descriptors_and_restart_are_scoped_to_the_device() {
        let device = DeviceOverview {
            id: "device-one".to_owned(),
            name: Some("Workshop AP".to_owned()),
            model: Some("U7 Pro".to_owned()),
            state: Some("ONLINE".to_owned()),
        };
        let target_id = "device:device-one";

        let descriptors = device_data_points(&device);
        assert_eq!(descriptors.len(), 3);
        assert!(descriptors
            .iter()
            .all(|descriptor| descriptor.target_id.as_deref() == Some(target_id)));
        let action = restart_action(target_id);
        assert_eq!(action.target_id.as_deref(), Some(target_id));
        assert!(action.is_disruptive);
        assert_eq!(action.snapshot_data_point_ids, [DATA_POINT_STATE]);
    }

    #[test]
    fn uptime_is_human_readable_and_missing_stats_are_honest() {
        assert_eq!(format_uptime(Some(183_780)), "2d 3h 3m");
        assert_eq!(format_uptime(Some(7_500)), "2h 5m");
        assert_eq!(format_uptime(Some(59)), "0m");
        assert_eq!(format_uptime(None), "Unavailable");
    }
}
