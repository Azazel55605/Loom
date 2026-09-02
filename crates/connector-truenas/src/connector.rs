//! Minimal host-level TrueNAS connector.
//!
//! This first connector pass intentionally exposes no sub-targets, resource
//! kinds, discovery, setup guide, or detailed capability test. Those remain on
//! the `Connector` trait defaults until the transport-backed host readings have
//! been proven in normal deployments.

use async_trait::async_trait;
use loom_core::connector::{
    details::set_detail, ActionResult, ConnectorAction, ConnectorError, ConnectorMetadata,
    ConnectorStatus, DataPointDescriptor, DataPointValueType, DisplayField, DisplayWidgetType,
    HealthState, NetworkTarget, WidgetBinding, WidgetLayout,
};
use serde_json::{json, Map, Value};

use crate::{config::TrueNasConnectorConfig, TrueNasClient, TrueNasError};

pub const TYPE_ID: &str = "truenas";
pub const DISPLAY_NAME: &str = "TrueNAS";
pub const ICON: &str = "brand:truenas";

pub const DATA_POINT_POOL_COUNT: &str = "poolCount";
pub const DATA_POINT_TOTAL_CAPACITY_BYTES: &str = "totalCapacityBytes";
pub const DATA_POINT_USED_CAPACITY_BYTES: &str = "usedCapacityBytes";
pub const DATA_POINT_FREE_CAPACITY_BYTES: &str = "freeCapacityBytes";
pub const DATA_POINT_TRUENAS_VERSION: &str = "truenasVersion";

const METHOD_SYSTEM_INFO: &str = "system.info";
const METHOD_POOL_QUERY: &str = "pool.query";

/// One configured and authenticated TrueNAS host.
#[derive(Debug)]
pub struct TrueNasConnector {
    config: TrueNasConnectorConfig,
    client: TrueNasClient,
}

impl TrueNasConnector {
    /// Validates configuration and proves the WSS/authentication boundary.
    pub async fn from_config_value(value: Value) -> Result<Self, ConnectorError> {
        let config = TrueNasConnectorConfig::from_value(value)?;
        let client =
            TrueNasClient::connect(&config.host, &config.api_key, config.allow_insecure_cert)
                .await
                .map_err(connector_error)?;

        Ok(Self { config, client })
    }

    async fn read_host(&self) -> Result<HostReadings, String> {
        // Both methods are independent and the transport correlates concurrent
        // JSON-RPC calls, so one slow pool query need not delay system.info.
        let (system, pools) = tokio::join!(
            self.client.call(METHOD_SYSTEM_INFO, json!([])),
            self.client.call(METHOD_POOL_QUERY, json!([])),
        );
        map_host_readings(
            system.map_err(|error| error.to_string())?,
            pools.map_err(|error| error.to_string())?,
        )
    }
}

#[async_trait]
impl loom_core::connector::Connector for TrueNasConnector {
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
        let readings = match self.read_host().await {
            Ok(readings) => readings,
            Err(error) => return Ok(unavailable_status(&error.to_string())),
        };

        let mut details = Value::Object(Map::new());
        for (id, value) in [
            (DATA_POINT_POOL_COUNT, json!(readings.pool_count)),
            (
                DATA_POINT_TOTAL_CAPACITY_BYTES,
                json!(readings.total_capacity_bytes),
            ),
            (
                DATA_POINT_USED_CAPACITY_BYTES,
                json!(readings.used_capacity_bytes),
            ),
            (
                DATA_POINT_FREE_CAPACITY_BYTES,
                json!(readings.free_capacity_bytes),
            ),
            (DATA_POINT_TRUENAS_VERSION, json!(readings.version)),
        ] {
            set_detail(&mut details, None, id, value);
        }

        Ok(ConnectorStatus::new(HealthState::Healthy, details))
    }

    async fn actions(&self) -> Vec<ConnectorAction> {
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
            min_size: (2, 2),
        }
    }

    fn display_fields(&self) -> Vec<DisplayField> {
        vec![DisplayField::new("TrueNAS host", self.config.host.clone())]
    }

    fn data_points(&self) -> Vec<DataPointDescriptor> {
        // `system.info.physmem` is total installed RAM, not usage, and
        // `loadavg` is not a CPU percentage. Neither is mislabeled as the
        // requested utilization reading in this minimal pass.
        vec![
            DataPointDescriptor::new(DATA_POINT_POOL_COUNT, "Pools", DataPointValueType::Number),
            DataPointDescriptor::new(
                DATA_POINT_TOTAL_CAPACITY_BYTES,
                "Total capacity",
                DataPointValueType::Number,
            )
            .with_unit("bytes"),
            DataPointDescriptor::new(
                DATA_POINT_USED_CAPACITY_BYTES,
                "Used capacity",
                DataPointValueType::Number,
            )
            .with_unit("bytes"),
            DataPointDescriptor::new(
                DATA_POINT_FREE_CAPACITY_BYTES,
                "Free capacity",
                DataPointValueType::Number,
            )
            .with_unit("bytes"),
            DataPointDescriptor::new(
                DATA_POINT_TRUENAS_VERSION,
                "TrueNAS version",
                DataPointValueType::String,
            ),
        ]
    }

    fn default_layout(&self) -> WidgetLayout {
        WidgetLayout::new(vec![
            WidgetBinding::display(DATA_POINT_POOL_COUNT, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_TOTAL_CAPACITY_BYTES, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_TRUENAS_VERSION, DisplayWidgetType::StatTile),
            WidgetBinding::display(
                DATA_POINT_USED_CAPACITY_BYTES,
                DisplayWidgetType::ProgressBar,
            )
            .with_config(json!({
                "min": 0,
                "maxDataPointId": DATA_POINT_TOTAL_CAPACITY_BYTES
            })),
        ])
    }

    fn network_target(&self) -> Option<NetworkTarget> {
        Some(NetworkTarget::new(
            self.config.host.trim_matches(['[', ']']),
            443,
        ))
    }
}

#[derive(Debug, PartialEq, Eq)]
struct HostReadings {
    version: String,
    pool_count: u64,
    total_capacity_bytes: u64,
    used_capacity_bytes: u64,
    free_capacity_bytes: u64,
}

fn map_host_readings(system: Value, pools: Value) -> Result<HostReadings, String> {
    let version = required_string(&system, "version", METHOD_SYSTEM_INFO)?.to_owned();
    // Hostname is a required system.info field in the stable schema. Validate
    // it even though this minimal dashboard does not yet publish it as a data
    // point, so a response from the wrong method shape cannot look healthy.
    required_string(&system, "hostname", METHOD_SYSTEM_INFO)?;

    let pools = pools
        .as_array()
        .ok_or_else(|| format!("{METHOD_POOL_QUERY} did not return an array"))?;
    let mut total_capacity_bytes = 0_u64;
    let mut used_capacity_bytes = 0_u64;
    let mut free_capacity_bytes = 0_u64;
    for pool in pools {
        total_capacity_bytes = checked_capacity_sum(
            total_capacity_bytes,
            optional_bytes(pool, "size", METHOD_POOL_QUERY)?,
        )?;
        used_capacity_bytes = checked_capacity_sum(
            used_capacity_bytes,
            optional_bytes(pool, "allocated", METHOD_POOL_QUERY)?,
        )?;
        free_capacity_bytes = checked_capacity_sum(
            free_capacity_bytes,
            optional_bytes(pool, "free", METHOD_POOL_QUERY)?,
        )?;
    }

    Ok(HostReadings {
        version,
        pool_count: pools.len() as u64,
        total_capacity_bytes,
        used_capacity_bytes,
        free_capacity_bytes,
    })
}

fn required_string<'a>(value: &'a Value, key: &str, method: &str) -> Result<&'a str, String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| format!("{method} returned no usable `{key}` field"))
}

fn optional_bytes(value: &Value, key: &str, method: &str) -> Result<u64, String> {
    match value.get(key) {
        Some(Value::Null) => Ok(0),
        Some(value) => value
            .as_u64()
            .ok_or_else(|| format!("{method} returned a non-byte `{key}` value")),
        None => Err(format!("{method} returned no `{key}` field")),
    }
}

fn checked_capacity_sum(total: u64, next: u64) -> Result<u64, String> {
    total
        .checked_add(next)
        .ok_or_else(|| "TrueNAS pool capacities exceeded the supported numeric range".to_owned())
}

fn unavailable_status(reason: &str) -> ConnectorStatus {
    let mut details = Value::Object(Map::new());
    for (id, value) in [
        (DATA_POINT_POOL_COUNT, json!(0)),
        (DATA_POINT_TOTAL_CAPACITY_BYTES, json!(0)),
        (DATA_POINT_USED_CAPACITY_BYTES, json!(0)),
        (DATA_POINT_FREE_CAPACITY_BYTES, json!(0)),
        (DATA_POINT_TRUENAS_VERSION, json!("unavailable")),
        ("error", json!(reason)),
    ] {
        set_detail(&mut details, None, id, value);
    }
    ConnectorStatus::new(HealthState::Down, details)
}

fn connector_error(error: TrueNasError) -> ConnectorError {
    match error {
        TrueNasError::AuthFailed(reason) => ConnectorError::AuthFailed { reason },
        TrueNasError::ConnectionFailed(reason) => ConnectorError::unreachable(reason),
        TrueNasError::Timeout => ConnectorError::unreachable("the TrueNAS connection timed out"),
        TrueNasError::Disconnected => {
            ConnectorError::unreachable("the TrueNAS connection was lost")
        }
        TrueNasError::RpcError { code, message } => ConnectorError::Internal(format!(
            "TrueNAS returned RPC error {code} while connecting: {message}"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documented_responses_map_to_host_data_points() {
        let readings = map_host_readings(
            json!({
                "version": "TrueNAS-SCALE-25.10.0",
                "hostname": "nas"
            }),
            json!([
                { "name": "tank", "size": 1000, "allocated": 400, "free": 600 },
                { "name": "backup", "size": 2000, "allocated": 750, "free": 1250 }
            ]),
        )
        .expect("documented response shape");

        assert_eq!(
            readings,
            HostReadings {
                version: "TrueNAS-SCALE-25.10.0".to_owned(),
                pool_count: 2,
                total_capacity_bytes: 3000,
                used_capacity_bytes: 1150,
                free_capacity_bytes: 1850,
            }
        );
    }

    #[test]
    fn nullable_capacity_fields_contribute_zero_without_hiding_the_pool() {
        let readings = map_host_readings(
            json!({ "version": "25.10", "hostname": "nas" }),
            json!([{ "size": null, "allocated": null, "free": null }]),
        )
        .expect("nullable fields are documented");
        assert_eq!(readings.pool_count, 1);
        assert_eq!(readings.total_capacity_bytes, 0);
    }

    #[test]
    fn descriptors_and_layout_are_host_only_and_consistent() {
        let points = [
            DATA_POINT_POOL_COUNT,
            DATA_POINT_TOTAL_CAPACITY_BYTES,
            DATA_POINT_USED_CAPACITY_BYTES,
            DATA_POINT_FREE_CAPACITY_BYTES,
            DATA_POINT_TRUENAS_VERSION,
        ];
        let layout = WidgetLayout::new(vec![
            WidgetBinding::display(DATA_POINT_POOL_COUNT, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_TOTAL_CAPACITY_BYTES, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_TRUENAS_VERSION, DisplayWidgetType::StatTile),
            WidgetBinding::display(
                DATA_POINT_USED_CAPACITY_BYTES,
                DisplayWidgetType::ProgressBar,
            )
            .with_config(json!({
                "min": 0,
                "maxDataPointId": DATA_POINT_TOTAL_CAPACITY_BYTES
            })),
        ]);
        for binding in layout.bindings {
            let WidgetBinding::Display { data_point_id, .. } = binding else {
                panic!("the read-only connector must not ship action widgets");
            };
            assert!(points.contains(&data_point_id.as_str()));
        }
    }

    #[test]
    fn construction_errors_preserve_authentication_and_network_categories() {
        assert!(matches!(
            connector_error(TrueNasError::AuthFailed("bad key".to_owned())),
            ConnectorError::AuthFailed { .. }
        ));
        assert!(matches!(
            connector_error(TrueNasError::ConnectionFailed("refused".to_owned())),
            ConnectorError::Unreachable { .. }
        ));
    }
}
