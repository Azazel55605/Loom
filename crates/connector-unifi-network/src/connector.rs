use std::sync::Mutex;

use async_trait::async_trait;
use futures_util::future::join_all;
use loom_core::connector::{
    details::set_detail, ActionResult, ActionWidgetType, ApplicableTarget, CapabilityStatus,
    ColumnDescriptor, ColumnValueType, ConnectionTestResult, ConnectorAction, ConnectorError,
    ConnectorMetadata, ConnectorStatus, DataPointDescriptor, DataPointValueType, DisplayField,
    DisplayWidgetType, HealthState, NetworkTarget, ResourceItem, ResourceKindDescriptor,
    SetupGuide, SetupGuideVariant, SubTarget, WidgetBinding, WidgetLayout,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};

#[cfg(test)]
use crate::client::Page;
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
pub const DATA_POINT_CONNECTED_CLIENT_COUNT: &str = "connectedClientCount";
pub const DATA_POINT_RADIOS: &str = "radios";
pub const DATA_POINT_CPU_UTILIZATION: &str = "cpuUtilization";
pub const DATA_POINT_MEMORY_UTILIZATION: &str = "memoryUtilization";
pub const DATA_POINT_UPLINK_RX_RATE: &str = "uplinkRxRate";
pub const DATA_POINT_UPLINK_TX_RATE: &str = "uplinkTxRate";
pub const ACTION_RESTART: &str = "restart";
pub const ACTION_CYCLE_POE: &str = "cyclePoe";
pub const ACTION_AUTHORIZE_GUEST: &str = "authorizeGuest";
pub const ACTION_CREATE_VOUCHER: &str = "createVoucher";
pub const ACTION_REVOKE_VOUCHER: &str = "revokeVoucher";

pub const RESOURCE_KIND_PORTS: &str = "ports";
pub const RESOURCE_KIND_CLIENTS: &str = "clients";
pub const RESOURCE_KIND_VOUCHERS: &str = "vouchers";

pub const CAPABILITY_READ_DEVICES: &str = "readDevices";
pub const CAPABILITY_READ_CLIENTS: &str = "readClients";
pub const CAPABILITY_RESTART: &str = ACTION_RESTART;
pub const CAPABILITY_CYCLE_POE: &str = ACTION_CYCLE_POE;
pub const CAPABILITY_AUTHORIZE_GUEST: &str = ACTION_AUTHORIZE_GUEST;
pub const CAPABILITY_CREATE_VOUCHER: &str = ACTION_CREATE_VOUCHER;
pub const CAPABILITY_REVOKE_VOUCHER: &str = ACTION_REVOKE_VOUCHER;

const PAGE_LIMIT: usize = 200;
const VOUCHER_PAGE_LIMIT: usize = 1_000;
const RESOURCE_ID_PARAM: &str = "resourceId";

/// Setup instructions published with the connector type catalog.
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        variants: vec![SetupGuideVariant {
            id: "api-key".to_owned(),
            label: "Connect via API key".to_owned(),
            description: "UniFi Network 9.1.105 or newer is required for the official Integration API. In UniFi Network, open Settings > Control Plane > Integrations, create an API key, and enter it in Loom's API key field. Local consoles commonly present self-signed certificates: enable allowInsecureCert only after verifying the console when its certificate is not trusted by this host. This opt-in relaxes certificate verification for this connector instance; TLS encryption always remains enabled and Loom never sends the API key over plaintext transport."
                .to_owned(),
            // API-key creation is an instruction-only flow, not a deployment
            // template. The shared guide UI omits an empty template surface.
            template: String::new(),
            toggles: Vec::new(),
            capability_requirements: Vec::new(),
        }],
    }
}

/// One configured local UniFi Network console and selected site.
pub struct UniFiNetworkConnector {
    config: UniFiNetworkConfig,
    client: UniFiNetworkClient,
    site: SiteOverview,
    known_devices: Mutex<Vec<DeviceOverview>>,
    known_clients: Mutex<Vec<ClientOverview>>,
}

impl UniFiNetworkConnector {
    /// Validates configuration and proves the API key can see the selected site.
    pub async fn from_config_value(value: Value) -> Result<Self, ConnectorError> {
        let config = UniFiNetworkConfig::from_value(value)?;
        let client =
            UniFiNetworkClient::connect(&config.host, &config.api_key, config.allow_insecure_cert)
                .map_err(connector_error)?;
        let sites = client
            .fetch_all_pages::<SiteOverview>("sites", PAGE_LIMIT)
            .await
            .map_err(connector_error)?;
        let site = resolve_site(&sites, &config.site).ok_or_else(|| {
            let available = sites
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
            known_clients: Mutex::new(Vec::new()),
        })
    }

    /// Builds the throwaway connector used by the setup connection check.
    /// Network I/O intentionally begins inside `test_connection`, so a failed
    /// site-list request becomes a structured `reachable: false` result.
    pub fn from_config_value_for_connection_test(value: Value) -> Result<Self, ConnectorError> {
        let config = UniFiNetworkConfig::from_value(value)?;
        let client =
            UniFiNetworkClient::connect(&config.host, &config.api_key, config.allow_insecure_cert)
                .map_err(connector_error)?;
        let requested_site = config.site.clone();
        Ok(Self {
            config,
            client,
            site: SiteOverview {
                id: String::new(),
                internal_reference: requested_site.clone(),
                name: requested_site,
            },
            known_devices: Mutex::new(Vec::new()),
            known_clients: Mutex::new(Vec::new()),
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
        let device_readings = join_all(devices.iter().map(|device| {
            let statistics_path = format!(
                "sites/{}/devices/{}/statistics/latest",
                self.site.id, device.id
            );
            let details_path = format!("sites/{}/devices/{}", self.site.id, device.id);
            async move {
                tokio::join!(
                    self.client.get::<DeviceStatistics>(&statistics_path),
                    self.client.get::<DeviceDetails>(&details_path),
                )
            }
        }));
        let clients = self.list_all_clients();
        let (device_readings, clients) = tokio::join!(device_readings, clients);
        let clients = clients?;
        self.remember_clients(clients.clone());

        Ok(PollReadings {
            summary: map_site_summary(&devices, clients.len()),
            devices: devices
                .into_iter()
                .zip(device_readings)
                .map(|(device, (statistics, details))| DeviceReading {
                    connected_client_count: connected_client_count(&clients, &device.id),
                    device,
                    statistics: statistics.ok(),
                    details: details.ok(),
                })
                .collect(),
        })
    }

    async fn list_all_devices(&self) -> Result<Vec<DeviceOverview>, UniFiNetworkError> {
        self.list_all_devices_for_site(&self.site.id).await
    }

    async fn list_all_devices_for_site(
        &self,
        site_id: &str,
    ) -> Result<Vec<DeviceOverview>, UniFiNetworkError> {
        let mut devices = self
            .client
            .fetch_all_pages::<DeviceOverview>(&format!("sites/{site_id}/devices"), PAGE_LIMIT)
            .await?;
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

    fn device_snapshot(&self) -> Vec<DeviceOverview> {
        self.known_devices
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn device_type_for_target(&self, target_id: &str) -> DeviceType {
        let Some(device_id) = device_id_from_target(target_id) else {
            return DeviceType::NetworkDevice;
        };
        self.device_snapshot()
            .iter()
            .find(|device| device.id == device_id)
            .map(device_type)
            .unwrap_or(DeviceType::NetworkDevice)
    }

    async fn list_all_clients(&self) -> Result<Vec<ClientOverview>, UniFiNetworkError> {
        self.list_all_clients_for_site(&self.site.id).await
    }

    async fn list_all_clients_for_site(
        &self,
        site_id: &str,
    ) -> Result<Vec<ClientOverview>, UniFiNetworkError> {
        let mut clients = self
            .client
            .fetch_all_pages::<ClientOverview>(&format!("sites/{site_id}/clients"), PAGE_LIMIT)
            .await?;
        clients.retain(|client| !client.id.trim().is_empty());
        Ok(clients)
    }

    fn remember_clients(&self, clients: Vec<ClientOverview>) {
        *self
            .known_clients
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = clients;
    }

    async fn clients_for_resources(&self) -> Result<Vec<ClientOverview>, ConnectorError> {
        let cached = self
            .known_clients
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if !cached.is_empty() {
            return Ok(cached);
        }
        let clients = self.list_all_clients().await.map_err(connector_error)?;
        self.remember_clients(clients.clone());
        Ok(clients)
    }

    async fn list_all_vouchers(&self) -> Result<Vec<VoucherOverview>, UniFiNetworkError> {
        let mut vouchers = self
            .client
            .fetch_all_pages::<VoucherOverview>(
                &format!("sites/{}/hotspot/vouchers", self.site.id),
                VOUCHER_PAGE_LIMIT,
            )
            .await?;
        vouchers.retain(|voucher| !voucher.id.trim().is_empty());
        Ok(vouchers)
    }

    async fn list_sub_targets_live(&self) -> Result<Vec<SubTarget>, ConnectorError> {
        let devices = self.list_all_devices().await.map_err(connector_error)?;
        let targets = device_sub_targets(&devices);
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
                        json!(format_uptime(
                            reading
                                .statistics
                                .as_ref()
                                .and_then(|value| value.uptime_sec)
                        )),
                    );
                    if let Some(statistics) = &reading.statistics {
                        set_optional_number(
                            &mut details,
                            &target_id,
                            DATA_POINT_CPU_UTILIZATION,
                            statistics.cpu_utilization_pct,
                        );
                        set_optional_number(
                            &mut details,
                            &target_id,
                            DATA_POINT_MEMORY_UTILIZATION,
                            statistics.memory_utilization_pct,
                        );
                        if let Some(uplink) = &statistics.uplink {
                            set_optional_number(
                                &mut details,
                                &target_id,
                                DATA_POINT_UPLINK_RX_RATE,
                                uplink.rx_rate_bps.map(|value| value as f64),
                            );
                            set_optional_number(
                                &mut details,
                                &target_id,
                                DATA_POINT_UPLINK_TX_RATE,
                                uplink.tx_rate_bps.map(|value| value as f64),
                            );
                        }
                    }
                    if device_type(&reading.device) == DeviceType::AccessPoint {
                        set_detail(
                            &mut details,
                            Some(&target_id),
                            DATA_POINT_CONNECTED_CLIENT_COUNT,
                            json!(reading.connected_client_count),
                        );
                        if let Some(radios) = reading
                            .details
                            .as_ref()
                            .map(|details| format_radios(&details.interfaces.radios))
                            .filter(|radios| !radios.is_empty())
                        {
                            set_detail(
                                &mut details,
                                Some(&target_id),
                                DATA_POINT_RADIOS,
                                json!(radios),
                            );
                        }
                    }
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

    async fn test_connection(&self) -> ConnectionTestResult {
        let sites = match self
            .client
            .fetch_all_pages::<SiteOverview>("sites", PAGE_LIMIT)
            .await
        {
            Ok(sites) => sites,
            Err(error) => return unreachable_connection(error.to_string()),
        };
        let Some(site) = resolve_site(&sites, &self.config.site) else {
            return unreachable_connection(format!(
                "site `{}` was not returned by the console",
                self.config.site
            ));
        };

        let (devices, clients) = tokio::join!(
            self.list_all_devices_for_site(&site.id),
            self.list_all_clients_for_site(&site.id),
        );
        connection_test_from_reads(devices, clients)
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
        params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        match action_id {
            ACTION_RESTART => {
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
            ACTION_CYCLE_POE => {
                let Some(device_id) = target_id.and_then(device_id_from_target) else {
                    return Err(ConnectorError::invalid_action(action_id));
                };
                let port_idx = required_resource_id(action_id, &params)?
                    .parse::<u32>()
                    .map_err(|_| invalid_param(action_id, "`resourceId` must be a port number"))?;
                self.client
                    .post_json(
                        &format!(
                            "sites/{}/devices/{device_id}/interfaces/ports/{port_idx}/actions",
                            self.site.id
                        ),
                        json!({ "action": "POWER_CYCLE" }),
                    )
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok(format!(
                    "PoE power-cycle requested for port {port_idx}. The powered device and anything behind it will disconnect briefly."
                )))
            }
            ACTION_AUTHORIZE_GUEST => {
                if target_id.is_some() {
                    return Err(ConnectorError::invalid_action(action_id));
                }
                let client_id = required_resource_id(action_id, &params)?;
                let clients = self.clients_for_resources().await?;
                if !clients.iter().any(|client| client.id == client_id) {
                    return Err(invalid_param(
                        action_id,
                        "`resourceId` is not a connected client returned by this site",
                    ));
                }
                self.client
                    .post_json(
                        &format!("sites/{}/clients/{client_id}/actions", self.site.id),
                        guest_authorization_body(action_id, &params)?,
                    )
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok(
                    "Guest network access was authorized with the requested limits.",
                ))
            }
            ACTION_CREATE_VOUCHER => {
                if target_id.is_some() {
                    return Err(ConnectorError::invalid_action(action_id));
                }
                self.client
                    .post_json(
                        &format!("sites/{}/hotspot/vouchers", self.site.id),
                        voucher_creation_body(action_id, &params)?,
                    )
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok("Created one hotspot voucher."))
            }
            ACTION_REVOKE_VOUCHER => {
                if target_id.is_some() {
                    return Err(ConnectorError::invalid_action(action_id));
                }
                let voucher_id = required_resource_id(action_id, &params)?;
                let vouchers = self.list_all_vouchers().await.map_err(connector_error)?;
                if !vouchers.iter().any(|voucher| voucher.id == voucher_id) {
                    return Err(invalid_param(
                        action_id,
                        "`resourceId` is not a voucher returned by this site",
                    ));
                }
                self.client
                    .delete(&format!(
                        "sites/{}/hotspot/vouchers/{voucher_id}",
                        self.site.id
                    ))
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok("Revoked the hotspot voucher."))
            }
            _ => Err(ConnectorError::invalid_action(action_id)),
        }
    }

    fn resource_kinds(&self, target_id: Option<&str>) -> Vec<ResourceKindDescriptor> {
        match target_id {
            None => vec![clients_kind(), vouchers_kind()],
            Some(target) if device_id_from_target(target).is_some() => vec![ports_kind()],
            _ => Vec::new(),
        }
    }

    async fn list_resource_items(
        &self,
        kind: &str,
        target_id: Option<&str>,
    ) -> Result<Vec<ResourceItem>, ConnectorError> {
        match (kind, target_id) {
            (RESOURCE_KIND_PORTS, Some(target)) => {
                let Some(device_id) = device_id_from_target(target) else {
                    return Ok(Vec::new());
                };
                let details: DeviceDetails = self
                    .client
                    .get(&format!("sites/{}/devices/{device_id}", self.site.id))
                    .await
                    .map_err(connector_error)?;
                Ok(port_resource_items(details.interfaces.ports))
            }
            (RESOURCE_KIND_CLIENTS, None) => {
                let clients = self.clients_for_resources().await?;
                let mut devices = self.device_snapshot();
                if devices.is_empty() {
                    devices = self.list_all_devices().await.map_err(connector_error)?;
                    self.remember_devices(devices.clone());
                }
                Ok(client_resource_items(clients, &devices))
            }
            (RESOURCE_KIND_VOUCHERS, None) => Ok(voucher_resource_items(
                self.list_all_vouchers().await.map_err(connector_error)?,
            )),
            _ => Ok(Vec::new()),
        }
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

    fn setup_guide(&self) -> Option<SetupGuide> {
        Some(setup_guide())
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
            Some(_) => {
                let mut bindings = vec![
                    WidgetBinding::display(DATA_POINT_STATE, DisplayWidgetType::StatusDot)
                        .with_config(json!({
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
                        })),
                    WidgetBinding::display(DATA_POINT_MODEL, DisplayWidgetType::StatTile),
                    WidgetBinding::display(DATA_POINT_UPTIME, DisplayWidgetType::StatTile),
                ];
                match target_id.map_or(DeviceType::NetworkDevice, |target| {
                    self.device_type_for_target(target)
                }) {
                    DeviceType::AccessPoint => bindings.extend([
                        WidgetBinding::display(
                            DATA_POINT_CONNECTED_CLIENT_COUNT,
                            DisplayWidgetType::StatTile,
                        ),
                        WidgetBinding::display(DATA_POINT_RADIOS, DisplayWidgetType::StatTile),
                        WidgetBinding::display(
                            DATA_POINT_CPU_UTILIZATION,
                            DisplayWidgetType::ProgressBar,
                        ),
                        WidgetBinding::display(
                            DATA_POINT_MEMORY_UTILIZATION,
                            DisplayWidgetType::ProgressBar,
                        ),
                    ]),
                    DeviceType::Switch | DeviceType::Gateway => bindings.extend([
                        WidgetBinding::display(
                            DATA_POINT_CPU_UTILIZATION,
                            DisplayWidgetType::ProgressBar,
                        ),
                        WidgetBinding::display(
                            DATA_POINT_MEMORY_UTILIZATION,
                            DisplayWidgetType::ProgressBar,
                        ),
                        WidgetBinding::display(
                            DATA_POINT_UPLINK_RX_RATE,
                            DisplayWidgetType::StatTile,
                        ),
                        WidgetBinding::display(
                            DATA_POINT_UPLINK_TX_RATE,
                            DisplayWidgetType::StatTile,
                        ),
                    ]),
                    DeviceType::NetworkDevice => bindings.extend([
                        WidgetBinding::display(
                            DATA_POINT_CPU_UTILIZATION,
                            DisplayWidgetType::ProgressBar,
                        ),
                        WidgetBinding::display(
                            DATA_POINT_MEMORY_UTILIZATION,
                            DisplayWidgetType::ProgressBar,
                        ),
                    ]),
                }
                bindings.push(WidgetBinding::action(
                    ACTION_RESTART,
                    ActionWidgetType::Button,
                ));
                WidgetLayout::new(bindings)
            }
            None => self.default_layout(),
        }
    }

    fn network_target(&self) -> Option<NetworkTarget> {
        self.config.network_target()
    }
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
    #[serde(default)]
    mac_address: Option<String>,
    /// The official API's authoritative capability categories. Do not infer a
    /// product family from model names: an unknown future model still carries
    /// these feature flags, and a missing flag safely becomes `NetworkDevice`.
    #[serde(default)]
    features: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceStatistics {
    #[serde(default)]
    uptime_sec: Option<u64>,
    #[serde(default)]
    cpu_utilization_pct: Option<f64>,
    #[serde(default)]
    memory_utilization_pct: Option<f64>,
    #[serde(default)]
    uplink: Option<UplinkStatistics>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct UplinkStatistics {
    #[serde(default)]
    tx_rate_bps: Option<u64>,
    #[serde(default)]
    rx_rate_bps: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DeviceDetails {
    #[serde(default)]
    interfaces: DeviceInterfaces,
}

#[derive(Debug, Default, Deserialize)]
struct DeviceInterfaces {
    #[serde(default)]
    ports: Vec<PortOverview>,
    #[serde(default)]
    radios: Vec<RadioOverview>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RadioOverview {
    wlan_standard: String,
    #[serde(rename = "frequencyGHz")]
    frequency_ghz: String,
    #[serde(rename = "channelWidthMHz")]
    channel_width_mhz: u32,
    #[serde(default)]
    channel: Option<u32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortOverview {
    idx: u32,
    state: String,
    #[serde(default)]
    poe: Option<PortPoeOverview>,
}

#[derive(Debug, Deserialize)]
struct PortPoeOverview {
    enabled: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClientOverview {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    mac_address: Option<String>,
    #[serde(default)]
    ip_address: Option<String>,
    #[serde(default)]
    uplink_device_id: Option<String>,
    access: ClientAccessOverview,
}

#[derive(Debug, Clone, Deserialize)]
struct ClientAccessOverview {
    #[serde(rename = "type")]
    access_type: String,
    #[serde(default)]
    authorized: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VoucherOverview {
    id: String,
    code: String,
    created_at: String,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    authorized_guest_limit: Option<u64>,
    authorized_guest_count: u64,
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
    statistics: Option<DeviceStatistics>,
    details: Option<DeviceDetails>,
    connected_client_count: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeviceType {
    AccessPoint,
    Switch,
    Gateway,
    NetworkDevice,
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

fn connected_client_count(clients: &[ClientOverview], device_id: &str) -> usize {
    clients
        .iter()
        .filter(|client| client.uplink_device_id.as_deref() == Some(device_id))
        .count()
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

fn device_sub_targets(devices: &[DeviceOverview]) -> Vec<SubTarget> {
    let base_labels = devices.iter().map(device_label).collect::<Vec<_>>();
    let mut counts = std::collections::HashMap::new();
    for label in &base_labels {
        *counts.entry(label.clone()).or_insert(0usize) += 1;
    }

    devices
        .iter()
        .zip(base_labels)
        .map(|(device, label)| {
            let label = if counts.get(label.as_str()).copied().unwrap_or_default() > 1 {
                format!("{label} ({})", device_disambiguator(device))
            } else {
                label
            };
            SubTarget::new(device_target_id(&device.id), label)
                .of_kind("device")
                .with_icon(device_icon(device))
        })
        .collect()
}

fn device_disambiguator(device: &DeviceOverview) -> String {
    let compact = device
        .mac_address
        .as_deref()
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_hexdigit)
        .collect::<String>()
        .to_ascii_lowercase();
    if compact.len() >= 4 {
        let suffix = &compact[compact.len() - 4..];
        return format!("{}:{}", &suffix[..2], &suffix[2..]);
    }

    // The OpenAPI schema requires a MAC address, but a stable id suffix keeps
    // two malformed real-world rows distinguishable instead of recreating the
    // original collision.
    device
        .id
        .chars()
        .rev()
        .take(6)
        .collect::<String>()
        .chars()
        .rev()
        .collect()
}

fn device_type(device: &DeviceOverview) -> DeviceType {
    if device.features.iter().any(|feature| feature == "gateway") {
        DeviceType::Gateway
    } else if device
        .features
        .iter()
        .any(|feature| feature == "accessPoint")
    {
        DeviceType::AccessPoint
    } else if device.features.iter().any(|feature| feature == "switching") {
        DeviceType::Switch
    } else {
        DeviceType::NetworkDevice
    }
}

fn device_icon(device: &DeviceOverview) -> &'static str {
    match device_type(device) {
        DeviceType::AccessPoint => "lucide:wifi",
        DeviceType::Switch => "lucide:ethernet-port",
        DeviceType::Gateway => "lucide:router",
        DeviceType::NetworkDevice => "lucide:network",
    }
}

fn device_data_points(device: &DeviceOverview) -> Vec<DataPointDescriptor> {
    let target_id = device_target_id(&device.id);
    let mut points = vec![
        DataPointDescriptor::new(DATA_POINT_STATE, "State", DataPointValueType::String)
            .for_target(&target_id),
        DataPointDescriptor::new(DATA_POINT_MODEL, "Model", DataPointValueType::String)
            .for_target(&target_id),
        DataPointDescriptor::new(DATA_POINT_UPTIME, "Uptime", DataPointValueType::String)
            .for_target(&target_id),
        DataPointDescriptor::new(
            DATA_POINT_CPU_UTILIZATION,
            "CPU utilization",
            DataPointValueType::Number,
        )
        .with_unit("%")
        .for_target(&target_id),
        DataPointDescriptor::new(
            DATA_POINT_MEMORY_UTILIZATION,
            "Memory utilization",
            DataPointValueType::Number,
        )
        .with_unit("%")
        .for_target(&target_id),
        DataPointDescriptor::new(
            DATA_POINT_UPLINK_RX_RATE,
            "Uplink receive rate",
            DataPointValueType::Number,
        )
        .with_unit("bps")
        .for_target(&target_id),
        DataPointDescriptor::new(
            DATA_POINT_UPLINK_TX_RATE,
            "Uplink transmit rate",
            DataPointValueType::Number,
        )
        .with_unit("bps")
        .for_target(&target_id),
    ];
    if device_type(device) == DeviceType::AccessPoint {
        points.extend([
            DataPointDescriptor::new(
                DATA_POINT_CONNECTED_CLIENT_COUNT,
                "Connected clients",
                DataPointValueType::Number,
            )
            .for_target(&target_id),
            DataPointDescriptor::new(DATA_POINT_RADIOS, "Radios", DataPointValueType::String)
                .for_target(target_id),
        ]);
    }
    points
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

fn ports_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_PORTS,
        "Ports",
        vec![
            ColumnDescriptor::new("port", "Port", ColumnValueType::Number),
            ColumnDescriptor::new("poeEnabled", "PoE enabled", ColumnValueType::Bool),
            ColumnDescriptor::new("linkStatus", "Link", ColumnValueType::Text),
        ],
    )
    .applicable_to(ApplicableTarget::TargetOnly)
    .with_row_actions(vec![resource_row_action(
        ACTION_CYCLE_POE,
        "Power-cycle PoE",
        "Briefly cuts power to the device attached to this port.",
        true,
    )])
}

fn clients_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_CLIENTS,
        "Clients",
        vec![
            ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
            ColumnDescriptor::new("mac", "MAC", ColumnValueType::Text),
            ColumnDescriptor::new("ipAddress", "IP address", ColumnValueType::Text),
            ColumnDescriptor::new("connectedTo", "Connected to", ColumnValueType::Text),
            ColumnDescriptor::new("isGuest", "Guest", ColumnValueType::Bool),
            ColumnDescriptor::new("authorized", "Authorized", ColumnValueType::Bool),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    .with_row_actions(vec![authorize_guest_action()])
}

fn vouchers_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_VOUCHERS,
        "Vouchers",
        vec![
            ColumnDescriptor::new("code", "Code", ColumnValueType::Text),
            ColumnDescriptor::new("expiresAt", "Expires", ColumnValueType::Timestamp),
            ColumnDescriptor::new("usesRemaining", "Uses remaining", ColumnValueType::Number),
            ColumnDescriptor::new("createdAt", "Created", ColumnValueType::Timestamp),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    .with_row_actions(vec![resource_row_action(
        ACTION_REVOKE_VOUCHER,
        "Revoke",
        "Permanently revoke this hotspot voucher.",
        false,
    )])
    .with_kind_actions(vec![create_voucher_action()])
}

fn authorize_guest_action() -> ConnectorAction {
    ConnectorAction {
        id: ACTION_AUTHORIZE_GUEST.to_owned(),
        target_id: None,
        label: "Authorize guest".to_owned(),
        description: Some(
            "Authorize this guest client, replacing any existing authorization and resetting its traffic counters."
                .to_owned(),
        ),
        params_schema: json!({
            "type": "object",
            "properties": {
                RESOURCE_ID_PARAM: resource_id_schema("Client"),
                "timeLimitMinutes": integer_schema(
                    "Access duration (minutes)",
                    "Optional; the site's default is used when omitted.",
                    1,
                    1_000_000
                ),
                "dataUsageLimitMBytes": integer_schema(
                    "Data limit (MB)",
                    "Optional total data allowance.",
                    1,
                    1_048_576
                ),
                "rxRateLimitKbps": integer_schema(
                    "Download limit (Kbps)",
                    "Optional download rate limit.",
                    2,
                    100_000
                ),
                "txRateLimitKbps": integer_schema(
                    "Upload limit (Kbps)",
                    "Optional upload rate limit.",
                    2,
                    100_000
                )
            },
            "required": [RESOURCE_ID_PARAM],
            "additionalProperties": false
        }),
        is_disruptive: false,
        snapshot_data_point_ids: Vec::new(),
    }
}

fn create_voucher_action() -> ConnectorAction {
    ConnectorAction {
        id: ACTION_CREATE_VOUCHER.to_owned(),
        target_id: None,
        label: "Create voucher".to_owned(),
        description: Some(
            "Create one hotspot voucher with explicit duration and usage limits.".to_owned(),
        ),
        params_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "title": "Name",
                    "description": "A note used to identify the voucher.",
                    "minLength": 1
                },
                "timeLimitMinutes": integer_schema(
                    "Access duration (minutes)",
                    "Time from first activation until the voucher expires.",
                    1,
                    1_000_000
                ),
                "authorizedGuestLimit": {
                    "type": "integer",
                    "title": "Usage quota",
                    "description": "How many different guests may use this voucher.",
                    "minimum": 1
                },
                "dataUsageLimitMBytes": integer_schema(
                    "Data limit (MB)",
                    "Optional total data allowance.",
                    1,
                    1_048_576
                ),
                "rxRateLimitKbps": integer_schema(
                    "Download limit (Kbps)",
                    "Optional download rate limit.",
                    2,
                    100_000
                ),
                "txRateLimitKbps": integer_schema(
                    "Upload limit (Kbps)",
                    "Optional upload rate limit.",
                    2,
                    100_000
                )
            },
            "required": ["name", "timeLimitMinutes", "authorizedGuestLimit"],
            "additionalProperties": false
        }),
        is_disruptive: false,
        snapshot_data_point_ids: Vec::new(),
    }
}

fn resource_row_action(
    id: &str,
    label: &str,
    description: &str,
    is_disruptive: bool,
) -> ConnectorAction {
    ConnectorAction {
        id: id.to_owned(),
        target_id: None,
        label: label.to_owned(),
        description: Some(description.to_owned()),
        params_schema: json!({
            "type": "object",
            "properties": {
                RESOURCE_ID_PARAM: resource_id_schema("Resource")
            },
            "required": [RESOURCE_ID_PARAM],
            "additionalProperties": false
        }),
        is_disruptive,
        snapshot_data_point_ids: Vec::new(),
    }
}

fn resource_id_schema(title: &str) -> Value {
    json!({
        "type": "string",
        "title": title,
        "description": "The selected row; supplied automatically by Loom."
    })
}

fn integer_schema(title: &str, description: &str, minimum: u64, maximum: u64) -> Value {
    json!({
        "type": "integer",
        "title": title,
        "description": description,
        "minimum": minimum,
        "maximum": maximum
    })
}

fn port_resource_items(mut ports: Vec<PortOverview>) -> Vec<ResourceItem> {
    ports.sort_by_key(|port| port.idx);
    ports
        .into_iter()
        .map(|port| {
            ResourceItem::new(port.idx.to_string())
                .with_field("port", port.idx)
                .with_field(
                    "poeEnabled",
                    port.poe.as_ref().is_some_and(|poe| poe.enabled),
                )
                .with_field("linkStatus", port.state)
        })
        .collect()
}

fn client_resource_items(
    mut clients: Vec<ClientOverview>,
    devices: &[DeviceOverview],
) -> Vec<ResourceItem> {
    clients.sort_by(|left, right| {
        client_label(left)
            .to_ascii_lowercase()
            .cmp(&client_label(right).to_ascii_lowercase())
    });
    clients
        .into_iter()
        .map(|client| {
            let is_guest = client.access.access_type == "GUEST";
            let connected_to = client
                .uplink_device_id
                .as_deref()
                .and_then(|id| devices.iter().find(|device| device.id == id))
                .map(device_label)
                .unwrap_or_default();
            ResourceItem::new(client.id.clone())
                .with_field("name", client_label(&client))
                .with_field("mac", client.mac_address.unwrap_or_default())
                .with_field("ipAddress", client.ip_address.unwrap_or_default())
                .with_field("connectedTo", connected_to)
                .with_field("isGuest", is_guest)
                .with_field(
                    "authorized",
                    is_guest && client.access.authorized.unwrap_or(false),
                )
        })
        .collect()
}

fn client_label(client: &ClientOverview) -> String {
    let name = client.name.trim();
    if !name.is_empty() {
        return name.to_owned();
    }
    client
        .mac_address
        .as_deref()
        .or(client.ip_address.as_deref())
        .unwrap_or("Unknown client")
        .to_owned()
}

fn voucher_resource_items(mut vouchers: Vec<VoucherOverview>) -> Vec<ResourceItem> {
    vouchers.sort_by(|left, right| right.created_at.cmp(&left.created_at));
    vouchers
        .into_iter()
        .map(|voucher| {
            let uses_remaining = voucher
                .authorized_guest_limit
                .map(|limit| json!(limit.saturating_sub(voucher.authorized_guest_count)))
                .unwrap_or(Value::Null);
            ResourceItem::new(voucher.id)
                .with_field("code", voucher.code)
                .with_field(
                    "expiresAt",
                    voucher.expires_at.map(Value::String).unwrap_or(Value::Null),
                )
                .with_field("usesRemaining", uses_remaining)
                .with_field("createdAt", voucher.created_at)
        })
        .collect()
}

fn guest_authorization_body(action_id: &str, params: &Value) -> Result<Value, ConnectorError> {
    let mut body = Map::from_iter([(
        "action".to_owned(),
        Value::String("AUTHORIZE_GUEST_ACCESS".to_owned()),
    )]);
    copy_optional_integer_params(
        action_id,
        params,
        &mut body,
        &[
            "timeLimitMinutes",
            "dataUsageLimitMBytes",
            "rxRateLimitKbps",
            "txRateLimitKbps",
        ],
    )?;
    Ok(Value::Object(body))
}

fn voucher_creation_body(action_id: &str, params: &Value) -> Result<Value, ConnectorError> {
    let name = required_string_param(action_id, params, "name")?;
    let time_limit = required_integer_param(action_id, params, "timeLimitMinutes")?;
    let guest_limit = required_integer_param(action_id, params, "authorizedGuestLimit")?;
    let mut body = Map::from_iter([
        ("count".to_owned(), json!(1)),
        ("name".to_owned(), json!(name)),
        ("timeLimitMinutes".to_owned(), json!(time_limit)),
        ("authorizedGuestLimit".to_owned(), json!(guest_limit)),
    ]);
    copy_optional_integer_params(
        action_id,
        params,
        &mut body,
        &["dataUsageLimitMBytes", "rxRateLimitKbps", "txRateLimitKbps"],
    )?;
    Ok(Value::Object(body))
}

fn required_resource_id<'a>(action_id: &str, params: &'a Value) -> Result<&'a str, ConnectorError> {
    required_string_param(action_id, params, RESOURCE_ID_PARAM)
}

fn required_string_param<'a>(
    action_id: &str,
    params: &'a Value,
    key: &str,
) -> Result<&'a str, ConnectorError> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| invalid_param(action_id, format!("`{key}` must be a non-empty string")))
}

fn required_integer_param(
    action_id: &str,
    params: &Value,
    key: &str,
) -> Result<u64, ConnectorError> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .filter(|value| *value > 0)
        .ok_or_else(|| invalid_param(action_id, format!("`{key}` must be a positive integer")))
}

fn copy_optional_integer_params(
    action_id: &str,
    params: &Value,
    body: &mut Map<String, Value>,
    keys: &[&str],
) -> Result<(), ConnectorError> {
    for key in keys {
        let Some(value) = params.get(*key) else {
            continue;
        };
        let number = value.as_u64().filter(|number| *number > 0).ok_or_else(|| {
            invalid_param(action_id, format!("`{key}` must be a positive integer"))
        })?;
        body.insert((*key).to_owned(), json!(number));
    }
    Ok(())
}

fn invalid_param(action_id: &str, reason: impl Into<String>) -> ConnectorError {
    ConnectorError::InvalidParams {
        action_id: action_id.to_owned(),
        reason: reason.into(),
    }
}

fn connection_test_from_reads(
    devices: Result<Vec<DeviceOverview>, UniFiNetworkError>,
    clients: Result<Vec<ClientOverview>, UniFiNetworkError>,
) -> ConnectionTestResult {
    let read_devices = tested_read_capability(
        CAPABILITY_READ_DEVICES,
        "List devices",
        "device listing",
        devices,
    );
    let read_clients = tested_read_capability(
        CAPABILITY_READ_CLIENTS,
        "List clients",
        "client listing",
        clients,
    );
    let all_reads_available = read_devices.available && read_clients.available;
    let authenticated_note = Some(
        "Available after API-key authentication; Test Connection does not perform writes."
            .to_owned(),
    );

    ConnectionTestResult {
        reachable: true,
        capabilities: vec![
            read_devices,
            read_clients,
            available_capability(
                CAPABILITY_RESTART,
                "Restart devices",
                authenticated_note.clone(),
            ),
            available_capability(
                CAPABILITY_CYCLE_POE,
                "Power-cycle PoE ports",
                authenticated_note.clone(),
            ),
            available_capability(
                CAPABILITY_AUTHORIZE_GUEST,
                "Authorize guest clients",
                authenticated_note.clone(),
            ),
            available_capability(
                CAPABILITY_CREATE_VOUCHER,
                "Create vouchers",
                authenticated_note.clone(),
            ),
            available_capability(
                CAPABILITY_REVOKE_VOUCHER,
                "Revoke vouchers",
                authenticated_note,
            ),
        ],
        message: Some(if all_reads_available {
            "Authenticated successfully and verified device and client listings. Write capabilities are available but were not exercised."
                .to_owned()
        } else {
            "Authenticated successfully, but one or more read endpoints could not be verified. Write capabilities are available but were not exercised."
                .to_owned()
        }),
    }
}

fn tested_read_capability<T>(
    key: &str,
    label: &str,
    operation: &str,
    result: Result<T, UniFiNetworkError>,
) -> CapabilityStatus {
    match result {
        Ok(_) => available_capability(key, label, None),
        Err(error) => CapabilityStatus {
            key: key.to_owned(),
            label: label.to_owned(),
            available: false,
            note: Some(format!("{operation} failed: {error}")),
        },
    }
}

fn available_capability(key: &str, label: &str, note: Option<String>) -> CapabilityStatus {
    CapabilityStatus {
        key: key.to_owned(),
        label: label.to_owned(),
        available: true,
        note,
    }
}

fn unreachable_connection(message: impl Into<String>) -> ConnectionTestResult {
    ConnectionTestResult {
        reachable: false,
        capabilities: Vec::new(),
        message: Some(message.into()),
    }
}

fn set_optional_number(
    details: &mut Value,
    target_id: &str,
    data_point_id: &str,
    value: Option<f64>,
) {
    if let Some(value) = value.filter(|value| value.is_finite()) {
        set_detail(details, Some(target_id), data_point_id, json!(value));
    }
}

fn format_radios(radios: &[RadioOverview]) -> String {
    radios
        .iter()
        .map(|radio| {
            let channel = radio
                .channel
                .map_or_else(|| "auto".to_owned(), |channel| channel.to_string());
            format!(
                "{} GHz ch {channel} / {} MHz ({})",
                radio.frequency_ghz, radio.channel_width_mhz, radio.wlan_standard
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
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

    fn capability<'a>(result: &'a ConnectionTestResult, key: &str) -> &'a CapabilityStatus {
        result
            .capabilities
            .iter()
            .find(|capability| capability.key == key)
            .unwrap_or_else(|| panic!("connection test omitted capability {key}"))
    }

    #[test]
    fn setup_guide_describes_the_official_api_key_and_tls_path() {
        let guide = setup_guide();
        assert_eq!(guide.variants.len(), 1);
        let variant = &guide.variants[0];
        assert_eq!(variant.id, "api-key");
        assert_eq!(variant.label, "Connect via API key");
        assert!(variant.description.contains("9.1.105"));
        assert!(variant
            .description
            .contains("Settings > Control Plane > Integrations"));
        assert!(variant.description.contains("self-signed"));
        assert!(variant.description.contains("allowInsecureCert"));
        assert!(variant
            .description
            .contains("TLS encryption always remains enabled"));
        assert!(variant.template.is_empty());
        assert!(variant.toggles.is_empty());
        assert!(variant.capability_requirements.is_empty());
    }

    #[test]
    fn connection_test_probes_reads_and_never_executes_writes() {
        let result = connection_test_from_reads(
            Ok(vec![DeviceOverview {
                id: "device-one".to_owned(),
                name: Some("Switch".to_owned()),
                model: Some("USW".to_owned()),
                state: Some("ONLINE".to_owned()),
                mac_address: Some("00:11:22:33:44:55".to_owned()),
                features: vec!["switching".to_owned()],
            }]),
            Err(UniFiNetworkError::ApiError {
                status: 500,
                message: "temporary client-list failure".to_owned(),
            }),
        );

        assert!(result.reachable);
        assert!(capability(&result, CAPABILITY_READ_DEVICES).available);
        let clients = capability(&result, CAPABILITY_READ_CLIENTS);
        assert!(!clients.available);
        assert!(clients
            .note
            .as_deref()
            .is_some_and(|note| note.contains("temporary client-list failure")));
        for key in [
            CAPABILITY_RESTART,
            CAPABILITY_CYCLE_POE,
            CAPABILITY_AUTHORIZE_GUEST,
            CAPABILITY_CREATE_VOUCHER,
            CAPABILITY_REVOKE_VOUCHER,
        ] {
            let write = capability(&result, key);
            assert!(write.available, "{key}");
            assert!(write
                .note
                .as_deref()
                .is_some_and(|note| note.contains("does not perform writes")));
        }
        assert!(result
            .message
            .as_deref()
            .is_some_and(|message| message.contains("one or more read endpoints")));
    }

    #[test]
    fn connection_failure_is_unreachable_and_claims_no_capabilities() {
        let result = unreachable_connection(
            UniFiNetworkError::AuthFailed("API key was rejected".to_owned()).to_string(),
        );
        assert!(!result.reachable);
        assert!(result.capabilities.is_empty());
        assert!(result
            .message
            .as_deref()
            .is_some_and(|message| message.contains("API key was rejected")));
    }

    #[test]
    fn connection_test_factory_performs_no_network_io() {
        let connector = UniFiNetworkConnector::from_config_value_for_connection_test(json!({
            "host": "https://console.example.com",
            "apiKey": "not-a-real-key"
        }))
        .expect("building the ephemeral connector is local-only");
        assert_eq!(connector.site.internal_reference, "default");
        assert!(connector.setup_guide().is_some());
    }

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
        let clients: Page<ClientOverview> = serde_json::from_value(json!({
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
            known_clients: Mutex::new(Vec::new()),
        };

        assert_eq!(connector.metadata().id, TYPE_ID);
        assert_eq!(connector.data_points().len(), 3);
        assert!(connector.supports_sub_targets());
        assert_eq!(connector.resource_kinds(None).len(), 2);
        assert!(connector.setup_guide().is_some());
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
            mac_address: Some("00:11:22:33:44:55".to_owned()),
            features: vec!["accessPoint".to_owned()],
        };
        let generic = DeviceOverview {
            id: "device-two".to_owned(),
            name: Some("Device".to_owned()),
            model: Some("USW Lite".to_owned()),
            state: Some("OFFLINE".to_owned()),
            mac_address: Some("00:11:22:33:66:77".to_owned()),
            features: vec!["switching".to_owned()],
        };

        let targets = device_sub_targets(&[named, generic]);
        assert_eq!(targets[0].id, "device:device-one");
        assert_eq!(targets[0].label, "Workshop AP");
        assert_eq!(targets[0].kind, "device");
        assert_eq!(targets[0].icon.as_deref(), Some("lucide:wifi"));
        assert_eq!(targets[1].label, "USW Lite");
        assert_eq!(targets[1].icon.as_deref(), Some("lucide:ethernet-port"));
    }

    #[test]
    fn duplicate_device_labels_gain_only_their_mac_suffix() {
        let devices = [
            DeviceOverview {
                id: "device-one".to_owned(),
                name: Some("Device".to_owned()),
                model: Some("U7 Lite".to_owned()),
                state: Some("ONLINE".to_owned()),
                mac_address: Some("68:d7:9a:11:a4:f6".to_owned()),
                features: vec!["accessPoint".to_owned()],
            },
            DeviceOverview {
                id: "device-two".to_owned(),
                name: Some("Device".to_owned()),
                model: Some("U7 Lite".to_owned()),
                state: Some("ONLINE".to_owned()),
                mac_address: Some("68:d7:9a:22:b5:07".to_owned()),
                features: vec!["accessPoint".to_owned()],
            },
            DeviceOverview {
                id: "device-three".to_owned(),
                name: Some("Gateway".to_owned()),
                model: Some("UDM".to_owned()),
                state: Some("ONLINE".to_owned()),
                mac_address: Some("68:d7:9a:33:c6:18".to_owned()),
                features: vec!["gateway".to_owned()],
            },
        ];

        let targets = device_sub_targets(&devices);
        assert_eq!(targets[0].label, "U7 Lite (a4:f6)");
        assert_eq!(targets[1].label, "U7 Lite (b5:07)");
        assert_eq!(targets[2].label, "Gateway");
    }

    #[test]
    fn device_type_uses_official_features_and_never_guesses_from_model() {
        let device = |model: &str, features: &[&str]| DeviceOverview {
            id: model.to_owned(),
            name: None,
            model: Some(model.to_owned()),
            state: Some("ONLINE".to_owned()),
            mac_address: Some("00:11:22:33:44:55".to_owned()),
            features: features
                .iter()
                .map(|feature| (*feature).to_owned())
                .collect(),
        };

        assert_eq!(
            device_type(&device("U7", &["accessPoint"])),
            DeviceType::AccessPoint
        );
        assert_eq!(
            device_type(&device("USW", &["switching"])),
            DeviceType::Switch
        );
        assert_eq!(
            device_type(&device("UDM", &["gateway", "switching"])),
            DeviceType::Gateway
        );
        assert_eq!(device_type(&device("U7", &[])), DeviceType::NetworkDevice);
    }

    #[test]
    fn device_descriptors_and_restart_are_scoped_to_the_device() {
        let device = DeviceOverview {
            id: "device-one".to_owned(),
            name: Some("Workshop AP".to_owned()),
            model: Some("U7 Pro".to_owned()),
            state: Some("ONLINE".to_owned()),
            mac_address: Some("00:11:22:33:44:55".to_owned()),
            features: vec!["accessPoint".to_owned()],
        };
        let target_id = "device:device-one";

        let descriptors = device_data_points(&device);
        assert_eq!(descriptors.len(), 9);
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

    #[test]
    fn documented_statistics_and_radio_details_map_without_invented_fields() {
        let statistics: DeviceStatistics = serde_json::from_value(json!({
            "uptimeSec": 42,
            "cpuUtilizationPct": 12.5,
            "memoryUtilizationPct": 48.25,
            "uplink": {"rxRateBps": 1000, "txRateBps": 2000},
            "interfaces": {"radios": [{"frequencyGHz": "5", "txRetriesPct": 1.2}]}
        }))
        .expect("official device statistics");
        assert_eq!(statistics.cpu_utilization_pct, Some(12.5));
        assert_eq!(statistics.memory_utilization_pct, Some(48.25));
        assert_eq!(
            statistics
                .uplink
                .as_ref()
                .and_then(|uplink| uplink.rx_rate_bps),
            Some(1000)
        );

        let details: DeviceDetails = serde_json::from_value(json!({
            "interfaces": {
                "ports": [],
                "radios": [
                    {
                        "wlanStandard": "802.11be",
                        "frequencyGHz": "6",
                        "channelWidthMHz": 160,
                        "channel": 37
                    },
                    {
                        "wlanStandard": "802.11ax",
                        "frequencyGHz": "2.4",
                        "channelWidthMHz": 20
                    }
                ]
            }
        }))
        .expect("official device detail");
        assert_eq!(
            format_radios(&details.interfaces.radios),
            "6 GHz ch 37 / 160 MHz (802.11be), 2.4 GHz ch auto / 20 MHz (802.11ax)"
        );
    }

    #[test]
    fn official_device_details_map_ports_without_inventing_missing_fields() {
        let details: DeviceDetails = serde_json::from_value(json!({
            "id": "11111111-1111-1111-1111-111111111111",
            "name": "Switch",
            "model": "USW",
            "state": "ONLINE",
            "interfaces": {
                "ports": [
                    {
                        "idx": 2,
                        "state": "DOWN",
                        "connector": "RJ45",
                        "maxSpeedMbps": 1000
                    },
                    {
                        "idx": 1,
                        "state": "UP",
                        "connector": "RJ45",
                        "maxSpeedMbps": 1000,
                        "speedMbps": 1000,
                        "poe": {
                            "standard": "802.3at",
                            "type": 2,
                            "enabled": true,
                            "state": "UP"
                        }
                    }
                ]
            }
        }))
        .expect("official device detail shape");

        let rows = port_resource_items(details.interfaces.ports);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].id, "1");
        assert_eq!(rows[0].fields.get("port"), Some(&json!(1)));
        assert_eq!(rows[0].fields.get("poeEnabled"), Some(&json!(true)));
        assert_eq!(rows[0].fields.get("linkStatus"), Some(&json!("UP")));
        assert_eq!(rows[1].fields.get("poeEnabled"), Some(&json!(false)));
        assert!(!rows[0].fields.contains_key("poePowerWatts"));
    }

    #[test]
    fn official_client_shapes_map_guest_access_and_uplink_labels() {
        let page: Page<ClientOverview> = serde_json::from_value(json!({
            "totalCount": 2,
            "data": [
                {
                    "type": "WIRELESS",
                    "id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                    "name": "Guest phone",
                    "macAddress": "00:00:00:00:00:01",
                    "ipAddress": "192.0.2.20",
                    "uplinkDeviceId": "11111111-1111-1111-1111-111111111111",
                    "access": {"type": "GUEST", "authorized": true}
                },
                {
                    "type": "VPN",
                    "id": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                    "name": "Remote user",
                    "ipAddress": "192.0.2.30",
                    "access": {"type": "DEFAULT"}
                }
            ]
        }))
        .expect("official polymorphic client overview shape");
        let devices = vec![DeviceOverview {
            id: "11111111-1111-1111-1111-111111111111".to_owned(),
            name: Some("Workshop AP".to_owned()),
            model: Some("U7 Pro".to_owned()),
            state: Some("ONLINE".to_owned()),
            mac_address: Some("00:11:22:33:44:55".to_owned()),
            features: vec!["accessPoint".to_owned()],
        }];

        assert_eq!(
            connected_client_count(&page.data, "11111111-1111-1111-1111-111111111111"),
            1
        );

        let rows = client_resource_items(page.data, &devices);
        let guest = rows
            .iter()
            .find(|row| row.id == "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa")
            .expect("guest row");
        assert_eq!(guest.fields.get("name"), Some(&json!("Guest phone")));
        assert_eq!(guest.fields.get("mac"), Some(&json!("00:00:00:00:00:01")));
        assert_eq!(guest.fields.get("connectedTo"), Some(&json!("Workshop AP")));
        assert_eq!(guest.fields.get("isGuest"), Some(&json!(true)));
        assert_eq!(guest.fields.get("authorized"), Some(&json!(true)));
        let vpn = rows
            .iter()
            .find(|row| row.id == "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb")
            .expect("VPN row");
        assert_eq!(vpn.fields.get("mac"), Some(&json!("")));
        assert_eq!(vpn.fields.get("isGuest"), Some(&json!(false)));
        assert_eq!(vpn.fields.get("authorized"), Some(&json!(false)));
    }

    #[test]
    fn official_voucher_shape_maps_remaining_uses_and_nullable_expiry() {
        let page: Page<VoucherOverview> = serde_json::from_value(json!({
            "totalCount": 2,
            "data": [
                {
                    "id": "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
                    "createdAt": "2026-09-04T10:00:00Z",
                    "name": "Visitor",
                    "code": "1234567890",
                    "authorizedGuestLimit": 3,
                    "authorizedGuestCount": 1,
                    "activatedAt": "2026-09-04T10:05:00Z",
                    "expiresAt": "2026-09-04T11:05:00Z",
                    "expired": false,
                    "timeLimitMinutes": 60
                },
                {
                    "id": "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
                    "createdAt": "2026-09-03T10:00:00Z",
                    "name": "Unlimited users",
                    "code": "0987654321",
                    "authorizedGuestCount": 0,
                    "expired": false,
                    "timeLimitMinutes": 60
                }
            ]
        }))
        .expect("official voucher detail page shape");

        let rows = voucher_resource_items(page.data);
        assert_eq!(rows[0].fields.get("code"), Some(&json!("1234567890")));
        assert_eq!(rows[0].fields.get("usesRemaining"), Some(&json!(2)));
        assert_eq!(
            rows[0].fields.get("expiresAt"),
            Some(&json!("2026-09-04T11:05:00Z"))
        );
        assert_eq!(rows[1].fields.get("usesRemaining"), Some(&Value::Null));
        assert_eq!(rows[1].fields.get("expiresAt"), Some(&Value::Null));
    }

    #[test]
    fn resource_kinds_are_scoped_and_publish_only_confirmed_actions() {
        let host = [clients_kind(), vouchers_kind()];
        assert!(host
            .iter()
            .all(|kind| kind.applicable_target == ApplicableTarget::HostOnly));
        assert_eq!(
            host[0]
                .row_actions
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            [ACTION_AUTHORIZE_GUEST]
        );
        assert_eq!(host[1].row_actions[0].id, ACTION_REVOKE_VOUCHER);
        assert_eq!(host[1].kind_actions[0].id, ACTION_CREATE_VOUCHER);

        let ports = ports_kind();
        assert_eq!(ports.applicable_target, ApplicableTarget::TargetOnly);
        assert_eq!(ports.row_actions[0].id, ACTION_CYCLE_POE);
        assert!(ports.row_actions[0].target_id.is_none());
        assert!(ports.row_actions[0].is_disruptive);
    }

    #[test]
    fn write_payloads_match_the_official_discriminated_request_shapes() {
        assert_eq!(
            guest_authorization_body(
                ACTION_AUTHORIZE_GUEST,
                &json!({
                    "resourceId": "not-sent-in-the-body",
                    "timeLimitMinutes": 30,
                    "dataUsageLimitMBytes": 512,
                    "rxRateLimitKbps": 10_000,
                    "txRateLimitKbps": 2_000
                })
            )
            .expect("valid guest authorization"),
            json!({
                "action": "AUTHORIZE_GUEST_ACCESS",
                "timeLimitMinutes": 30,
                "dataUsageLimitMBytes": 512,
                "rxRateLimitKbps": 10_000,
                "txRateLimitKbps": 2_000
            })
        );
        assert_eq!(
            voucher_creation_body(
                ACTION_CREATE_VOUCHER,
                &json!({
                    "name": "Visitor",
                    "timeLimitMinutes": 60,
                    "authorizedGuestLimit": 2,
                    "rxRateLimitKbps": 5_000
                })
            )
            .expect("valid voucher creation"),
            json!({
                "count": 1,
                "name": "Visitor",
                "timeLimitMinutes": 60,
                "authorizedGuestLimit": 2,
                "rxRateLimitKbps": 5_000
            })
        );
    }
}
