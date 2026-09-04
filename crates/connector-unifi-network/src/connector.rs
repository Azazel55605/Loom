use async_trait::async_trait;
use loom_core::connector::{
    details::set_detail, ActionResult, ConnectorAction, ConnectorError, ConnectorMetadata,
    ConnectorStatus, DataPointDescriptor, DataPointValueType, DisplayField, DisplayWidgetType,
    HealthState, NetworkTarget, WidgetBinding, WidgetLayout,
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

const PAGE_LIMIT: usize = 200;

/// One configured local UniFi Network console and selected site.
pub struct UniFiNetworkConnector {
    config: UniFiNetworkConfig,
    client: UniFiNetworkClient,
    site: SiteOverview,
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
        })
    }

    async fn read_status(&self) -> Result<SiteSummary, UniFiNetworkError> {
        let devices = self.list_all_devices().await?;
        let clients: CountPage = self
            .client
            .get(&format!("sites/{}/clients?limit=1", self.site.id))
            .await?;
        Ok(map_site_summary(devices, clients.total_count))
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
        Ok(devices)
    }
}

#[async_trait]
impl loom_core::connector::Connector for UniFiNetworkConnector {
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
        match self.read_status().await {
            Ok(summary) => {
                let details = summary_details(&summary);
                Ok(ConnectorStatus::new(HealthState::Healthy, details)
                    .with_target_health(String::new(), HealthState::Healthy))
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
        // The official API has write endpoints, but all of them address a
        // device, port, client, or voucher. This minimal host-only connector
        // cannot identify a safe target for one without inventing semantics.
        Vec::new()
    }

    async fn execute_action(
        &self,
        action_id: &str,
        _target_id: Option<&str>,
        _params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        Err(ConnectorError::invalid_action(action_id))
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
        vec![
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
        ]
    }

    fn default_layout(&self) -> WidgetLayout {
        WidgetLayout::new(vec![
            WidgetBinding::display(DATA_POINT_DEVICE_COUNT, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_ONLINE_DEVICE_COUNT, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_CLIENT_COUNT, DisplayWidgetType::StatTile),
        ])
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceOverview {
    // Although the published schema marks `state` required, real consoles can
    // temporarily return it absent or null while a device record is settling.
    // Such a row still contributes to the total, but is not claimed online.
    #[serde(default)]
    state: Option<String>,
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

fn map_site_summary(devices: Vec<DeviceOverview>, client_count: usize) -> SiteSummary {
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
            map_site_summary(devices.data, clients.total_count),
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
            map_site_summary(devices.data, 0),
            SiteSummary {
                device_count: 2,
                online_device_count: 1,
                client_count: 0,
            }
        );
    }

    #[test]
    fn descriptors_are_minimal_and_host_only() {
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
        };

        assert_eq!(connector.metadata().id, TYPE_ID);
        assert_eq!(connector.data_points().len(), 3);
        assert!(!connector.supports_sub_targets());
        assert!(connector.resource_kinds(None).is_empty());
        assert!(connector.setup_guide().is_none());
    }
}
