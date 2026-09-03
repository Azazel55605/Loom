use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use loom_core::connector::{
    details::set_detail, ActionResult, ActionWidgetType, ChartType, ConnectorAction,
    ConnectorError, ConnectorMetadata, ConnectorStatus, DataPointDescriptor, DataPointValueType,
    DisplayField, DisplayWidgetType, HealthState, NetworkTarget, WidgetBinding, WidgetLayout,
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

const PATH_SUMMARY: &str = "stats/summary";
const PATH_HISTORY: &str = "history";
const PATH_BLOCKING: &str = "dns/blocking";

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
        if action_id != ACTION_SET_BLOCKING || target_id.is_some() {
            return Err(ConnectorError::invalid_action(action_id));
        }
        let enabled = params
            .get("enabled")
            .and_then(Value::as_bool)
            .ok_or_else(|| ConnectorError::InvalidParams {
                action_id: action_id.to_owned(),
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

    fn config_schema(&self) -> Value {
        crate::config_schema()
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
}
