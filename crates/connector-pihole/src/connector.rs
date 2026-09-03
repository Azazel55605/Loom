use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use loom_core::connector::{
    details::set_detail, ActionResult, ActionWidgetType, ApplicableTarget, CapabilityStatus,
    ChartType, ColumnDescriptor, ColumnValueType, ConnectionTestResult, ConnectorAction,
    ConnectorError, ConnectorMetadata, ConnectorStatus, DataPointDescriptor, DataPointValueType,
    DisplayField, DisplayWidgetType, HealthState, NetworkTarget, ResourceItem,
    ResourceKindDescriptor, SetupGuide, SetupGuideVariant, WidgetBinding, WidgetLayout,
};
use serde::Deserialize;
use serde_json::{json, Map, Value};

use crate::{config::PiHoleConnectorConfig, PiHoleClient, PiHoleError};

pub const TYPE_ID: &str = "pihole";
pub const DISPLAY_NAME: &str = "Pi-hole";
pub const ICON: &str = "brand:pihole";

pub const DATA_POINT_QUERIES_TODAY: &str = "queriesToday";
pub const DATA_POINT_QUERIES_BLOCKED_TODAY: &str = "queriesBlockedToday";
pub const DATA_POINT_BLOCK_PERCENTAGE: &str = "blockPercentage";
pub const DATA_POINT_DOMAINS_ON_BLOCKLIST: &str = "domainsOnBlocklist";
pub const DATA_POINT_UNIQUE_CLIENTS: &str = "uniqueClients";
pub const DATA_POINT_BLOCKING_ENABLED: &str = "blockingEnabled";
pub const DATA_POINT_QUERIES_HISTORY: &str = "queriesHistory";
pub const ACTION_SET_BLOCKING: &str = "setBlocking";
pub const ACTION_ADD_DOMAIN: &str = "addDomain";
pub const ACTION_REMOVE_DOMAIN: &str = "removeDomain";
pub const ACTION_TOGGLE_DOMAIN_ENABLED: &str = "toggleDomainEnabled";
pub const RESOURCE_KIND_DOMAINS: &str = "domains";
pub const RESOURCE_KIND_CLIENTS: &str = "clients";

pub const CAPABILITY_READ_STATS: &str = "readStats";
pub const CAPABILITY_READ_DOMAINS: &str = "readDomains";
pub const CAPABILITY_READ_CLIENTS: &str = "readClients";
pub const CAPABILITY_SET_BLOCKING: &str = ACTION_SET_BLOCKING;
pub const CAPABILITY_ADD_DOMAIN: &str = ACTION_ADD_DOMAIN;
pub const CAPABILITY_REMOVE_DOMAIN: &str = ACTION_REMOVE_DOMAIN;
pub const CAPABILITY_TOGGLE_DOMAIN_ENABLED: &str = ACTION_TOGGLE_DOMAIN_ENABLED;

const PATH_SUMMARY: &str = "stats/summary";
const PATH_HISTORY: &str = "history";
const PATH_BLOCKING: &str = "dns/blocking";
const PATH_DOMAINS: &str = "domains";
const PATH_TOP_CLIENTS: &str = "stats/top_clients";
const RESOURCE_ID_PARAM: &str = "resourceId";

/// Setup instructions published with the connector type catalog.
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        variants: vec![SetupGuideVariant {
            id: "application-password".to_owned(),
            label: "Connect via application password".to_owned(),
            description: "In the Pi-hole web interface, open Settings > Web interface / API and choose Configure app password. Generate a dedicated application password and enter it in Loom's Password field. Pi-hole shows it only once, so store it safely. An application password is preferred over the administrator password for scripts and automation: it can be revoked independently and signs in to the API without a TOTP code when two-factor authentication is enabled."
                .to_owned(),
            // There is no command or deployment template for this in Pi-hole;
            // the shared guide UI deliberately omits an empty template block.
            template: String::new(),
            toggles: Vec::new(),
            capability_requirements: Vec::new(),
        }],
    }
}

/// One configured Pi-hole v6 host.
pub struct PiHoleConnector {
    config: PiHoleConnectorConfig,
    client: PiHoleClient,
}

impl PiHoleConnector {
    /// Validates the configuration and proves Pi-hole authentication.
    pub async fn from_config_value(value: Value) -> Result<Self, ConnectorError> {
        let config = PiHoleConnectorConfig::from_value(value)?;
        let client = PiHoleClient::connect_with_certificate_policy(
            &config.base_url,
            &config.password,
            config.allow_insecure_cert,
        )
        .await
        .map_err(connector_error)?;
        Ok(Self { config, client })
    }

    /// Builds the ephemeral setup-test connector without making a request.
    pub fn from_config_value_for_connection_test(value: Value) -> Result<Self, ConnectorError> {
        let config = PiHoleConnectorConfig::from_value(value)?;
        let client = PiHoleClient::new_with_certificate_policy(
            &config.base_url,
            &config.password,
            config.allow_insecure_cert,
        )
        .map_err(connector_error)?;
        Ok(Self { config, client })
    }

    async fn read_status(&self) -> Result<Value, PiHoleError> {
        // Pi-hole exposes these as three host-level resources. Fetch them in
        // parallel: the summary already contains all five scalar metrics, so
        // there is no per-field request pattern to grow with the dashboard.
        let (summary, history, blocking) = tokio::join!(
            self.client.get_json(PATH_SUMMARY),
            self.client.get_json(PATH_HISTORY),
            self.client.get_json(PATH_BLOCKING),
        );
        let summary: SummaryResponse = parse_response(PATH_SUMMARY, summary?)?;
        let history: HistoryResponse = parse_response(PATH_HISTORY, history?)?;
        let blocking: BlockingResponse = parse_response(PATH_BLOCKING, blocking?)?;
        map_status_details(summary, history, blocking).map_err(|message| PiHoleError::ApiError {
            status: 200,
            message,
        })
    }

    async fn set_blocking(&self, params: Value) -> Result<ActionResult, ConnectorError> {
        let enabled = params
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| ConnectorError::InvalidParams {
                action_id: ACTION_SET_BLOCKING.to_owned(),
                reason: "enabled must be a boolean".to_owned(),
            })?;
        let response = self
            .client
            .post_json(PATH_BLOCKING, json!({ "blocking": enabled, "timer": null }))
            .await
            .map_err(connector_error)?;
        let response: BlockingResponse = parse_response(PATH_BLOCKING, response)
            .map_err(|error| ConnectorError::Internal(error.to_string()))?;
        let actual = blocking_enabled(&response.blocking)
            .map_err(|error| ConnectorError::Internal(error.to_string()))?;

        if actual != enabled {
            return Ok(ActionResult::failed(format!(
                "Pi-hole reported blocking as {} after the request",
                response.blocking
            ))
            .with_payload(json!({ "blockingEnabled": actual })));
        }
        Ok(ActionResult::ok(if enabled {
            "Pi-hole DNS blocking enabled"
        } else {
            "Pi-hole DNS blocking disabled"
        })
        .with_payload(json!({ "blockingEnabled": actual })))
    }

    async fn add_domain(&self, params: Value) -> Result<ActionResult, ConnectorError> {
        let domain = required_string_param(ACTION_ADD_DOMAIN, &params, "domain")?;
        let list_type = required_string_param(ACTION_ADD_DOMAIN, &params, "listType")?;
        validate_list_type(ACTION_ADD_DOMAIN, list_type)?;
        let comment = params
            .get("comment")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|comment| !comment.is_empty());
        self.client
            .post_json_segments(
                &["domains", list_type, "exact"],
                json!({ "domain": domain, "comment": comment }),
            )
            .await
            .map_err(connector_error)?;
        Ok(ActionResult::ok(format!(
            "Added {domain} to the Pi-hole {list_type} list."
        )))
    }

    async fn remove_domain(&self, params: Value) -> Result<ActionResult, ConnectorError> {
        let domain = self
            .domain_for_action(ACTION_REMOVE_DOMAIN, &params)
            .await?;
        self.client
            .delete_segments(&["domains", &domain.list_type, &domain.kind, &domain.domain])
            .await
            .map_err(connector_error)?;
        Ok(ActionResult::ok(format!(
            "Removed {} from the Pi-hole {} list.",
            domain.domain, domain.list_type
        )))
    }

    async fn toggle_domain(&self, params: Value) -> Result<ActionResult, ConnectorError> {
        let domain = self
            .domain_for_action(ACTION_TOGGLE_DOMAIN_ENABLED, &params)
            .await?;
        let enabled = !domain.enabled;
        self.client
            .put_json_segments(
                &["domains", &domain.list_type, &domain.kind, &domain.domain],
                json!({
                    "type": domain.list_type,
                    "kind": domain.kind,
                    "comment": domain.comment,
                    "groups": domain.groups,
                    "enabled": enabled
                }),
            )
            .await
            .map_err(connector_error)?;
        Ok(ActionResult::ok(format!(
            "{} the Pi-hole domain {}.",
            if enabled { "Enabled" } else { "Disabled" },
            domain.domain
        )))
    }

    async fn domain_for_action(
        &self,
        action_id: &str,
        params: &Value,
    ) -> Result<DomainEntry, ConnectorError> {
        let resource_id = required_string_param(action_id, params, RESOURCE_ID_PARAM)?;
        let resource_id =
            resource_id
                .parse::<i64>()
                .map_err(|_| ConnectorError::InvalidParams {
                    action_id: action_id.to_owned(),
                    reason: format!("`{RESOURCE_ID_PARAM}` is not a Pi-hole domain id"),
                })?;
        let response = self
            .client
            .get_json(PATH_DOMAINS)
            .await
            .map_err(connector_error)?;
        let response: DomainsResponse = parse_response(PATH_DOMAINS, response)
            .map_err(|error| ConnectorError::Internal(error.to_string()))?;
        response
            .domains
            .into_iter()
            .find(|domain| domain.id == resource_id && domain.kind == "exact")
            .ok_or_else(|| ConnectorError::InvalidParams {
                action_id: action_id.to_owned(),
                reason: format!("domain resource `{resource_id}` no longer exists"),
            })
    }
}

#[async_trait]
impl loom_core::connector::Connector for PiHoleConnector {
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
        match self.read_status().await {
            Ok(details) => Ok(ConnectorStatus::new(HealthState::Healthy, details)
                .with_target_health(String::new(), HealthState::Healthy)),
            Err(error) => {
                let mut details = Value::Object(Map::new());
                set_detail(&mut details, None, "error", json!(error.to_string()));
                Ok(ConnectorStatus::new(HealthState::Down, details)
                    .with_target_health(String::new(), HealthState::Down))
            }
        }
    }

    async fn test_connection(&self) -> ConnectionTestResult {
        if let Err(error) = self.client.authenticate().await {
            return ConnectionTestResult {
                reachable: false,
                capabilities: Vec::new(),
                message: Some(error.to_string()),
            };
        }

        let (summary, domains, clients) = tokio::join!(
            self.client.get_json(PATH_SUMMARY),
            self.client.get_json(PATH_DOMAINS),
            self.client.get_json(PATH_TOP_CLIENTS),
        );
        let read_stats = tested_read::<SummaryResponse>(PATH_SUMMARY, summary);
        let read_domains = tested_read::<DomainsResponse>(PATH_DOMAINS, domains);
        let read_clients = tested_read::<TopClientsResponse>(PATH_TOP_CLIENTS, clients);
        let all_reads_available =
            read_stats.available && read_domains.available && read_clients.available;

        ConnectionTestResult {
            reachable: true,
            capabilities: vec![
                read_stats,
                read_domains,
                read_clients,
                available_capability(CAPABILITY_SET_BLOCKING, "Control DNS blocking"),
                available_capability(CAPABILITY_ADD_DOMAIN, "Add domains"),
                available_capability(CAPABILITY_REMOVE_DOMAIN, "Remove domains"),
                available_capability(
                    CAPABILITY_TOGGLE_DOMAIN_ENABLED,
                    "Enable or disable domains",
                ),
            ],
            message: Some(if all_reads_available {
                "Authenticated successfully and verified statistics, domains, and top clients."
                    .to_owned()
            } else {
                "Authenticated successfully, but one or more read endpoints could not be verified."
                    .to_owned()
            }),
        }
    }

    async fn actions(&self) -> Vec<ConnectorAction> {
        vec![ConnectorAction {
            id: ACTION_SET_BLOCKING.to_owned(),
            target_id: None,
            label: "Set blocking".to_owned(),
            description: Some("Enable or disable Pi-hole DNS blocking permanently.".to_owned()),
            params_schema: json!({
                "type": "object",
                "properties": {
                    "enabled": { "type": "boolean" }
                },
                "required": ["enabled"],
                "additionalProperties": false
            }),
            is_disruptive: true,
            snapshot_data_point_ids: vec![DATA_POINT_BLOCKING_ENABLED.to_owned()],
        }]
    }

    async fn execute_action(
        &self,
        action_id: &str,
        target_id: Option<&str>,
        params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        if target_id.is_some() {
            return Err(ConnectorError::invalid_action(action_id));
        }
        match action_id {
            ACTION_SET_BLOCKING => self.set_blocking(params).await,
            ACTION_ADD_DOMAIN => self.add_domain(params).await,
            ACTION_REMOVE_DOMAIN => self.remove_domain(params).await,
            ACTION_TOGGLE_DOMAIN_ENABLED => self.toggle_domain(params).await,
            _ => Err(ConnectorError::invalid_action(action_id)),
        }
    }

    fn resource_kinds(&self, target_id: Option<&str>) -> Vec<ResourceKindDescriptor> {
        if target_id.is_some() {
            Vec::new()
        } else {
            resource_kinds()
        }
    }

    async fn list_resource_items(
        &self,
        kind: &str,
        target_id: Option<&str>,
    ) -> Result<Vec<ResourceItem>, ConnectorError> {
        if target_id.is_some() {
            return Ok(Vec::new());
        }
        match kind {
            RESOURCE_KIND_DOMAINS => {
                let response = self
                    .client
                    .get_json(PATH_DOMAINS)
                    .await
                    .map_err(connector_error)?;
                let response: DomainsResponse = parse_response(PATH_DOMAINS, response)
                    .map_err(|error| ConnectorError::Internal(error.to_string()))?;
                Ok(domain_resource_items(response.domains))
            }
            RESOURCE_KIND_CLIENTS => {
                let response = self
                    .client
                    .get_json(PATH_TOP_CLIENTS)
                    .await
                    .map_err(connector_error)?;
                let response: TopClientsResponse = parse_response(PATH_TOP_CLIENTS, response)
                    .map_err(|error| ConnectorError::Internal(error.to_string()))?;
                Ok(client_resource_items(response.clients))
            }
            _ => Ok(Vec::new()),
        }
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
            min_size: (3, 3),
        }
    }

    fn display_fields(&self) -> Vec<DisplayField> {
        vec![DisplayField::new(
            "Pi-hole address",
            self.config.base_url.clone(),
        )]
    }

    fn data_points(&self) -> Vec<DataPointDescriptor> {
        vec![
            DataPointDescriptor::new(
                DATA_POINT_QUERIES_TODAY,
                "Queries today",
                DataPointValueType::Number,
            ),
            DataPointDescriptor::new(
                DATA_POINT_QUERIES_BLOCKED_TODAY,
                "Blocked today",
                DataPointValueType::Number,
            ),
            DataPointDescriptor::new(
                DATA_POINT_BLOCK_PERCENTAGE,
                "Blocked",
                DataPointValueType::Number,
            )
            .with_unit("%"),
            DataPointDescriptor::new(
                DATA_POINT_DOMAINS_ON_BLOCKLIST,
                "Domains on blocklist",
                DataPointValueType::Number,
            ),
            DataPointDescriptor::new(
                DATA_POINT_UNIQUE_CLIENTS,
                "Active clients",
                DataPointValueType::Number,
            ),
            DataPointDescriptor::new(
                DATA_POINT_BLOCKING_ENABLED,
                "Blocking enabled",
                DataPointValueType::Bool,
            ),
            DataPointDescriptor::new(
                DATA_POINT_QUERIES_HISTORY,
                "Query volume",
                DataPointValueType::TimeSeries,
            ),
        ]
    }

    fn default_layout(&self) -> WidgetLayout {
        WidgetLayout::new(vec![
            WidgetBinding::display(DATA_POINT_QUERIES_TODAY, DisplayWidgetType::StatTile),
            WidgetBinding::display(
                DATA_POINT_QUERIES_BLOCKED_TODAY,
                DisplayWidgetType::StatTile,
            ),
            WidgetBinding::display(DATA_POINT_BLOCK_PERCENTAGE, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_DOMAINS_ON_BLOCKLIST, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_UNIQUE_CLIENTS, DisplayWidgetType::StatTile),
            WidgetBinding::display(
                DATA_POINT_QUERIES_HISTORY,
                DisplayWidgetType::MetricChart {
                    chart_type: ChartType::Line,
                },
            ),
            WidgetBinding::action(ACTION_SET_BLOCKING, ActionWidgetType::Toggle).with_config(
                json!({
                    "paramName": "enabled",
                    "stateDataPointId": DATA_POINT_BLOCKING_ENABLED
                }),
            ),
        ])
    }

    fn network_target(&self) -> Option<NetworkTarget> {
        self.config.network_target()
    }
}

#[derive(Debug, Deserialize)]
struct SummaryResponse {
    queries: QuerySummary,
    clients: ClientSummary,
    gravity: GravitySummary,
}

#[derive(Debug, Deserialize)]
struct QuerySummary {
    total: u64,
    blocked: u64,
    percent_blocked: f64,
}

#[derive(Debug, Deserialize)]
struct ClientSummary {
    active: u64,
}

#[derive(Debug, Deserialize)]
struct GravitySummary {
    domains_being_blocked: u64,
}

#[derive(Debug, Deserialize)]
struct HistoryResponse {
    history: Vec<HistoryBucket>,
}

#[derive(Debug, Deserialize)]
struct HistoryBucket {
    timestamp: f64,
    total: u64,
}

#[derive(Debug, Deserialize)]
struct BlockingResponse {
    blocking: String,
}

#[derive(Debug, Deserialize)]
struct DomainsResponse {
    domains: Vec<DomainEntry>,
}

#[derive(Clone, Debug, Deserialize)]
struct DomainEntry {
    id: i64,
    domain: String,
    #[serde(rename = "type")]
    list_type: String,
    kind: String,
    comment: Option<String>,
    #[serde(default)]
    groups: Vec<i64>,
    enabled: bool,
}

#[derive(Debug, Deserialize)]
struct TopClientsResponse {
    clients: Vec<TopClient>,
}

#[derive(Debug, Deserialize)]
struct TopClient {
    ip: String,
    name: Option<String>,
    count: u64,
}

fn resource_kinds() -> Vec<ResourceKindDescriptor> {
    vec![
        ResourceKindDescriptor::new(
            RESOURCE_KIND_DOMAINS,
            "Domains",
            vec![
                ColumnDescriptor::new("domain", "Domain", ColumnValueType::Text),
                ColumnDescriptor::new("type", "Type", ColumnValueType::Text),
                ColumnDescriptor::new("comment", "Comment", ColumnValueType::Text),
                ColumnDescriptor::new("enabled", "Enabled", ColumnValueType::Bool),
            ],
        )
        .applicable_to(ApplicableTarget::HostOnly)
        .with_row_actions(vec![
            resource_row_action(
                ACTION_TOGGLE_DOMAIN_ENABLED,
                "Toggle enabled",
                "Enable or disable this exact allow/deny-list entry without removing it.",
            ),
            resource_row_action(
                ACTION_REMOVE_DOMAIN,
                "Remove",
                "Remove this exact entry from Pi-hole's domain lists.",
            ),
        ])
        .with_kind_actions(vec![ConnectorAction {
            id: ACTION_ADD_DOMAIN.to_owned(),
            target_id: None,
            label: "Add domain".to_owned(),
            description: Some("Add an exact domain to Pi-hole's allow or deny list.".to_owned()),
            params_schema: json!({
                "type": "object",
                "properties": {
                    "domain": {
                        "type": "string",
                        "title": "Domain",
                        "minLength": 1
                    },
                    "listType": {
                        "type": "string",
                        "title": "List type",
                        "default": "deny",
                        "description": "Enter `allow` or `deny`."
                    },
                    "comment": {
                        "type": "string",
                        "title": "Comment",
                        "description": "Optional note stored with this Pi-hole entry."
                    }
                },
                "required": ["domain", "listType"],
                "additionalProperties": false
            }),
            is_disruptive: false,
            snapshot_data_point_ids: Vec::new(),
        }]),
        ResourceKindDescriptor::new(
            RESOURCE_KIND_CLIENTS,
            "Clients",
            vec![
                ColumnDescriptor::new("client", "Client", ColumnValueType::Text),
                ColumnDescriptor::new("queryCount", "Queries", ColumnValueType::Number),
            ],
        )
        .applicable_to(ApplicableTarget::HostOnly),
    ]
}

fn resource_row_action(id: &str, label: &str, description: &str) -> ConnectorAction {
    ConnectorAction {
        id: id.to_owned(),
        target_id: None,
        label: label.to_owned(),
        description: Some(description.to_owned()),
        params_schema: json!({
            "type": "object",
            "properties": {
                RESOURCE_ID_PARAM: {
                    "type": "string",
                    "title": "Resource",
                    "description": "The row this action applies to."
                }
            },
            "required": [RESOURCE_ID_PARAM],
            "additionalProperties": false
        }),
        is_disruptive: false,
        snapshot_data_point_ids: Vec::new(),
    }
}

fn domain_resource_items(domains: Vec<DomainEntry>) -> Vec<ResourceItem> {
    let mut domains = domains
        .into_iter()
        // The requested four-column contract cannot distinguish exact and
        // regex rows. Manage exact entries only; regex management can become a
        // separate kind when its pattern semantics have an explicit UI.
        .filter(|domain| {
            domain.kind == "exact" && matches!(domain.list_type.as_str(), "allow" | "deny")
        })
        .collect::<Vec<_>>();
    domains.sort_by(|left, right| {
        left.domain
            .to_ascii_lowercase()
            .cmp(&right.domain.to_ascii_lowercase())
            .then_with(|| left.list_type.cmp(&right.list_type))
    });
    domains
        .into_iter()
        .map(|domain| {
            ResourceItem::new(domain.id.to_string())
                .with_field("domain", domain.domain)
                .with_field("type", domain.list_type)
                .with_field("comment", domain.comment.unwrap_or_default())
                .with_field("enabled", domain.enabled)
        })
        .collect()
}

fn client_resource_items(clients: Vec<TopClient>) -> Vec<ResourceItem> {
    clients
        .into_iter()
        .map(|client| {
            let label = client
                .name
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| client.ip.clone());
            ResourceItem::new(client.ip)
                .with_field("client", label)
                .with_field("queryCount", client.count)
        })
        .collect()
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
        .ok_or_else(|| ConnectorError::InvalidParams {
            action_id: action_id.to_owned(),
            reason: format!("`{key}` must be a non-empty string"),
        })
}

fn validate_list_type(action_id: &str, list_type: &str) -> Result<(), ConnectorError> {
    if matches!(list_type, "allow" | "deny") {
        Ok(())
    } else {
        Err(ConnectorError::InvalidParams {
            action_id: action_id.to_owned(),
            reason: "`listType` must be `allow` or `deny`".to_owned(),
        })
    }
}

fn tested_read<T: for<'de> Deserialize<'de>>(
    path: &str,
    result: Result<Value, PiHoleError>,
) -> CapabilityStatus {
    let (key, label) = match path {
        PATH_SUMMARY => (CAPABILITY_READ_STATS, "Read statistics"),
        PATH_DOMAINS => (CAPABILITY_READ_DOMAINS, "Browse domains"),
        PATH_TOP_CLIENTS => (CAPABILITY_READ_CLIENTS, "Browse top clients"),
        _ => (path, path),
    };
    match result.and_then(|value| parse_response::<T>(path, value)) {
        Ok(_) => available_capability(key, label),
        Err(error) => CapabilityStatus {
            key: key.to_owned(),
            label: label.to_owned(),
            available: false,
            note: Some(error.to_string()),
        },
    }
}

fn available_capability(key: &str, label: &str) -> CapabilityStatus {
    CapabilityStatus {
        key: key.to_owned(),
        label: label.to_owned(),
        available: true,
        note: None,
    }
}

fn parse_response<T: for<'de> Deserialize<'de>>(
    path: &str,
    value: Value,
) -> Result<T, PiHoleError> {
    serde_json::from_value(value).map_err(|error| PiHoleError::ApiError {
        status: 200,
        message: format!("{path} returned an unexpected response: {error}"),
    })
}

fn map_status_details(
    summary: SummaryResponse,
    history: HistoryResponse,
    blocking: BlockingResponse,
) -> Result<Value, String> {
    if !summary.queries.percent_blocked.is_finite() {
        return Err("stats/summary returned a non-finite blocked percentage".to_owned());
    }
    let mut points = history
        .history
        .into_iter()
        .map(|bucket| {
            let millis = epoch_millis(bucket.timestamp)?;
            let timestamp = DateTime::<Utc>::from_timestamp_millis(millis)
                .ok_or_else(|| {
                    format!("history timestamp {millis} is outside the supported range")
                })?
                .to_rfc3339_opts(SecondsFormat::Millis, true);
            Ok((
                millis,
                json!({ "timestamp": timestamp, "value": bucket.total }),
            ))
        })
        .collect::<Result<Vec<_>, String>>()?;
    points.sort_by_key(|(millis, _)| *millis);

    let mut details = Value::Object(Map::new());
    for (id, value) in [
        (DATA_POINT_QUERIES_TODAY, json!(summary.queries.total)),
        (
            DATA_POINT_QUERIES_BLOCKED_TODAY,
            json!(summary.queries.blocked),
        ),
        (
            DATA_POINT_BLOCK_PERCENTAGE,
            json!(summary.queries.percent_blocked),
        ),
        (
            DATA_POINT_DOMAINS_ON_BLOCKLIST,
            json!(summary.gravity.domains_being_blocked),
        ),
        (DATA_POINT_UNIQUE_CLIENTS, json!(summary.clients.active)),
        (
            DATA_POINT_BLOCKING_ENABLED,
            json!(blocking_enabled(&blocking.blocking)?),
        ),
        (
            DATA_POINT_QUERIES_HISTORY,
            Value::Array(points.into_iter().map(|(_, point)| point).collect()),
        ),
    ] {
        set_detail(&mut details, None, id, value);
    }
    Ok(details)
}

fn epoch_millis(seconds: f64) -> Result<i64, String> {
    let millis = seconds * 1_000.0;
    if !millis.is_finite() || millis < i64::MIN as f64 || millis > i64::MAX as f64 {
        return Err(format!("history returned invalid timestamp {seconds}"));
    }
    Ok(millis.round() as i64)
}

fn blocking_enabled(value: &str) -> Result<bool, String> {
    match value {
        "enabled" => Ok(true),
        "disabled" => Ok(false),
        other => Err(format!("dns/blocking returned unexpected state `{other}`")),
    }
}

fn connector_error(error: PiHoleError) -> ConnectorError {
    match error {
        PiHoleError::ConnectionFailed(reason) => ConnectorError::unreachable(reason),
        PiHoleError::AuthFailed(reason) => ConnectorError::AuthFailed { reason },
        PiHoleError::ApiError { status, message } => {
            ConnectorError::Internal(format!("Pi-hole API returned HTTP {status}: {message}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use loom_core::connector::details::get_detail;

    use super::*;

    #[test]
    fn current_v6_responses_map_to_nested_data_points() {
        let summary: SummaryResponse = serde_json::from_value(json!({
            "queries": { "total": 7497, "blocked": 3465, "percent_blocked": 34.5 },
            "clients": { "active": 10, "total": 22 },
            "gravity": { "domains_being_blocked": 104756, "last_update": 1725194639 },
            "took": 0.001
        }))
        .expect("official summary shape");
        let history: HistoryResponse = serde_json::from_value(json!({
            "history": [
                { "timestamp": 1511820500.583821, "total": 2014, "cached": 52, "blocked": 43, "forwarded": 1910 },
                { "timestamp": 1511819900.539157, "total": 2134, "cached": 525, "blocked": 413, "forwarded": 1196 }
            ],
            "took": 0.001
        }))
        .expect("official history shape");
        let blocking: BlockingResponse =
            serde_json::from_value(json!({ "blocking": "enabled", "timer": null }))
                .expect("official blocking shape");

        let details = map_status_details(summary, history, blocking).expect("mapped status");
        assert_eq!(
            get_detail(&details, None, DATA_POINT_QUERIES_TODAY),
            Some(&json!(7497))
        );
        assert_eq!(
            get_detail(&details, None, DATA_POINT_QUERIES_BLOCKED_TODAY),
            Some(&json!(3465))
        );
        assert_eq!(
            get_detail(&details, None, DATA_POINT_BLOCK_PERCENTAGE),
            Some(&json!(34.5))
        );
        assert_eq!(
            get_detail(&details, None, DATA_POINT_UNIQUE_CLIENTS),
            Some(&json!(10))
        );
        assert_eq!(
            get_detail(&details, None, DATA_POINT_BLOCKING_ENABLED),
            Some(&json!(true))
        );
        let series = get_detail(&details, None, DATA_POINT_QUERIES_HISTORY)
            .and_then(Value::as_array)
            .expect("time series");
        assert_eq!(series.len(), 2);
        assert_eq!(series[0]["value"], 2134);
        assert_eq!(series[1]["value"], 2014);
        assert!(series[0]["timestamp"]
            .as_str()
            .is_some_and(|timestamp| timestamp.ends_with('Z')));
    }

    #[test]
    fn unknown_blocking_states_are_not_misreported_as_disabled() {
        assert_eq!(blocking_enabled("enabled"), Ok(true));
        assert_eq!(blocking_enabled("disabled"), Ok(false));
        assert!(blocking_enabled("failed").is_err());
        assert!(blocking_enabled("unknown").is_err());
    }

    #[test]
    fn default_layout_uses_the_live_blocking_state_for_its_toggle() {
        let binding = WidgetBinding::action(ACTION_SET_BLOCKING, ActionWidgetType::Toggle)
            .with_config(json!({
                "paramName": "enabled",
                "stateDataPointId": DATA_POINT_BLOCKING_ENABLED
            }));
        assert_eq!(
            serde_json::to_value(binding).expect("serializable")["action"]["config"]
                ["stateDataPointId"],
            DATA_POINT_BLOCKING_ENABLED
        );
    }

    #[test]
    fn official_domain_rows_map_to_exact_allow_and_deny_resources() {
        let response: DomainsResponse = serde_json::from_value(json!({
            "domains": [
                {
                    "id": 12,
                    "domain": "blocked.example",
                    "unicode": "blocked.example",
                    "type": "deny",
                    "kind": "exact",
                    "comment": "test entry",
                    "groups": [0],
                    "enabled": true,
                    "date_added": 1611239095,
                    "date_modified": 1612163756
                },
                {
                    "id": 13,
                    "domain": "allowed.example",
                    "type": "allow",
                    "kind": "exact",
                    "comment": null,
                    "groups": [0],
                    "enabled": false
                },
                {
                    "id": 14,
                    "domain": "(^|\\.)regex.example$",
                    "type": "deny",
                    "kind": "regex",
                    "comment": null,
                    "groups": [0],
                    "enabled": true
                }
            ],
            "took": 0.012
        }))
        .expect("official domains shape");

        let rows = domain_resource_items(response.domains);
        assert_eq!(
            rows.len(),
            2,
            "regex rules are deliberately a separate concern"
        );
        assert_eq!(rows[0].fields["domain"], "allowed.example");
        assert_eq!(rows[0].fields["type"], "allow");
        assert_eq!(rows[0].fields["comment"], "");
        assert_eq!(rows[0].fields["enabled"], false);
        assert_eq!(rows[1].id, "12");
        assert_eq!(rows[1].fields["domain"], "blocked.example");
    }

    #[test]
    fn official_top_clients_rows_prefer_a_resolved_name_and_fall_back_to_ip() {
        let response: TopClientsResponse = serde_json::from_value(json!({
            "clients": [
                { "ip": "192.0.2.20", "name": "workstation.example", "count": 5896 },
                { "ip": "192.0.2.21", "name": null, "count": 42 },
                { "ip": "192.0.2.22", "name": "", "count": 7 }
            ],
            "total_queries": 5945,
            "blocked_queries": 123,
            "took": 0.001
        }))
        .expect("official top-clients shape");

        let rows = client_resource_items(response.clients);
        assert_eq!(rows[0].id, "192.0.2.20");
        assert_eq!(rows[0].fields["client"], "workstation.example");
        assert_eq!(rows[0].fields["queryCount"], 5896);
        assert_eq!(rows[1].fields["client"], "192.0.2.21");
        assert_eq!(rows[2].fields["client"], "192.0.2.22");
    }

    #[test]
    fn resource_descriptors_and_setup_guide_are_host_only_and_generic() {
        let kinds = resource_kinds();
        assert_eq!(kinds.len(), 2);
        assert!(kinds
            .iter()
            .all(|kind| kind.applicable_target == ApplicableTarget::HostOnly));
        let domains = kinds
            .iter()
            .find(|kind| kind.kind == RESOURCE_KIND_DOMAINS)
            .expect("domains kind");
        assert_eq!(domains.row_actions.len(), 2);
        assert_eq!(domains.kind_actions[0].id, ACTION_ADD_DOMAIN);
        let clients = kinds
            .iter()
            .find(|kind| kind.kind == RESOURCE_KIND_CLIENTS)
            .expect("clients kind");
        assert!(clients.row_actions.is_empty());
        assert!(clients.kind_actions.is_empty());

        let guide = setup_guide();
        assert_eq!(guide.variants.len(), 1);
        assert_eq!(guide.variants[0].id, "application-password");
        assert!(guide.variants[0].template.is_empty());
        assert!(guide.variants[0].toggles.is_empty());
        assert!(guide.variants[0].capability_requirements.is_empty());
    }
}
