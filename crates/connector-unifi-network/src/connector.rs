use std::{
    collections::{HashMap, HashSet},
    sync::Mutex,
};

use async_trait::async_trait;
use futures_util::future::join_all;
use loom_core::connector::{
    details::set_detail, ActionResult, ActionWidgetType, ApplicableTarget, CapabilityStatus,
    ColumnDescriptor, ColumnValueType, ConnectionTestResult, ConnectorAction, ConnectorError,
    ConnectorMetadata, ConnectorStatus, DataPointDescriptor, DataPointValueType, DisplayField,
    DisplayWidgetType, HealthState, NetworkTarget, ResourceItem, ResourceKindDescriptor,
    SetupGuide, SetupGuideVariant, SubTarget, WidgetBinding, WidgetLayout,
};
use serde::{Deserialize, Deserializer};
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
pub const DATA_POINT_LOAD_AVERAGE_1M: &str = "loadAverage1m";
pub const DATA_POINT_LOAD_AVERAGE_5M: &str = "loadAverage5m";
pub const DATA_POINT_LOAD_AVERAGE_15M: &str = "loadAverage15m";
pub const DATA_POINT_LAST_HEARTBEAT_AT: &str = "lastHeartbeatAt";
pub const DATA_POINT_RADIO_TX_RETRY_PERCENT: &str = "radioTxRetryPercent";
pub const DATA_POINT_WAN_COUNT: &str = "wanCount";
pub const ACTION_RESTART: &str = "restart";
pub const ACTION_CYCLE_POE: &str = "cyclePoe";
pub const ACTION_AUTHORIZE_GUEST: &str = "authorizeGuest";
pub const ACTION_UNAUTHORIZE_GUEST: &str = "unauthorizeGuest";
pub const ACTION_CREATE_VOUCHER: &str = "createVoucher";
pub const ACTION_REVOKE_VOUCHER: &str = "revokeVoucher";
pub const ACTION_ADOPT: &str = "adopt";
pub const ACTION_DELETE_ACL_RULE: &str = "deleteAclRule";
pub const ACTION_DELETE_DNS_POLICY: &str = "deleteDnsPolicy";
pub const ACTION_CREATE_A_RECORD: &str = "createARecord";
pub const ACTION_CREATE_CNAME_RECORD: &str = "createCnameRecord";
pub const ACTION_CREATE_FORWARD_DOMAIN: &str = "createForwardDomain";
pub const ACTION_DELETE_FIREWALL_ZONE: &str = "deleteFirewallZone";
pub const ACTION_CREATE_FIREWALL_ZONE: &str = "createZone";
pub const ACTION_DELETE_FIREWALL_POLICY: &str = "deleteFirewallPolicy";
pub const ACTION_TOGGLE_FIREWALL_LOGGING: &str = "toggleLogging";
pub const ACTION_DELETE_NETWORK: &str = "deleteNetwork";
pub const ACTION_TOGGLE_WLAN_ENABLED: &str = "toggleEnabled";

pub const RESOURCE_KIND_PORTS: &str = "ports";
pub const RESOURCE_KIND_CLIENTS: &str = "clients";
pub const RESOURCE_KIND_VOUCHERS: &str = "vouchers";
pub const RESOURCE_KIND_WANS: &str = "wans";
pub const RESOURCE_KIND_PENDING_DEVICES: &str = "pendingDevices";
pub const RESOURCE_KIND_ACL_RULES: &str = "aclRules";
pub const RESOURCE_KIND_DNS_POLICIES: &str = "dnsPolicies";
pub const RESOURCE_KIND_FIREWALL_ZONES: &str = "firewallZones";
pub const RESOURCE_KIND_FIREWALL_POLICIES: &str = "firewallPolicies";
pub const RESOURCE_KIND_NETWORKS: &str = "networks";
pub const RESOURCE_KIND_WLAN_BROADCASTS: &str = "wlanBroadcasts";
pub const RESOURCE_KIND_VPN_SERVERS: &str = "vpnServers";
pub const RESOURCE_KIND_SITE_TO_SITE_TUNNELS: &str = "siteToSiteTunnels";

pub const CAPABILITY_READ_DEVICES: &str = "readDevices";
pub const CAPABILITY_READ_CLIENTS: &str = "readClients";
pub const CAPABILITY_RESTART: &str = ACTION_RESTART;
pub const CAPABILITY_CYCLE_POE: &str = ACTION_CYCLE_POE;
pub const CAPABILITY_AUTHORIZE_GUEST: &str = ACTION_AUTHORIZE_GUEST;
pub const CAPABILITY_UNAUTHORIZE_GUEST: &str = ACTION_UNAUTHORIZE_GUEST;
pub const CAPABILITY_CREATE_VOUCHER: &str = ACTION_CREATE_VOUCHER;
pub const CAPABILITY_REVOKE_VOUCHER: &str = ACTION_REVOKE_VOUCHER;
pub const CAPABILITY_ADOPT: &str = ACTION_ADOPT;

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
        let (inventory, wans) = tokio::join!(self.list_network_inventory(), self.list_all_wans());
        let (devices, clients) = inventory?;
        let wans = wans?;
        self.remember_clients(clients.clone());

        // This deliberately fetches detail for every known device, whether or
        // not somebody has placed that target on a dashboard. That is simple
        // and honest for typical homelab device counts; the client's shared
        // semaphore caps the fan-out at ten rather than betting the console's
        // middleware can absorb an unbounded future installation.
        let device_readings = join_all(devices.iter().map(|device| async move {
            let Some(device_id) = api_device_id(device) else {
                return (None, None);
            };
            let statistics_path = format!(
                "sites/{}/devices/{device_id}/statistics/latest",
                self.site.id
            );
            let details_path = format!("sites/{}/devices/{device_id}", self.site.id);
            let (statistics, details) = tokio::join!(
                self.client.get::<DeviceStatistics>(&statistics_path),
                self.client.get::<DeviceDetails>(&details_path),
            );
            (statistics.ok(), details.ok())
        }));
        let device_readings = device_readings.await;

        let summary = map_site_summary(&devices, clients.len(), wans.len());
        let device_readings = devices
            .into_iter()
            .zip(device_readings)
            .map(|(mut device, (statistics, details))| {
                if let Some(details) = &details {
                    merge_device_detail_metadata(&mut device, details);
                }
                DeviceReading {
                    connected_client_count: connected_client_count(&clients, &device.id),
                    device,
                    statistics,
                    details,
                }
            })
            .collect::<Vec<_>>();
        self.remember_devices(
            device_readings
                .iter()
                .map(|reading| reading.device.clone())
                .collect(),
        );

        Ok(PollReadings {
            summary,
            devices: device_readings,
        })
    }

    async fn list_all_devices(&self) -> Result<Vec<DeviceOverview>, UniFiNetworkError> {
        let mut devices = self.list_all_devices_for_site(&self.site.id).await?;
        if let Ok(clients) = self.list_all_clients().await {
            self.recover_missing_client_uplinks(&self.site.id, &mut devices, &clients)
                .await;
            self.remember_clients(clients);
        }
        Ok(devices)
    }

    async fn list_all_devices_for_site(
        &self,
        site_id: &str,
    ) -> Result<Vec<DeviceOverview>, UniFiNetworkError> {
        let mut devices = self
            .client
            .fetch_all_pages::<DeviceOverview>(&format!("sites/{site_id}/devices"), PAGE_LIMIT)
            .await?;
        normalize_device_keys(&mut devices);
        if site_id == self.site.id {
            merge_known_device_metadata(&mut devices, &self.device_snapshot());
        }
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
        self.resolve_missing_client_uplinks(site_id, &mut clients)
            .await;
        Ok(clients)
    }

    /// Some console versions violate the published client-overview schema by
    /// omitting `uplinkDeviceId` from otherwise valid local client rows. The
    /// documented per-client detail endpoint carries the same field, so fill
    /// only those gaps before client-to-device reconciliation. Detail lookup
    /// remains best-effort because one stale client must not fail inventory.
    async fn resolve_missing_client_uplinks(&self, site_id: &str, clients: &mut [ClientOverview]) {
        let resolved = join_all(
            clients
                .iter()
                .filter(|client| client.needs_uplink_resolution())
                .map(|client| {
                    let client_id = client.id.clone();
                    async move {
                        let path = format!("sites/{site_id}/clients/{client_id}");
                        let details = self.client.get::<ClientOverview>(&path).await.ok()?;
                        (details.id == client_id).then_some((client_id, details.uplink_device_id))
                    }
                }),
        )
        .await;
        apply_resolved_client_uplinks(clients, resolved.into_iter().flatten());
    }

    async fn list_network_inventory(
        &self,
    ) -> Result<(Vec<DeviceOverview>, Vec<ClientOverview>), UniFiNetworkError> {
        self.list_network_inventory_for_site(&self.site.id).await
    }

    async fn list_network_inventory_for_site(
        &self,
        site_id: &str,
    ) -> Result<(Vec<DeviceOverview>, Vec<ClientOverview>), UniFiNetworkError> {
        let (devices, clients) = tokio::join!(
            self.list_all_devices_for_site(site_id),
            self.list_all_clients_for_site(site_id),
        );
        let mut devices = devices?;
        let clients = clients?;
        self.recover_missing_client_uplinks(site_id, &mut devices, &clients)
            .await;
        Ok((devices, clients))
    }

    /// Some real Network releases omit an adopted AP from the paginated
    /// devices collection even while connected clients name it as their
    /// uplink. Both the client uplink id and per-device detail route are part
    /// of the official Integration API, so reconcile those references rather
    /// than falling back to an undocumented legacy endpoint.
    ///
    /// Recovery is best-effort: one stale uplink reference must not turn an
    /// otherwise useful inventory into a connector-wide failure.
    async fn recover_missing_client_uplinks(
        &self,
        site_id: &str,
        devices: &mut Vec<DeviceOverview>,
        clients: &[ClientOverview],
    ) {
        let missing_ids = missing_uplink_device_ids(devices, clients);
        let recovered = join_all(missing_ids.into_iter().map(|device_id| async move {
            let path = format!("sites/{site_id}/devices/{device_id}");
            let details = self.client.get::<DeviceDetails>(&path).await.ok()?;
            (details.id == device_id)
                .then(|| device_overview_from_details(details))
                .flatten()
        }))
        .await;
        merge_recovered_devices(devices, recovered.into_iter().flatten());
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

    async fn list_all_wans(&self) -> Result<Vec<WanOverview>, UniFiNetworkError> {
        self.client
            .fetch_all_pages::<WanOverview>(&format!("sites/{}/wans", self.site.id), PAGE_LIMIT)
            .await
    }

    async fn list_all_pending_devices(
        &self,
    ) -> Result<Vec<PendingDeviceOverview>, UniFiNetworkError> {
        self.client
            .fetch_all_pages::<PendingDeviceOverview>("pending-devices", PAGE_LIMIT)
            .await
    }

    async fn list_all_acl_rules(&self) -> Result<Vec<AclRuleOverview>, UniFiNetworkError> {
        self.client
            .fetch_all_pages::<AclRuleOverview>(
                &format!("sites/{}/acl-rules", self.site.id),
                PAGE_LIMIT,
            )
            .await
    }

    async fn list_all_dns_policies(&self) -> Result<Vec<DnsPolicyOverview>, UniFiNetworkError> {
        self.client
            .fetch_all_pages::<DnsPolicyOverview>(
                &format!("sites/{}/dns/policies", self.site.id),
                PAGE_LIMIT,
            )
            .await
    }

    async fn list_all_firewall_zones(
        &self,
    ) -> Result<Vec<FirewallZoneOverview>, UniFiNetworkError> {
        self.client
            .fetch_all_pages::<FirewallZoneOverview>(
                &format!("sites/{}/firewall/zones", self.site.id),
                PAGE_LIMIT,
            )
            .await
    }

    async fn list_all_firewall_policies(
        &self,
    ) -> Result<Vec<FirewallPolicyOverview>, UniFiNetworkError> {
        self.client
            .fetch_all_pages::<FirewallPolicyOverview>(
                &format!("sites/{}/firewall/policies", self.site.id),
                PAGE_LIMIT,
            )
            .await
    }

    async fn list_all_networks(&self) -> Result<Vec<NetworkOverview>, UniFiNetworkError> {
        self.client
            .fetch_all_pages::<NetworkOverview>(
                &format!("sites/{}/networks", self.site.id),
                PAGE_LIMIT,
            )
            .await
    }

    async fn list_all_wifi_broadcasts(
        &self,
    ) -> Result<Vec<WifiBroadcastDetails>, UniFiNetworkError> {
        let overviews = self
            .client
            .fetch_all_pages::<WifiBroadcastOverview>(
                &format!("sites/{}/wifi/broadcasts", self.site.id),
                PAGE_LIMIT,
            )
            .await?;
        let details = join_all(overviews.into_iter().map(|overview| async move {
            self.client
                .get::<WifiBroadcastDetails>(&format!(
                    "sites/{}/wifi/broadcasts/{}",
                    self.site.id, overview.id
                ))
                .await
        }))
        .await;
        details.into_iter().collect()
    }

    async fn list_all_vpn_servers(&self) -> Result<Vec<VpnServerOverview>, UniFiNetworkError> {
        self.client
            .fetch_all_pages::<VpnServerOverview>(
                &format!("sites/{}/vpn/servers", self.site.id),
                PAGE_LIMIT,
            )
            .await
    }

    async fn list_all_site_to_site_tunnels(
        &self,
    ) -> Result<Vec<SiteToSiteTunnelOverview>, UniFiNetworkError> {
        self.client
            .fetch_all_pages::<SiteToSiteTunnelOverview>(
                &format!("sites/{}/vpn/site-to-site-tunnels", self.site.id),
                PAGE_LIMIT,
            )
            .await
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
                        set_optional_number(
                            &mut details,
                            &target_id,
                            DATA_POINT_LOAD_AVERAGE_1M,
                            statistics.load_average_1_min,
                        );
                        set_optional_number(
                            &mut details,
                            &target_id,
                            DATA_POINT_LOAD_AVERAGE_5M,
                            statistics.load_average_5_min,
                        );
                        set_optional_number(
                            &mut details,
                            &target_id,
                            DATA_POINT_LOAD_AVERAGE_15M,
                            statistics.load_average_15_min,
                        );
                        if let Some(last_heartbeat_at) = &statistics.last_heartbeat_at {
                            set_detail(
                                &mut details,
                                Some(&target_id),
                                DATA_POINT_LAST_HEARTBEAT_AT,
                                json!(last_heartbeat_at),
                            );
                        }
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
                    if device_has_capability(&reading.device, "accessPoint") {
                        if let Some(statistics) = &reading.statistics {
                            set_optional_number(
                                &mut details,
                                &target_id,
                                DATA_POINT_RADIO_TX_RETRY_PERCENT,
                                max_radio_tx_retry_percent(statistics),
                            );
                        }
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
        let devices = match (devices, clients.as_ref()) {
            (Ok(mut devices), Ok(clients)) => {
                self.recover_missing_client_uplinks(&site.id, &mut devices, clients)
                    .await;
                Ok(devices)
            }
            (devices, _) => devices,
        };
        connection_test_from_reads(devices, clients)
    }

    async fn actions(&self) -> Vec<ConnectorAction> {
        let Ok(devices) = self.list_all_devices().await else {
            return Vec::new();
        };
        self.remember_devices(devices.clone());
        devices
            .into_iter()
            .filter(|device| api_device_id(device).is_some())
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
            ACTION_UNAUTHORIZE_GUEST => {
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
                        guest_unauthorization_body(),
                    )
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok(
                    "Guest network access was unauthorized and the client was disconnected.",
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
            ACTION_ADOPT => {
                if target_id.is_some() {
                    return Err(ConnectorError::invalid_action(action_id));
                }
                let mac_address = required_resource_id(action_id, &params)?;
                let pending = self
                    .list_all_pending_devices()
                    .await
                    .map_err(connector_error)?;
                if !pending
                    .iter()
                    .any(|device| device.mac_address.eq_ignore_ascii_case(mac_address))
                {
                    return Err(invalid_param(
                        action_id,
                        "`resourceId` is not a device currently pending adoption",
                    ));
                }
                self.client
                    .post_json(
                        &format!("sites/{}/devices", self.site.id),
                        device_adoption_body(mac_address),
                    )
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok(
                    "Device adoption started. The device will become selectable after the connector's sub-targets are refreshed.",
                ))
            }
            ACTION_DELETE_ACL_RULE => {
                require_host_action(action_id, target_id)?;
                let resource_id = required_resource_id(action_id, &params)?;
                self.client
                    .delete(&format!("sites/{}/acl-rules/{resource_id}", self.site.id))
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok("Deleted the ACL rule."))
            }
            ACTION_DELETE_DNS_POLICY => {
                require_host_action(action_id, target_id)?;
                let resource_id = required_resource_id(action_id, &params)?;
                self.client
                    .delete(&format!(
                        "sites/{}/dns/policies/{resource_id}",
                        self.site.id
                    ))
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok("Deleted the DNS policy."))
            }
            ACTION_CREATE_A_RECORD | ACTION_CREATE_CNAME_RECORD | ACTION_CREATE_FORWARD_DOMAIN => {
                require_host_action(action_id, target_id)?;
                self.client
                    .post_json(
                        &format!("sites/{}/dns/policies", self.site.id),
                        dns_policy_creation_body(action_id, &params)?,
                    )
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok("Created the DNS policy."))
            }
            ACTION_DELETE_FIREWALL_ZONE => {
                require_host_action(action_id, target_id)?;
                let resource_id = required_resource_id(action_id, &params)?;
                self.client
                    .delete(&format!(
                        "sites/{}/firewall/zones/{resource_id}",
                        self.site.id
                    ))
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok("Deleted the firewall zone."))
            }
            ACTION_CREATE_FIREWALL_ZONE => {
                require_host_action(action_id, target_id)?;
                self.client
                    .post_json(
                        &format!("sites/{}/firewall/zones", self.site.id),
                        firewall_zone_creation_body(action_id, &params)?,
                    )
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok("Created the firewall zone."))
            }
            ACTION_DELETE_FIREWALL_POLICY => {
                require_host_action(action_id, target_id)?;
                let resource_id = required_resource_id(action_id, &params)?;
                self.client
                    .delete(&format!(
                        "sites/{}/firewall/policies/{resource_id}",
                        self.site.id
                    ))
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok("Deleted the firewall policy."))
            }
            ACTION_TOGGLE_FIREWALL_LOGGING => {
                require_host_action(action_id, target_id)?;
                let resource_id = required_resource_id(action_id, &params)?;
                let policy: FirewallPolicyOverview = self
                    .client
                    .get(&format!(
                        "sites/{}/firewall/policies/{resource_id}",
                        self.site.id
                    ))
                    .await
                    .map_err(connector_error)?;
                self.client
                    .patch_json(
                        &format!("sites/{}/firewall/policies/{resource_id}", self.site.id),
                        json!({ "loggingEnabled": !policy.logging_enabled }),
                    )
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok("Toggled firewall-policy logging."))
            }
            ACTION_DELETE_NETWORK => {
                require_host_action(action_id, target_id)?;
                let resource_id = required_resource_id(action_id, &params)?;
                self.client
                    .delete(&format!(
                        "sites/{}/networks/{resource_id}?force=false",
                        self.site.id
                    ))
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok("Deleted the network."))
            }
            ACTION_TOGGLE_WLAN_ENABLED => {
                require_host_action(action_id, target_id)?;
                let resource_id = required_resource_id(action_id, &params)?;
                let details: WifiBroadcastDetails = self
                    .client
                    .get(&format!(
                        "sites/{}/wifi/broadcasts/{resource_id}",
                        self.site.id
                    ))
                    .await
                    .map_err(connector_error)?;
                let body = wifi_broadcast_toggle_body(action_id, details)?;
                self.client
                    .put_json(
                        &format!("sites/{}/wifi/broadcasts/{resource_id}", self.site.id),
                        body,
                    )
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok("Toggled the WLAN broadcast state."))
            }
            _ => Err(ConnectorError::invalid_action(action_id)),
        }
    }

    fn resource_kinds(&self, target_id: Option<&str>) -> Vec<ResourceKindDescriptor> {
        match target_id {
            None => vec![
                clients_kind(),
                vouchers_kind(),
                wans_kind(),
                pending_devices_kind(),
                acl_rules_kind(),
                dns_policies_kind(),
                firewall_zones_kind(),
                firewall_policies_kind(),
                networks_kind(),
                wlan_broadcasts_kind(),
                vpn_servers_kind(),
                site_to_site_tunnels_kind(),
            ],
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
            (RESOURCE_KIND_WANS, None) => Ok(wan_resource_items(
                self.list_all_wans().await.map_err(connector_error)?,
            )),
            (RESOURCE_KIND_PENDING_DEVICES, None) => Ok(pending_device_resource_items(
                self.list_all_pending_devices()
                    .await
                    .map_err(connector_error)?,
            )),
            (RESOURCE_KIND_ACL_RULES, None) => Ok(acl_rule_resource_items(
                self.list_all_acl_rules().await.map_err(connector_error)?,
            )),
            (RESOURCE_KIND_DNS_POLICIES, None) => Ok(dns_policy_resource_items(
                self.list_all_dns_policies()
                    .await
                    .map_err(connector_error)?,
            )),
            (RESOURCE_KIND_FIREWALL_ZONES, None) => {
                let (zones, networks) =
                    tokio::join!(self.list_all_firewall_zones(), self.list_all_networks());
                Ok(firewall_zone_resource_items(
                    zones.map_err(connector_error)?,
                    &networks.map_err(connector_error)?,
                ))
            }
            (RESOURCE_KIND_FIREWALL_POLICIES, None) => {
                let (policies, zones) = tokio::join!(
                    self.list_all_firewall_policies(),
                    self.list_all_firewall_zones()
                );
                Ok(firewall_policy_resource_items(
                    policies.map_err(connector_error)?,
                    &zones.map_err(connector_error)?,
                ))
            }
            (RESOURCE_KIND_NETWORKS, None) => Ok(network_resource_items(
                self.list_all_networks().await.map_err(connector_error)?,
            )),
            (RESOURCE_KIND_WLAN_BROADCASTS, None) => Ok(wifi_broadcast_resource_items(
                self.list_all_wifi_broadcasts()
                    .await
                    .map_err(connector_error)?,
            )),
            (RESOURCE_KIND_VPN_SERVERS, None) => Ok(vpn_server_resource_items(
                self.list_all_vpn_servers().await.map_err(connector_error)?,
            )),
            (RESOURCE_KIND_SITE_TO_SITE_TUNNELS, None) => Ok(site_to_site_tunnel_resource_items(
                self.list_all_site_to_site_tunnels()
                    .await
                    .map_err(connector_error)?,
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
            DataPointDescriptor::new(DATA_POINT_WAN_COUNT, "WANs", DataPointValueType::Number),
        ];
        descriptors.extend(devices.iter().flat_map(device_data_points));
        descriptors
    }

    fn default_layout(&self) -> WidgetLayout {
        WidgetLayout::new(vec![
            WidgetBinding::display(DATA_POINT_DEVICE_COUNT, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_ONLINE_DEVICE_COUNT, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_CLIENT_COUNT, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_WAN_COUNT, DisplayWidgetType::StatTile),
        ])
    }

    fn default_layout_for(&self, target_id: Option<&str>) -> WidgetLayout {
        match target_id.and_then(device_key_from_target) {
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
                    WidgetBinding::display(
                        DATA_POINT_LAST_HEARTBEAT_AT,
                        DisplayWidgetType::StatTile,
                    ),
                    WidgetBinding::display(DATA_POINT_LOAD_AVERAGE_1M, DisplayWidgetType::StatTile),
                    WidgetBinding::display(DATA_POINT_LOAD_AVERAGE_5M, DisplayWidgetType::StatTile),
                    WidgetBinding::display(
                        DATA_POINT_LOAD_AVERAGE_15M,
                        DisplayWidgetType::StatTile,
                    ),
                ];
                bindings.extend([
                    WidgetBinding::display(
                        DATA_POINT_CPU_UTILIZATION,
                        DisplayWidgetType::ProgressBar,
                    ),
                    WidgetBinding::display(
                        DATA_POINT_MEMORY_UTILIZATION,
                        DisplayWidgetType::ProgressBar,
                    ),
                ]);
                let device = target_id
                    .and_then(device_key_from_target)
                    .and_then(|device_id| {
                        self.device_snapshot()
                            .into_iter()
                            .find(|device| device.id == device_id)
                    });
                if device
                    .as_ref()
                    .is_some_and(|device| device_has_capability(device, "accessPoint"))
                {
                    bindings.extend([
                        WidgetBinding::display(
                            DATA_POINT_CONNECTED_CLIENT_COUNT,
                            DisplayWidgetType::StatTile,
                        ),
                        WidgetBinding::display(DATA_POINT_RADIOS, DisplayWidgetType::StatTile),
                        WidgetBinding::display(
                            DATA_POINT_RADIO_TX_RETRY_PERCENT,
                            DisplayWidgetType::StatTile,
                        ),
                    ]);
                }
                if device.as_ref().is_some_and(|device| {
                    device_has_capability(device, "gateway")
                        || device_has_capability(device, "switching")
                }) {
                    bindings.extend([
                        WidgetBinding::display(
                            DATA_POINT_UPLINK_RX_RATE,
                            DisplayWidgetType::StatTile,
                        ),
                        WidgetBinding::display(
                            DATA_POINT_UPLINK_TX_RATE,
                            DisplayWidgetType::StatTile,
                        ),
                    ]);
                }
                if target_id.and_then(device_id_from_target).is_some() {
                    bindings.push(WidgetBinding::action(
                        ACTION_RESTART,
                        ActionWidgetType::Button,
                    ));
                }
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
    // The schema requires a UUID, but Network 10.x can return `null` for an
    // adopted online device. It is normalized to a MAC-backed local key after
    // deserialization so the device remains visible without pretending that
    // UUID-only endpoints accept its MAC address.
    #[serde(default, deserialize_with = "deserialize_nullable_string")]
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
    load_average_1_min: Option<f64>,
    #[serde(default)]
    load_average_5_min: Option<f64>,
    #[serde(default)]
    load_average_15_min: Option<f64>,
    #[serde(default)]
    last_heartbeat_at: Option<String>,
    #[serde(default)]
    interfaces: DeviceInterfaceStatistics,
    #[serde(default)]
    uplink: Option<UplinkStatistics>,
}

#[derive(Debug, Default, Deserialize)]
struct DeviceInterfaceStatistics {
    #[serde(default)]
    radios: Vec<RadioStatistics>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RadioStatistics {
    #[serde(default)]
    tx_retries_pct: Option<f64>,
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
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    mac_address: Option<String>,
    #[serde(default)]
    features: Map<String, Value>,
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
    #[serde(
        rename = "frequencyGHz",
        deserialize_with = "deserialize_string_or_number"
    )]
    frequency_ghz: String,
    #[serde(rename = "channelWidthMHz")]
    channel_width_mhz: u32,
    #[serde(default)]
    channel: Option<u32>,
}

fn deserialize_string_or_number<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        String(String),
        Number(serde_json::Number),
    }

    Ok(match StringOrNumber::deserialize(deserializer)? {
        StringOrNumber::String(value) => value,
        StringOrNumber::Number(value) => value.to_string(),
    })
}

fn deserialize_nullable_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
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
    #[serde(rename = "type")]
    client_type: String,
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

impl ClientOverview {
    fn needs_uplink_resolution(&self) -> bool {
        matches!(self.client_type.as_str(), "WIRED" | "WIRELESS")
            && self
                .uplink_device_id
                .as_deref()
                .is_none_or(|device_id| device_id.trim().is_empty())
    }
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

#[derive(Debug, Deserialize)]
struct WanOverview {
    id: String,
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PendingDeviceOverview {
    mac_address: String,
    model: String,
    state: String,
    #[serde(default)]
    firmware_version: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AclRuleOverview {
    id: String,
    name: String,
    #[serde(rename = "type")]
    rule_type: String,
    action: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DnsPolicyOverview {
    id: String,
    #[serde(rename = "type")]
    policy_type: String,
    #[serde(default)]
    domain: Option<String>,
    enabled: bool,
    #[serde(default)]
    ipv4_address: Option<String>,
    #[serde(default)]
    ipv6_address: Option<String>,
    #[serde(default)]
    target_domain: Option<String>,
    #[serde(default)]
    ip_address: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirewallZoneOverview {
    id: String,
    name: String,
    network_ids: Vec<String>,
    metadata: EntityMetadata,
}

#[derive(Debug, Deserialize)]
struct EntityMetadata {
    origin: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FirewallPolicyOverview {
    id: String,
    name: String,
    action: TypedValue,
    source: ZoneReference,
    destination: ZoneReference,
    enabled: bool,
    logging_enabled: bool,
}

#[derive(Debug, Deserialize)]
struct TypedValue {
    #[serde(rename = "type")]
    value_type: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ZoneReference {
    zone_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct NetworkOverview {
    id: String,
    name: String,
    vlan_id: u32,
    management: String,
    enabled: bool,
}

/// The list response omits fields needed by the resource table, so only its ID
/// is read before fetching the documented full configuration for each row.
#[derive(Debug, Deserialize)]
struct WifiBroadcastOverview {
    id: String,
}

/// Keeps the complete documented WLAN configuration intact for read-modify-
/// write. `id` is response-only; every other property is retained verbatim.
#[derive(Debug, Clone, Deserialize)]
struct WifiBroadcastDetails {
    id: String,
    #[serde(flatten)]
    config: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct VpnServerOverview {
    id: String,
    name: String,
    #[serde(rename = "type")]
    server_type: String,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct SiteToSiteTunnelOverview {
    id: String,
    name: String,
    #[serde(rename = "type")]
    tunnel_type: String,
}

#[derive(Debug, PartialEq, Eq)]
struct SiteSummary {
    device_count: usize,
    online_device_count: usize,
    client_count: usize,
    wan_count: usize,
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

fn map_site_summary(
    devices: &[DeviceOverview],
    client_count: usize,
    wan_count: usize,
) -> SiteSummary {
    let online_device_count = devices
        .iter()
        .filter(|device| device.state.as_deref() == Some("ONLINE"))
        .count();
    SiteSummary {
        device_count: devices.len(),
        online_device_count,
        client_count,
        wan_count,
    }
}

fn max_radio_tx_retry_percent(statistics: &DeviceStatistics) -> Option<f64> {
    statistics
        .interfaces
        .radios
        .iter()
        .filter_map(|radio| radio.tx_retries_pct)
        .filter(|value| value.is_finite())
        .reduce(f64::max)
}

fn connected_client_count(clients: &[ClientOverview], device_id: &str) -> usize {
    clients
        .iter()
        .filter(|client| client.uplink_device_id.as_deref() == Some(device_id))
        .count()
}

fn missing_uplink_device_ids(
    devices: &[DeviceOverview],
    clients: &[ClientOverview],
) -> Vec<String> {
    let known = devices
        .iter()
        .map(|device| device.id.as_str())
        .collect::<HashSet<_>>();
    let mut missing = HashSet::new();
    clients
        .iter()
        .filter_map(|client| client.uplink_device_id.as_deref())
        .map(str::trim)
        .filter(|device_id| !device_id.is_empty() && !known.contains(device_id))
        .filter(|device_id| missing.insert((*device_id).to_owned()))
        .map(str::to_owned)
        .collect()
}

fn apply_resolved_client_uplinks(
    clients: &mut [ClientOverview],
    resolved: impl IntoIterator<Item = (String, Option<String>)>,
) {
    let resolved = resolved
        .into_iter()
        .filter_map(|(client_id, uplink_device_id)| {
            uplink_device_id
                .filter(|device_id| !device_id.trim().is_empty())
                .map(|device_id| (client_id, device_id))
        })
        .collect::<HashMap<_, _>>();
    for client in clients {
        if let Some(device_id) = resolved.get(&client.id) {
            client.uplink_device_id = Some(device_id.clone());
        }
    }
}

fn normalize_device_keys(devices: &mut Vec<DeviceOverview>) {
    for device in devices.iter_mut() {
        device.id = match device.id.trim() {
            "" => device
                .mac_address
                .as_deref()
                .and_then(mac_device_key)
                .unwrap_or_default(),
            id => id.to_owned(),
        };
    }
    // A row with neither the documented UUID nor a usable MAC cannot have a
    // stable target identity. All other malformed rows remain visible.
    devices.retain(|device| !device.id.is_empty());
}

fn mac_device_key(mac_address: &str) -> Option<String> {
    let compact = mac_address
        .chars()
        .filter(char::is_ascii_hexdigit)
        .collect::<String>()
        .to_ascii_lowercase();
    (compact.len() == 12).then(|| format!("mac:{compact}"))
}

fn api_device_id(device: &DeviceOverview) -> Option<&str> {
    (!device.id.starts_with("mac:")).then_some(device.id.as_str())
}

fn merge_device_features(device: &mut DeviceOverview, features: impl IntoIterator<Item = String>) {
    for feature in features {
        if !device.features.contains(&feature) {
            device.features.push(feature);
        }
    }
}

fn merge_device_detail_metadata(device: &mut DeviceOverview, details: &DeviceDetails) {
    merge_device_features(device, details.features.keys().cloned());
    if device.name.as_deref().is_none_or(str::is_empty) {
        device.name.clone_from(&details.name);
    }
    if device.model.as_deref().is_none_or(str::is_empty) {
        device.model.clone_from(&details.model);
    }
    if device.mac_address.as_deref().is_none_or(str::is_empty) {
        device.mac_address.clone_from(&details.mac_address);
    }
}

fn merge_known_device_metadata(devices: &mut [DeviceOverview], known: &[DeviceOverview]) {
    for device in devices {
        if let Some(previous) = known.iter().find(|previous| previous.id == device.id) {
            merge_device_features(device, previous.features.iter().cloned());
        }
    }
}

fn merge_recovered_devices(
    devices: &mut Vec<DeviceOverview>,
    recovered: impl IntoIterator<Item = DeviceOverview>,
) {
    for recovered_device in recovered {
        if devices
            .iter()
            .any(|device| device.id == recovered_device.id)
        {
            continue;
        }
        if let Some(recovered_mac) = recovered_device
            .mac_address
            .as_deref()
            .and_then(mac_device_key)
        {
            devices.retain(|device| !(device.id.starts_with("mac:") && device.id == recovered_mac));
        }
        devices.push(recovered_device);
    }
}

fn device_overview_from_details(details: DeviceDetails) -> Option<DeviceOverview> {
    if details.id.trim().is_empty() {
        return None;
    }
    let mut features = ["gateway", "accessPoint", "switching"]
        .into_iter()
        .filter(|feature| details.features.contains_key(*feature))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if features.is_empty() && !details.interfaces.radios.is_empty() {
        features.push("accessPoint".to_owned());
    }
    Some(DeviceOverview {
        state: details.state,
        id: details.id,
        name: details.name,
        model: details.model,
        mac_address: details.mac_address,
        features,
    })
}

fn device_target_id(device_id: &str) -> String {
    format!("device:{device_id}")
}

fn device_key_from_target(target_id: &str) -> Option<&str> {
    target_id
        .strip_prefix("device:")
        .filter(|device_id| !device_id.is_empty())
}

fn device_id_from_target(target_id: &str) -> Option<&str> {
    device_key_from_target(target_id).filter(|device_id| !device_id.starts_with("mac:"))
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
    if device_has_capability(device, "gateway") {
        DeviceType::Gateway
    } else if device_has_capability(device, "accessPoint") {
        DeviceType::AccessPoint
    } else if device_has_capability(device, "switching") {
        DeviceType::Switch
    } else {
        DeviceType::NetworkDevice
    }
}

fn device_has_capability(device: &DeviceOverview, capability: &str) -> bool {
    device
        .features
        .iter()
        .any(|candidate| candidate == capability)
        || model_capability_override(device, capability)
}

/// Network's overview sometimes labels integrated consoles only as switching,
/// while its published detail schema cannot express `gateway` at all. Keep the
/// fallback deliberately limited to unambiguous integrated-gateway families;
/// ordinary AP/switch model names are never classified heuristically.
fn model_capability_override(device: &DeviceOverview, capability: &str) -> bool {
    let model = device
        .model
        .as_deref()
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    let is_cloud_gateway = model.starts_with("ucg");
    let is_dream_gateway = model.starts_with("udm") || model.starts_with("udr");
    let is_express = matches!(
        model.as_str(),
        "ux" | "ux7" | "unifiexpress" | "unifiexpress7"
    );
    match capability {
        "gateway" => is_cloud_gateway || is_dream_gateway || is_express,
        "switching" | "accessPoint" => is_express,
        _ => false,
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
        DataPointDescriptor::new(
            DATA_POINT_LOAD_AVERAGE_1M,
            "Load average (1 min)",
            DataPointValueType::Number,
        )
        .for_target(&target_id),
        DataPointDescriptor::new(
            DATA_POINT_LOAD_AVERAGE_5M,
            "Load average (5 min)",
            DataPointValueType::Number,
        )
        .for_target(&target_id),
        DataPointDescriptor::new(
            DATA_POINT_LOAD_AVERAGE_15M,
            "Load average (15 min)",
            DataPointValueType::Number,
        )
        .for_target(&target_id),
        DataPointDescriptor::new(
            DATA_POINT_LAST_HEARTBEAT_AT,
            "Last heartbeat",
            DataPointValueType::String,
        )
        .for_target(&target_id),
    ];
    if device_has_capability(device, "accessPoint") {
        points.extend([
            DataPointDescriptor::new(
                DATA_POINT_CONNECTED_CLIENT_COUNT,
                "Connected clients",
                DataPointValueType::Number,
            )
            .for_target(&target_id),
            DataPointDescriptor::new(DATA_POINT_RADIOS, "Radios", DataPointValueType::String)
                .for_target(&target_id),
            DataPointDescriptor::new(
                DATA_POINT_RADIO_TX_RETRY_PERCENT,
                "Worst radio TX retries",
                DataPointValueType::Number,
            )
            .with_unit("%")
            .for_target(&target_id),
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
    .with_row_actions(vec![
        authorize_guest_action(),
        resource_row_action(
            ACTION_UNAUTHORIZE_GUEST,
            "Unauthorize guest",
            "Revoke guest network access and disconnect this client.",
            true,
        ),
    ])
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

fn wans_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_WANS,
        "WANs",
        vec![
            ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
            ColumnDescriptor::new("id", "Identifier", ColumnValueType::Text),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
}

fn pending_devices_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_PENDING_DEVICES,
        "Pending Devices",
        vec![
            ColumnDescriptor::new("model", "Model", ColumnValueType::Text),
            ColumnDescriptor::new("macAddress", "MAC address", ColumnValueType::Text),
            ColumnDescriptor::new("state", "State", ColumnValueType::Text),
            ColumnDescriptor::new("firmwareVersion", "Firmware version", ColumnValueType::Text),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    .with_row_actions(vec![resource_row_action(
        ACTION_ADOPT,
        "Adopt",
        "Adopt this device into the configured site without bypassing its device limit.",
        false,
    )])
}

fn acl_rules_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_ACL_RULES,
        "ACL Rules",
        vec![
            ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
            ColumnDescriptor::new("type", "Type", ColumnValueType::Text),
            ColumnDescriptor::new("action", "Action", ColumnValueType::Text),
            ColumnDescriptor::new("enabled", "Enabled", ColumnValueType::Bool),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    .with_row_actions(vec![resource_row_action(
        ACTION_DELETE_ACL_RULE,
        "Delete",
        "Permanently delete this ACL rule.",
        false,
    )])
}

fn dns_policies_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_DNS_POLICIES,
        "DNS Policies",
        vec![
            ColumnDescriptor::new("type", "Type", ColumnValueType::Text),
            ColumnDescriptor::new("domain", "Domain", ColumnValueType::Text),
            ColumnDescriptor::new("target", "Target", ColumnValueType::Text),
            ColumnDescriptor::new("enabled", "Enabled", ColumnValueType::Bool),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    // AAAA/MX/SRV/TXT remain browse-and-delete only in this pass. Their
    // subtype-specific forms do not belong in one misleading mega-action.
    .with_row_actions(vec![resource_row_action(
        ACTION_DELETE_DNS_POLICY,
        "Delete",
        "Permanently delete this DNS policy.",
        false,
    )])
    .with_kind_actions(vec![
        create_a_record_action(),
        create_cname_record_action(),
        create_forward_domain_action(),
    ])
}

fn firewall_zones_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_FIREWALL_ZONES,
        "Firewall Zones",
        vec![
            ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
            ColumnDescriptor::new("networks", "Networks", ColumnValueType::Text),
            ColumnDescriptor::new("systemDerived", "System derived", ColumnValueType::Bool),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    .with_row_actions(vec![resource_row_action(
        ACTION_DELETE_FIREWALL_ZONE,
        "Delete",
        "Delete this firewall zone. UniFi will reject protected system zones.",
        false,
    )])
    .with_kind_actions(vec![create_firewall_zone_action()])
}

fn firewall_policies_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_FIREWALL_POLICIES,
        "Firewall Policies",
        vec![
            ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
            ColumnDescriptor::new("action", "Action", ColumnValueType::Text),
            ColumnDescriptor::new("sourceZone", "Source zone", ColumnValueType::Text),
            ColumnDescriptor::new("destinationZone", "Destination zone", ColumnValueType::Text),
            ColumnDescriptor::new("enabled", "Enabled", ColumnValueType::Bool),
            ColumnDescriptor::new("loggingEnabled", "Logging", ColumnValueType::Bool),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    .with_row_actions(vec![
        resource_row_action(
            ACTION_DELETE_FIREWALL_POLICY,
            "Delete",
            "Permanently delete this firewall policy.",
            false,
        ),
        resource_row_action(
            ACTION_TOGGLE_FIREWALL_LOGGING,
            "Toggle logging",
            "Flip logging for this policy without changing its other configuration.",
            false,
        ),
    ])
}

fn networks_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_NETWORKS,
        "Networks",
        vec![
            ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
            ColumnDescriptor::new("vlanId", "VLAN", ColumnValueType::Number),
            ColumnDescriptor::new("management", "Management", ColumnValueType::Text),
            ColumnDescriptor::new("enabled", "Enabled", ColumnValueType::Bool),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    .with_row_actions(vec![resource_row_action(
        ACTION_DELETE_NETWORK,
        "Delete network",
        "Deleting a network can disconnect every client and device using it. Loom does not force deletion when UniFi reports references.",
        true,
    )])
}

fn wlan_broadcasts_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_WLAN_BROADCASTS,
        "WLAN Broadcasts",
        vec![
            ColumnDescriptor::new("name", "SSID", ColumnValueType::Text),
            ColumnDescriptor::new("enabled", "Enabled", ColumnValueType::Bool),
            ColumnDescriptor::new("hidden", "Hidden", ColumnValueType::Bool),
            ColumnDescriptor::new("securityType", "Security", ColumnValueType::Text),
            ColumnDescriptor::new("frequencies", "Frequencies", ColumnValueType::Text),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    .with_row_actions(vec![resource_row_action(
        ACTION_TOGGLE_WLAN_ENABLED,
        "Toggle enabled",
        "Enable or disable this WLAN without changing its other configuration. Connected clients may be disconnected.",
        true,
    )])
}

fn vpn_servers_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_VPN_SERVERS,
        "VPN Servers",
        vec![
            ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
            ColumnDescriptor::new("type", "Type", ColumnValueType::Text),
            ColumnDescriptor::new("enabled", "Enabled", ColumnValueType::Bool),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
}

fn site_to_site_tunnels_kind() -> ResourceKindDescriptor {
    // Network 10.4.57 publishes no remote-peer or enabled field for this
    // collection and exposes no detail route. Keep the table truthful instead
    // of filling requested-looking columns with guesses or placeholders.
    ResourceKindDescriptor::new(
        RESOURCE_KIND_SITE_TO_SITE_TUNNELS,
        "Site-to-Site Tunnels",
        vec![
            ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
            ColumnDescriptor::new("type", "Type", ColumnValueType::Text),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
}

fn create_a_record_action() -> ConnectorAction {
    dns_creation_action(
        ACTION_CREATE_A_RECORD,
        "Create A record",
        json!({
            "domain": text_schema("Domain", "DNS name for the record."),
            "ipv4Address": text_schema("IPv4 address", "IPv4 address returned by this record."),
            "ttl": ttl_schema(86_400)
        }),
        &["domain", "ipv4Address", "ttl"],
    )
}

fn create_cname_record_action() -> ConnectorAction {
    dns_creation_action(
        ACTION_CREATE_CNAME_RECORD,
        "Create CNAME record",
        json!({
            "domain": text_schema("Domain", "Alias DNS name."),
            "targetDomain": text_schema("Target domain", "Canonical DNS name the alias resolves to."),
            "ttl": ttl_schema(604_800)
        }),
        &["domain", "targetDomain", "ttl"],
    )
}

fn create_forward_domain_action() -> ConnectorAction {
    dns_creation_action(
        ACTION_CREATE_FORWARD_DOMAIN,
        "Create forward domain",
        json!({
            "domain": text_schema("Domain", "DNS suffix to forward."),
            "forwardIp": text_schema("Forwarding server", "IPv4 or IPv6 address of the DNS server."),
        }),
        &["domain", "forwardIp"],
    )
}

fn dns_creation_action(
    id: &str,
    label: &str,
    properties: Value,
    required: &[&str],
) -> ConnectorAction {
    ConnectorAction {
        id: id.to_owned(),
        target_id: None,
        label: label.to_owned(),
        description: Some(format!("{label} using the official UniFi DNS policy API.")),
        params_schema: json!({
            "type": "object",
            "properties": properties,
            "required": required,
            "additionalProperties": false
        }),
        is_disruptive: false,
        snapshot_data_point_ids: Vec::new(),
    }
}

fn create_firewall_zone_action() -> ConnectorAction {
    ConnectorAction {
        id: ACTION_CREATE_FIREWALL_ZONE.to_owned(),
        target_id: None,
        label: "Create zone".to_owned(),
        description: Some("Create a firewall zone and attach network IDs.".to_owned()),
        params_schema: json!({
            "type": "object",
            "properties": {
                "name": text_schema("Name", "Firewall zone name."),
                // SchemaForm intentionally does not support arrays yet. Match
                // its established string control and parse a comma-separated
                // list at the connector boundary.
                "networkIds": {
                    "type": "string",
                    "title": "Network IDs",
                    "description": "Comma-separated network IDs; leave empty for an unattached zone."
                }
            },
            "required": ["name"],
            "additionalProperties": false
        }),
        is_disruptive: false,
        snapshot_data_point_ids: Vec::new(),
    }
}

fn text_schema(title: &str, description: &str) -> Value {
    json!({
        "type": "string",
        "title": title,
        "description": description,
        "minLength": 1
    })
}

fn ttl_schema(maximum: u64) -> Value {
    json!({
        "type": "integer",
        "title": "TTL (seconds)",
        "description": "DNS time to live in seconds.",
        "minimum": 0,
        "maximum": maximum
    })
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
            "Create one hotspot voucher with a duration and optional usage limits.".to_owned(),
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
            "required": ["name", "timeLimitMinutes"],
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

fn wan_resource_items(mut wans: Vec<WanOverview>) -> Vec<ResourceItem> {
    wans.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
    });
    wans.into_iter()
        .map(|wan| {
            ResourceItem::new(wan.id.clone())
                .with_field("name", wan.name)
                .with_field("id", wan.id)
        })
        .collect()
}

fn pending_device_resource_items(mut devices: Vec<PendingDeviceOverview>) -> Vec<ResourceItem> {
    devices.sort_by(|left, right| {
        left.model
            .to_ascii_lowercase()
            .cmp(&right.model.to_ascii_lowercase())
            .then_with(|| left.mac_address.cmp(&right.mac_address))
    });
    devices
        .into_iter()
        .map(|device| {
            ResourceItem::new(device.mac_address.clone())
                .with_field("model", device.model)
                .with_field("macAddress", device.mac_address)
                .with_field("state", device.state)
                .with_field(
                    "firmwareVersion",
                    device
                        .firmware_version
                        .map(Value::String)
                        .unwrap_or(Value::Null),
                )
        })
        .collect()
}

fn acl_rule_resource_items(mut rules: Vec<AclRuleOverview>) -> Vec<ResourceItem> {
    rules.sort_by_key(|rule| rule.name.to_lowercase());
    rules
        .into_iter()
        .map(|rule| {
            let rule_type = if rule.rule_type == "IPV4" {
                "IPv4".to_owned()
            } else {
                rule.rule_type
            };
            ResourceItem::new(rule.id)
                .with_field("name", rule.name)
                .with_field("type", rule_type)
                .with_field("action", rule.action)
                .with_field("enabled", rule.enabled)
        })
        .collect()
}

fn dns_policy_resource_items(mut policies: Vec<DnsPolicyOverview>) -> Vec<ResourceItem> {
    policies.sort_by(|left, right| {
        left.domain
            .as_deref()
            .unwrap_or_default()
            .to_lowercase()
            .cmp(&right.domain.as_deref().unwrap_or_default().to_lowercase())
    });
    policies
        .into_iter()
        .map(|policy| {
            let target = match policy.policy_type.as_str() {
                "A_RECORD" => policy.ipv4_address,
                "AAAA_RECORD" => policy.ipv6_address,
                "CNAME_RECORD" => policy.target_domain,
                "FORWARD_DOMAIN" => policy.ip_address,
                _ => None,
            }
            .unwrap_or_else(|| "—".to_owned());
            ResourceItem::new(policy.id)
                .with_field("type", policy.policy_type)
                .with_field("domain", policy.domain.unwrap_or_else(|| "—".to_owned()))
                .with_field("target", target)
                .with_field("enabled", policy.enabled)
        })
        .collect()
}

fn firewall_zone_resource_items(
    mut zones: Vec<FirewallZoneOverview>,
    networks: &[NetworkOverview],
) -> Vec<ResourceItem> {
    let network_names = networks
        .iter()
        .map(|network| (network.id.as_str(), network.name.as_str()))
        .collect::<HashMap<_, _>>();
    zones.sort_by_key(|zone| zone.name.to_lowercase());
    zones
        .into_iter()
        .map(|zone| {
            let attached = zone
                .network_ids
                .iter()
                .map(|id| network_names.get(id.as_str()).copied().unwrap_or(id))
                .collect::<Vec<_>>()
                .join(", ");
            ResourceItem::new(zone.id)
                .with_field("name", zone.name)
                .with_field("networks", attached)
                .with_field("systemDerived", zone.metadata.origin == "SYSTEM_DEFINED")
        })
        .collect()
}

fn firewall_policy_resource_items(
    mut policies: Vec<FirewallPolicyOverview>,
    zones: &[FirewallZoneOverview],
) -> Vec<ResourceItem> {
    let zone_names = zones
        .iter()
        .map(|zone| (zone.id.as_str(), zone.name.as_str()))
        .collect::<HashMap<_, _>>();
    policies.sort_by_key(|policy| policy.name.to_lowercase());
    policies
        .into_iter()
        .map(|policy| {
            let source = zone_names
                .get(policy.source.zone_id.as_str())
                .copied()
                .unwrap_or(&policy.source.zone_id);
            let destination = zone_names
                .get(policy.destination.zone_id.as_str())
                .copied()
                .unwrap_or(&policy.destination.zone_id);
            ResourceItem::new(policy.id)
                .with_field("name", policy.name)
                .with_field("action", policy.action.value_type)
                .with_field("sourceZone", source)
                .with_field("destinationZone", destination)
                .with_field("enabled", policy.enabled)
                .with_field("loggingEnabled", policy.logging_enabled)
        })
        .collect()
}

fn network_resource_items(mut networks: Vec<NetworkOverview>) -> Vec<ResourceItem> {
    networks.sort_by_key(|network| network.name.to_lowercase());
    networks
        .into_iter()
        .map(|network| {
            ResourceItem::new(network.id)
                .with_field("name", network.name)
                .with_field("vlanId", network.vlan_id)
                .with_field("management", network.management)
                .with_field("enabled", network.enabled)
        })
        .collect()
}

fn wifi_broadcast_resource_items(mut broadcasts: Vec<WifiBroadcastDetails>) -> Vec<ResourceItem> {
    broadcasts.sort_by(|left, right| {
        wifi_config_string(&left.config, "name")
            .to_lowercase()
            .cmp(&wifi_config_string(&right.config, "name").to_lowercase())
    });
    broadcasts
        .into_iter()
        .map(|broadcast| {
            let frequencies = broadcast
                .config
                .get("broadcastingFrequenciesGHz")
                .and_then(Value::as_array)
                .map(|values| {
                    values
                        .iter()
                        .filter_map(Value::as_f64)
                        .map(|frequency| format!("{frequency} GHz"))
                        .collect::<Vec<_>>()
                        .join(", ")
                })
                .filter(|value| !value.is_empty())
                .unwrap_or_else(|| "—".to_owned());
            let security_type = broadcast
                .config
                .get("securityConfiguration")
                .and_then(|value| value.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("—");
            ResourceItem::new(broadcast.id)
                .with_field("name", wifi_config_string(&broadcast.config, "name"))
                .with_field(
                    "enabled",
                    broadcast
                        .config
                        .get("enabled")
                        .cloned()
                        .unwrap_or(Value::Null),
                )
                .with_field(
                    "hidden",
                    broadcast
                        .config
                        .get("hideName")
                        .cloned()
                        .unwrap_or(Value::Null),
                )
                .with_field("securityType", security_type)
                .with_field("frequencies", frequencies)
        })
        .collect()
}

fn wifi_config_string(config: &Map<String, Value>, key: &str) -> String {
    config
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("—")
        .to_owned()
}

fn vpn_server_resource_items(mut servers: Vec<VpnServerOverview>) -> Vec<ResourceItem> {
    servers.sort_by_key(|server| server.name.to_lowercase());
    servers
        .into_iter()
        .map(|server| {
            ResourceItem::new(server.id)
                .with_field("name", server.name)
                .with_field("type", server.server_type)
                .with_field("enabled", server.enabled)
        })
        .collect()
}

fn site_to_site_tunnel_resource_items(
    mut tunnels: Vec<SiteToSiteTunnelOverview>,
) -> Vec<ResourceItem> {
    tunnels.sort_by_key(|tunnel| tunnel.name.to_lowercase());
    tunnels
        .into_iter()
        .map(|tunnel| {
            ResourceItem::new(tunnel.id)
                .with_field("name", tunnel.name)
                .with_field("type", tunnel.tunnel_type)
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

fn guest_unauthorization_body() -> Value {
    json!({"action": "UNAUTHORIZE_GUEST_ACCESS"})
}

fn dns_policy_creation_body(action_id: &str, params: &Value) -> Result<Value, ConnectorError> {
    let domain = required_string_param(action_id, params, "domain")?;
    match action_id {
        ACTION_CREATE_A_RECORD => Ok(json!({
            "type": "A_RECORD",
            "enabled": true,
            "domain": domain,
            "ipv4Address": required_string_param(action_id, params, "ipv4Address")?,
            "ttlSeconds": required_integer_in_range(action_id, params, "ttl", 0, 86_400)?,
        })),
        ACTION_CREATE_CNAME_RECORD => Ok(json!({
            "type": "CNAME_RECORD",
            "enabled": true,
            "domain": domain,
            "targetDomain": required_string_param(action_id, params, "targetDomain")?,
            "ttlSeconds": required_integer_in_range(action_id, params, "ttl", 0, 604_800)?,
        })),
        ACTION_CREATE_FORWARD_DOMAIN => Ok(json!({
            "type": "FORWARD_DOMAIN",
            "enabled": true,
            "domain": domain,
            "ipAddress": required_string_param(action_id, params, "forwardIp")?,
        })),
        _ => Err(ConnectorError::invalid_action(action_id)),
    }
}

fn firewall_zone_creation_body(action_id: &str, params: &Value) -> Result<Value, ConnectorError> {
    let name = required_string_param(action_id, params, "name")?;
    let raw_network_ids = params
        .get("networkIds")
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| invalid_param(action_id, "`networkIds` must be a string"))
        })
        .transpose()?
        .unwrap_or_default();
    let mut seen = HashSet::new();
    let network_ids = raw_network_ids
        .split(',')
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .filter(|id| seen.insert((*id).to_owned()))
        .collect::<Vec<_>>();
    Ok(json!({ "name": name, "networkIds": network_ids }))
}

fn wifi_broadcast_toggle_body(
    action_id: &str,
    mut details: WifiBroadcastDetails,
) -> Result<Value, ConnectorError> {
    let enabled = details
        .config
        .get("enabled")
        .and_then(Value::as_bool)
        .ok_or_else(|| {
            ConnectorError::Internal(
                "UniFi WLAN detail omitted the required boolean `enabled` field".to_owned(),
            )
        })?;
    details.config.insert("enabled".to_owned(), json!(!enabled));
    // `id` is represented separately by WifiBroadcastDetails. Metadata is
    // response-only; every mutable configuration property remains byte-for-
    // byte equivalent at the JSON-value level apart from `enabled`.
    details.config.remove("metadata");
    if details.config.is_empty() {
        return Err(invalid_param(action_id, "WLAN configuration was empty"));
    }
    Ok(Value::Object(details.config))
}

fn device_adoption_body(mac_address: &str) -> Value {
    json!({
        "macAddress": mac_address,
        "ignoreDeviceLimit": false
    })
}

fn voucher_creation_body(action_id: &str, params: &Value) -> Result<Value, ConnectorError> {
    let name = required_string_param(action_id, params, "name")?;
    let time_limit = required_integer_param(action_id, params, "timeLimitMinutes")?;
    let mut body = Map::from_iter([
        ("count".to_owned(), json!(1)),
        ("name".to_owned(), json!(name)),
        ("timeLimitMinutes".to_owned(), json!(time_limit)),
    ]);
    copy_optional_integer_params(
        action_id,
        params,
        &mut body,
        &[
            "authorizedGuestLimit",
            "dataUsageLimitMBytes",
            "rxRateLimitKbps",
            "txRateLimitKbps",
        ],
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

fn required_integer_in_range(
    action_id: &str,
    params: &Value,
    key: &str,
    minimum: u64,
    maximum: u64,
) -> Result<u64, ConnectorError> {
    params
        .get(key)
        .and_then(Value::as_u64)
        .filter(|value| (minimum..=maximum).contains(value))
        .ok_or_else(|| {
            invalid_param(
                action_id,
                format!("`{key}` must be an integer from {minimum} through {maximum}"),
            )
        })
}

fn require_host_action(action_id: &str, target_id: Option<&str>) -> Result<(), ConnectorError> {
    if target_id.is_some() {
        Err(ConnectorError::invalid_action(action_id))
    } else {
        Ok(())
    }
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
    let read_devices = match devices {
        Ok(devices) => available_capability(
            CAPABILITY_READ_DEVICES,
            "List devices",
            Some(format!(
                "The configured site returned {} adopted device{}.",
                devices.len(),
                if devices.len() == 1 { "" } else { "s" }
            )),
        ),
        Err(error) => CapabilityStatus {
            key: CAPABILITY_READ_DEVICES.to_owned(),
            label: "List devices".to_owned(),
            available: false,
            note: Some(format!("device listing failed: {error}")),
        },
    };
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
                CAPABILITY_UNAUTHORIZE_GUEST,
                "Unauthorize guest clients",
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
                authenticated_note.clone(),
            ),
            available_capability(
                CAPABILITY_ADOPT,
                "Adopt pending devices",
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
                "{} GHz · Ch {channel} · {} MHz · {}",
                radio.frequency_ghz,
                radio.channel_width_mhz,
                describe_wifi_standard(&radio.wlan_standard)
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn describe_wifi_standard(standard: &str) -> String {
    let generation = match standard {
        "802.11n" => Some("Wi-Fi 4"),
        "802.11ac" => Some("Wi-Fi 5"),
        "802.11ax" => Some("Wi-Fi 6"),
        "802.11be" => Some("Wi-Fi 7"),
        _ => None,
    };
    generation.map_or_else(|| standard.to_owned(), str::to_owned)
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
    set_detail(
        &mut details,
        None,
        DATA_POINT_WAN_COUNT,
        json!(summary.wan_count),
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
        let devices = capability(&result, CAPABILITY_READ_DEVICES);
        assert!(devices.available);
        assert!(devices
            .note
            .as_deref()
            .is_some_and(|note| note.contains("1 adopted device.")));
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
            CAPABILITY_UNAUTHORIZE_GUEST,
            CAPABILITY_CREATE_VOUCHER,
            CAPABILITY_REVOKE_VOUCHER,
            CAPABILITY_ADOPT,
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
            map_site_summary(&devices.data, clients.total_count, 2),
            SiteSummary {
                device_count: 3,
                online_device_count: 2,
                client_count: 14,
                wan_count: 2,
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
            map_site_summary(&devices.data, 0, 0),
            SiteSummary {
                device_count: 2,
                online_device_count: 1,
                client_count: 0,
                wan_count: 0,
            }
        );
    }

    #[test]
    fn a_null_device_uuid_remains_visible_under_a_mac_backed_read_only_target() {
        let mut page: Page<DeviceOverview> = serde_json::from_value(json!({
            "totalCount": 1,
            "data": [{
                "id": null,
                "name": "Upstairs AP",
                "model": "U7 Lite",
                "state": "ONLINE",
                "macAddress": "00:11:22:33:44:55",
                "features": ["accessPoint"]
            }]
        }))
        .expect("a real console's schema-violating null device id");

        normalize_device_keys(&mut page.data);

        assert_eq!(page.data.len(), 1);
        assert_eq!(page.data[0].id, "mac:001122334455");
        assert_eq!(device_label(&page.data[0]), "Upstairs AP");
        assert_eq!(
            device_sub_targets(&page.data)[0].id,
            "device:mac:001122334455"
        );
        assert!(api_device_id(&page.data[0]).is_none());
        assert!(device_id_from_target("device:mac:001122334455").is_none());
        assert_eq!(
            map_site_summary(&page.data, 0, 0),
            SiteSummary {
                device_count: 1,
                online_device_count: 1,
                client_count: 0,
                wan_count: 0,
            }
        );
    }

    #[test]
    fn a_later_uuid_recovery_replaces_the_same_mac_backed_device() {
        let mut devices = vec![DeviceOverview {
            id: "mac:001122334455".to_owned(),
            name: Some("Upstairs AP".to_owned()),
            model: Some("U7 Lite".to_owned()),
            state: Some("ONLINE".to_owned()),
            mac_address: Some("00:11:22:33:44:55".to_owned()),
            features: vec!["accessPoint".to_owned()],
        }];
        let recovered = DeviceOverview {
            id: "11111111-1111-1111-1111-111111111111".to_owned(),
            name: Some("Upstairs AP".to_owned()),
            model: Some("U7 Lite".to_owned()),
            state: Some("ONLINE".to_owned()),
            mac_address: Some("00:11:22:33:44:55".to_owned()),
            features: vec!["accessPoint".to_owned()],
        };

        merge_recovered_devices(&mut devices, [recovered]);

        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id, "11111111-1111-1111-1111-111111111111");
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
        assert_eq!(connector.data_points().len(), 4);
        assert!(connector.supports_sub_targets());
        assert_eq!(connector.resource_kinds(None).len(), 12);
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
    fn device_type_uses_features_with_narrow_integrated_gateway_fallbacks() {
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
        assert_eq!(
            device_type(&device("UCG Max", &["switching"])),
            DeviceType::Gateway
        );
        let express = device("UniFi Express 7", &[]);
        assert_eq!(device_type(&express), DeviceType::Gateway);
        assert!(device_has_capability(&express, "gateway"));
        assert!(device_has_capability(&express, "switching"));
        assert!(device_has_capability(&express, "accessPoint"));
        assert_eq!(device_type(&device("U7", &[])), DeviceType::NetworkDevice);
    }

    #[test]
    fn combined_appliances_keep_every_capability_with_one_gateway_identity() {
        let mut device = DeviceOverview {
            id: "device-one".to_owned(),
            name: Some("Combined appliance".to_owned()),
            model: Some("UX".to_owned()),
            state: Some("ONLINE".to_owned()),
            mac_address: Some("00:11:22:33:44:55".to_owned()),
            features: vec!["switching".to_owned()],
        };
        let details: DeviceDetails = serde_json::from_value(json!({
            "id": "device-one",
            "name": "Combined appliance",
            "model": "UX",
            "state": "ONLINE",
            "macAddress": "00:11:22:33:44:55",
            "features": {
                "gateway": {},
                "switching": {},
                "accessPoint": {}
            },
            "interfaces": {"ports": [], "radios": []}
        }))
        .expect("combined device details");

        merge_device_detail_metadata(&mut device, &details);

        assert!(device_has_capability(&device, "gateway"));
        assert!(device_has_capability(&device, "switching"));
        assert!(device_has_capability(&device, "accessPoint"));
        assert_eq!(device_type(&device), DeviceType::Gateway);
        assert_eq!(device_icon(&device), "lucide:router");
        assert!(device_data_points(&device)
            .iter()
            .any(|point| point.id == DATA_POINT_RADIOS));

        let connector = UniFiNetworkConnector::from_config_value_for_connection_test(json!({
            "host": "https://console.example.com",
            "apiKey": "not-a-real-key"
        }))
        .expect("local-only connector");
        connector.remember_devices(vec![device]);
        let layout = connector.default_layout_for(Some("device:device-one"));
        let display_ids = layout
            .bindings
            .iter()
            .filter_map(|binding| match binding {
                WidgetBinding::Display { data_point_id, .. } => Some(data_point_id.as_str()),
                WidgetBinding::Action { .. } | WidgetBinding::ResourceKindDisplay { .. } => None,
            })
            .collect::<HashSet<_>>();
        for expected in [
            DATA_POINT_RADIOS,
            DATA_POINT_RADIO_TX_RETRY_PERCENT,
            DATA_POINT_UPLINK_RX_RATE,
            DATA_POINT_UPLINK_TX_RATE,
        ] {
            assert!(display_ids.contains(expected), "missing {expected}");
        }
        assert!(layout.bindings.iter().any(|binding| matches!(
            binding,
            WidgetBinding::Action { action_id, .. } if action_id == ACTION_RESTART
        )));
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
        assert_eq!(descriptors.len(), 14);
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
            "loadAverage1Min": 0.25,
            "loadAverage5Min": 0.5,
            "loadAverage15Min": 0.75,
            "lastHeartbeatAt": "2026-09-04T15:00:00Z",
            "uplink": {"rxRateBps": 1000, "txRateBps": 2000},
            "interfaces": {"radios": [
                {"frequencyGHz": 2.4, "txRetriesPct": 1.2},
                {"frequencyGHz": 5, "txRetriesPct": 3.4}
            ]}
        }))
        .expect("official device statistics");
        assert_eq!(statistics.cpu_utilization_pct, Some(12.5));
        assert_eq!(statistics.memory_utilization_pct, Some(48.25));
        assert_eq!(statistics.load_average_1_min, Some(0.25));
        assert_eq!(statistics.load_average_5_min, Some(0.5));
        assert_eq!(statistics.load_average_15_min, Some(0.75));
        assert_eq!(
            statistics.last_heartbeat_at.as_deref(),
            Some("2026-09-04T15:00:00Z")
        );
        assert_eq!(max_radio_tx_retry_percent(&statistics), Some(3.4));
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
                        "frequencyGHz": 2.4,
                        "channelWidthMHz": 20
                    }
                ]
            }
        }))
        .expect("official device detail");
        assert_eq!(
            format_radios(&details.interfaces.radios),
            "6 GHz · Ch 37 · 160 MHz · Wi-Fi 7\n2.4 GHz · Ch auto · 20 MHz · Wi-Fi 6"
        );
    }

    #[test]
    fn legacy_radio_standards_remain_explicit_when_they_have_no_wifi_generation_name() {
        assert_eq!(describe_wifi_standard("802.11a"), "802.11a");
    }

    #[test]
    fn missing_client_uplinks_identify_unlisted_devices_once() {
        let devices = vec![DeviceOverview {
            id: "known-device".to_owned(),
            name: None,
            model: None,
            state: None,
            mac_address: None,
            features: Vec::new(),
        }];
        let client = |id: &str, uplink: Option<&str>| ClientOverview {
            id: id.to_owned(),
            client_type: "WIRELESS".to_owned(),
            name: String::new(),
            mac_address: None,
            ip_address: None,
            uplink_device_id: uplink.map(str::to_owned),
            access: ClientAccessOverview {
                access_type: "WIRELESS".to_owned(),
                authorized: None,
            },
        };
        let clients = vec![
            client("one", Some("known-device")),
            client("two", Some("missing-device")),
            client("three", Some("missing-device")),
            client("four", Some("")),
            client("five", None),
        ];

        assert_eq!(
            missing_uplink_device_ids(&devices, &clients),
            ["missing-device"]
        );
    }

    #[test]
    fn missing_list_uplink_is_filled_from_official_client_detail() {
        let mut clients = vec![ClientOverview {
            id: "wireless-client".to_owned(),
            client_type: "WIRELESS".to_owned(),
            name: "Phone".to_owned(),
            mac_address: None,
            ip_address: None,
            uplink_device_id: None,
            access: ClientAccessOverview {
                access_type: "DEFAULT".to_owned(),
                authorized: None,
            },
        }];

        apply_resolved_client_uplinks(
            &mut clients,
            [("wireless-client".to_owned(), Some("missing-ap".to_owned()))],
        );

        assert_eq!(clients[0].uplink_device_id.as_deref(), Some("missing-ap"));
    }

    #[test]
    fn remote_clients_do_not_trigger_local_uplink_detail_recovery() {
        let client: ClientOverview = serde_json::from_value(json!({
            "type": "VPN",
            "id": "remote-client",
            "name": "Remote user",
            "access": {"type": "DEFAULT"}
        }))
        .expect("official remote client overview");

        assert!(!client.needs_uplink_resolution());
    }

    #[test]
    fn official_device_detail_can_recover_an_omitted_access_point() {
        let details: DeviceDetails = serde_json::from_value(json!({
            "id": "missing-ap",
            "name": "Upstairs",
            "model": "U7 Pro",
            "state": "ONLINE",
            "macAddress": "00:11:22:33:44:55",
            "features": {"accessPoint": {}},
            "interfaces": {"ports": [], "radios": []}
        }))
        .expect("official device detail");

        let recovered = device_overview_from_details(details).expect("recoverable device");
        assert_eq!(recovered.id, "missing-ap");
        assert_eq!(recovered.state.as_deref(), Some("ONLINE"));
        assert_eq!(recovered.features, ["accessPoint"]);
        assert_eq!(device_type(&recovered), DeviceType::AccessPoint);
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
    fn official_wan_and_pending_device_shapes_map_without_invented_fields() {
        let wans: Page<WanOverview> = serde_json::from_value(json!({
            "offset": 0,
            "limit": 200,
            "count": 2,
            "totalCount": 2,
            "data": [
                {"id": "wan-two", "name": "Backup"},
                {"id": "wan-one", "name": "Primary"}
            ]
        }))
        .expect("official WAN page");
        let wan_rows = wan_resource_items(wans.data);
        assert_eq!(wan_rows[0].id, "wan-two");
        assert_eq!(wan_rows[0].fields.get("name"), Some(&json!("Backup")));
        assert_eq!(wan_rows[1].fields.get("id"), Some(&json!("wan-one")));

        let pending: Page<PendingDeviceOverview> = serde_json::from_value(json!({
            "offset": 0,
            "limit": 200,
            "count": 2,
            "totalCount": 2,
            "data": [
                {
                    "macAddress": "00:11:22:33:44:66",
                    "model": "USW",
                    "state": "PENDING_ADOPTION",
                    "firmwareVersion": "7.1.0"
                },
                {
                    "macAddress": "00:11:22:33:44:55",
                    "model": "U7",
                    "state": "PENDING_ADOPTION"
                }
            ]
        }))
        .expect("official pending-device page");
        let pending_rows = pending_device_resource_items(pending.data);
        assert_eq!(pending_rows[0].id, "00:11:22:33:44:55");
        assert_eq!(pending_rows[0].fields.get("model"), Some(&json!("U7")));
        assert_eq!(
            pending_rows[0].fields.get("firmwareVersion"),
            Some(&Value::Null)
        );
        assert_eq!(
            pending_rows[1].fields.get("firmwareVersion"),
            Some(&json!("7.1.0"))
        );
    }

    #[test]
    fn resource_kinds_are_scoped_and_publish_only_confirmed_actions() {
        let host = [
            clients_kind(),
            vouchers_kind(),
            wans_kind(),
            pending_devices_kind(),
            acl_rules_kind(),
            dns_policies_kind(),
            firewall_zones_kind(),
            firewall_policies_kind(),
            networks_kind(),
            wlan_broadcasts_kind(),
            vpn_servers_kind(),
            site_to_site_tunnels_kind(),
        ];
        assert!(host
            .iter()
            .all(|kind| kind.applicable_target == ApplicableTarget::HostOnly));
        assert_eq!(
            host[0]
                .row_actions
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            [ACTION_AUTHORIZE_GUEST, ACTION_UNAUTHORIZE_GUEST]
        );
        assert_eq!(host[1].row_actions[0].id, ACTION_REVOKE_VOUCHER);
        assert_eq!(host[1].kind_actions[0].id, ACTION_CREATE_VOUCHER);
        assert!(host[2].row_actions.is_empty());
        assert!(host[2].kind_actions.is_empty());
        assert_eq!(host[3].row_actions[0].id, ACTION_ADOPT);
        assert_eq!(host[4].row_actions[0].id, ACTION_DELETE_ACL_RULE);
        assert_eq!(host[5].kind_actions.len(), 3);
        assert_eq!(host[6].kind_actions[0].id, ACTION_CREATE_FIREWALL_ZONE);
        assert_eq!(host[7].row_actions.len(), 2);
        assert!(host[8].row_actions[0].is_disruptive);
        assert!(host[9].row_actions[0].is_disruptive);
        assert!(host[10].row_actions.is_empty());
        assert!(host[10].kind_actions.is_empty());
        assert!(host[11].row_actions.is_empty());
        assert!(host[11].kind_actions.is_empty());

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
            guest_unauthorization_body(),
            json!({"action": "UNAUTHORIZE_GUEST_ACCESS"})
        );
        assert_eq!(
            device_adoption_body("00:11:22:33:44:55"),
            json!({
                "macAddress": "00:11:22:33:44:55",
                "ignoreDeviceLimit": false
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
        assert_eq!(
            voucher_creation_body(
                ACTION_CREATE_VOUCHER,
                &json!({"name": "Visitor", "timeLimitMinutes": 60})
            )
            .expect("guest limit is optional in v10.4.57"),
            json!({"count": 1, "name": "Visitor", "timeLimitMinutes": 60})
        );
        assert_eq!(
            dns_policy_creation_body(
                ACTION_CREATE_A_RECORD,
                &json!({"domain": "host.example.com", "ipv4Address": "192.0.2.10", "ttl": 300})
            )
            .expect("A record"),
            json!({
                "type": "A_RECORD",
                "enabled": true,
                "domain": "host.example.com",
                "ipv4Address": "192.0.2.10",
                "ttlSeconds": 300
            })
        );
        assert_eq!(
            dns_policy_creation_body(
                ACTION_CREATE_CNAME_RECORD,
                &json!({"domain": "alias.example.com", "targetDomain": "host.example.com", "ttl": 600})
            )
            .expect("CNAME record"),
            json!({
                "type": "CNAME_RECORD",
                "enabled": true,
                "domain": "alias.example.com",
                "targetDomain": "host.example.com",
                "ttlSeconds": 600
            })
        );
        assert_eq!(
            dns_policy_creation_body(
                ACTION_CREATE_FORWARD_DOMAIN,
                &json!({"domain": "internal.example", "forwardIp": "192.0.2.53"})
            )
            .expect("forward domain"),
            json!({
                "type": "FORWARD_DOMAIN",
                "enabled": true,
                "domain": "internal.example",
                "ipAddress": "192.0.2.53"
            })
        );
        assert_eq!(
            firewall_zone_creation_body(
                ACTION_CREATE_FIREWALL_ZONE,
                &json!({"name": "Devices", "networkIds": "network-a, network-b, network-a"})
            )
            .expect("zone"),
            json!({"name": "Devices", "networkIds": ["network-a", "network-b"]})
        );
    }

    #[test]
    fn tier_two_acl_and_dns_shapes_map_to_browse_rows() {
        let acl: Page<AclRuleOverview> = serde_json::from_value(json!({
            "totalCount": 2,
            "data": [
                {"id":"acl-2","name":"Block cameras","type":"MAC","action":"BLOCK","enabled":false},
                {"id":"acl-1","name":"Allow DNS","type":"IPV4","action":"ALLOW","enabled":true}
            ]
        }))
        .expect("ACL page");
        let acl_rows = acl_rule_resource_items(acl.data);
        assert_eq!(acl_rows[0].fields.get("type"), Some(&json!("IPv4")));
        assert_eq!(acl_rows[1].fields.get("enabled"), Some(&json!(false)));

        let dns: Page<DnsPolicyOverview> = serde_json::from_value(json!({
            "totalCount": 4,
            "data": [
                {"id":"dns-a","type":"A_RECORD","domain":"a.example","ipv4Address":"192.0.2.10","ttlSeconds":300,"enabled":true,"metadata":{}},
                {"id":"dns-aaaa","type":"AAAA_RECORD","domain":"aaaa.example","ipv6Address":"2001:db8::10","ttlSeconds":300,"enabled":true,"metadata":{}},
                {"id":"dns-cname","type":"CNAME_RECORD","domain":"cname.example","targetDomain":"a.example","ttlSeconds":300,"enabled":true,"metadata":{}},
                {"id":"dns-txt","type":"TXT_RECORD","domain":"txt.example","text":"value","enabled":false,"metadata":{}}
            ]
        }))
        .expect("DNS page");
        let dns_rows = dns_policy_resource_items(dns.data);
        assert_eq!(dns_rows[0].fields.get("target"), Some(&json!("192.0.2.10")));
        assert_eq!(
            dns_rows[1].fields.get("target"),
            Some(&json!("2001:db8::10"))
        );
        assert_eq!(dns_rows[2].fields.get("target"), Some(&json!("a.example")));
        assert_eq!(dns_rows[3].fields.get("target"), Some(&json!("—")));
    }

    #[test]
    fn tier_two_zone_policy_and_network_shapes_resolve_names() {
        let networks: Page<NetworkOverview> = serde_json::from_value(json!({
            "totalCount": 2,
            "data": [
                {"id":"network-a","name":"Trusted","vlanId":10,"management":"GATEWAY","enabled":true,"default":false,"metadata":{}},
                {"id":"network-b","name":"IoT","vlanId":20,"management":"SWITCH","enabled":false,"default":false,"metadata":{}}
            ]
        }))
        .expect("network page");
        let zones: Page<FirewallZoneOverview> = serde_json::from_value(json!({
            "totalCount": 2,
            "data": [
                {"id":"zone-a","name":"Internal","networkIds":["network-a", "unknown-network"],"metadata":{"origin":"USER_DEFINED"}},
                {"id":"zone-b","name":"External","networkIds":[],"metadata":{"origin":"SYSTEM_DEFINED"}}
            ]
        }))
        .expect("zone page");
        let zone_rows = firewall_zone_resource_items(zones.data, &networks.data);
        assert_eq!(zone_rows[0].fields.get("systemDerived"), Some(&json!(true)));
        assert_eq!(
            zone_rows[1].fields.get("networks"),
            Some(&json!("Trusted, unknown-network"))
        );

        let policies: Page<FirewallPolicyOverview> = serde_json::from_value(json!({
            "totalCount": 1,
            "data": [{
                "id":"policy-a","name":"Allow outbound","action":{"type":"ALLOW"},
                "source":{"zoneId":"zone-a"},"destination":{"zoneId":"zone-b"},
                "enabled":true,"loggingEnabled":false,"index":0,"ipProtocolScope":{},"metadata":{}
            }]
        }))
        .expect("policy page");
        let zones_for_policy: Page<FirewallZoneOverview> = serde_json::from_value(json!({
            "totalCount": 2,
            "data": [
                {"id":"zone-a","name":"Internal","networkIds":[],"metadata":{"origin":"USER_DEFINED"}},
                {"id":"zone-b","name":"External","networkIds":[],"metadata":{"origin":"SYSTEM_DEFINED"}}
            ]
        }))
        .expect("zone page");
        let policy_rows = firewall_policy_resource_items(policies.data, &zones_for_policy.data);
        assert_eq!(policy_rows[0].fields.get("action"), Some(&json!("ALLOW")));
        assert_eq!(
            policy_rows[0].fields.get("sourceZone"),
            Some(&json!("Internal"))
        );
        assert_eq!(
            policy_rows[0].fields.get("destinationZone"),
            Some(&json!("External"))
        );

        let network_rows = network_resource_items(networks.data);
        assert_eq!(network_rows[0].fields.get("vlanId"), Some(&json!(20)));
        assert_eq!(
            network_rows[1].fields.get("management"),
            Some(&json!("GATEWAY"))
        );
    }

    #[test]
    fn wlan_toggle_preserves_every_mutable_field_and_rows_show_detail_only_values() {
        let details: WifiBroadcastDetails = serde_json::from_value(json!({
            "id":"wifi-a","metadata":{"origin":"USER_DEFINED"},"type":"STANDARD",
            "name":"Guest WiFi","enabled":true,"hideName":true,
            "securityConfiguration":{"type":"WPA2_PERSONAL","passphrase":"not-a-real-secret"},
            "broadcastingFrequenciesGHz":[2.4,5],"clientIsolationEnabled":true,
            "uapsdEnabled":false
        }))
        .expect("WLAN details");
        let body = wifi_broadcast_toggle_body(ACTION_TOGGLE_WLAN_ENABLED, details.clone())
            .expect("toggle body");
        assert_eq!(body.get("enabled"), Some(&json!(false)));
        assert_eq!(body.get("name"), Some(&json!("Guest WiFi")));
        assert_eq!(body.get("clientIsolationEnabled"), Some(&json!(true)));
        assert_eq!(body.get("uapsdEnabled"), Some(&json!(false)));
        assert!(body.get("id").is_none());
        assert!(body.get("metadata").is_none());

        let rows = wifi_broadcast_resource_items(vec![details]);
        assert_eq!(rows[0].fields.get("hidden"), Some(&json!(true)));
        assert_eq!(
            rows[0].fields.get("securityType"),
            Some(&json!("WPA2_PERSONAL"))
        );
        assert_eq!(
            rows[0].fields.get("frequencies"),
            Some(&json!("2.4 GHz, 5 GHz"))
        );
    }

    #[test]
    fn vpn_overview_shapes_map_only_fields_the_current_api_actually_exposes() {
        let servers: Page<VpnServerOverview> = serde_json::from_value(json!({
            "offset": 0,
            "limit": 25,
            "count": 2,
            "totalCount": 2,
            "data": [
                {"id":"vpn-b","name":"Remote staff","type":"WIREGUARD","enabled":true,"metadata":{"origin":"USER_DEFINED"}},
                {"id":"vpn-a","name":"Legacy access","type":"L2TP","enabled":false,"metadata":{"origin":"USER_DEFINED"}}
            ]
        }))
        .expect("VPN server page");
        let server_rows = vpn_server_resource_items(servers.data);
        assert_eq!(server_rows[0].id, "vpn-a");
        assert_eq!(server_rows[0].fields.get("type"), Some(&json!("L2TP")));
        assert_eq!(server_rows[1].fields.get("enabled"), Some(&json!(true)));

        let tunnels: Page<SiteToSiteTunnelOverview> = serde_json::from_value(json!({
            "offset": 0,
            "limit": 25,
            "count": 2,
            "totalCount": 2,
            "data": [
                {"id":"tunnel-b","name":"Warehouse","type":"IPSEC","metadata":{"origin":"USER_DEFINED"}},
                {"id":"tunnel-a","name":"Branch","type":"WIREGUARD","metadata":{"origin":"USER_DEFINED"}}
            ]
        }))
        .expect("site-to-site tunnel page");
        let tunnel_rows = site_to_site_tunnel_resource_items(tunnels.data);
        assert_eq!(tunnel_rows[0].id, "tunnel-a");
        assert_eq!(tunnel_rows[0].fields.get("type"), Some(&json!("WIREGUARD")));
        assert!(!tunnel_rows[0].fields.contains_key("remotePeer"));
        assert!(!tunnel_rows[0].fields.contains_key("enabled"));
    }
}
