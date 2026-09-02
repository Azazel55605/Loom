//! TrueNAS host, pool, and dataset connector surface.

use std::sync::Mutex;

use async_trait::async_trait;
use futures_util::future::join_all;
use loom_core::connector::{
    details::set_detail, ActionResult, ActionWidgetType, ConnectorAction, ConnectorError,
    ConnectorMetadata, ConnectorStatus, DataPointDescriptor, DataPointValueType, DisplayField,
    DisplayWidgetType, HealthState, NetworkTarget, SubTarget, WidgetBinding, WidgetLayout,
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
pub const DATA_POINT_STATUS: &str = "status";
pub const DATA_POINT_USED_BYTES: &str = "usedBytes";
pub const DATA_POINT_FREE_BYTES: &str = "freeBytes";
pub const DATA_POINT_CAPACITY_PERCENT: &str = "capacityPercent";
pub const DATA_POINT_AVAILABLE_BYTES: &str = "availableBytes";
pub const DATA_POINT_COMPRESSION_RATIO: &str = "compressionRatio";
pub const DATA_POINT_SNAPSHOT_COUNT: &str = "snapshotCount";
pub const ACTION_START_SCRUB: &str = "startScrub";

const METHOD_SYSTEM_INFO: &str = "system.info";
const METHOD_POOL_QUERY: &str = "pool.query";
const METHOD_DATASET_QUERY: &str = "pool.dataset.query";
const METHOD_SNAPSHOT_COUNT: &str = "pool.dataset.snapshot_count";
const METHOD_POOL_SCRUB: &str = "pool.scrub.scrub";

/// One configured and authenticated TrueNAS host.
#[derive(Debug)]
pub struct TrueNasConnector {
    config: TrueNasConnectorConfig,
    client: TrueNasClient,
    known_targets: Mutex<Vec<SubTarget>>,
}

impl TrueNasConnector {
    /// Validates configuration and proves the WSS/authentication boundary.
    pub async fn from_config_value(value: Value) -> Result<Self, ConnectorError> {
        let config = TrueNasConnectorConfig::from_value(value)?;
        let client = match config.username.as_deref() {
            Some(username) => {
                TrueNasClient::connect_with_username(
                    &config.host,
                    username,
                    &config.api_key,
                    config.allow_insecure_cert,
                )
                .await
            }
            // Compatibility for connector rows stored before `username` was
            // added. New submissions cannot take this path because the
            // published schema requires the field.
            None => {
                TrueNasClient::connect(&config.host, &config.api_key, config.allow_insecure_cert)
                    .await
            }
        }
        .map_err(connector_error)?;

        Ok(Self {
            config,
            client,
            known_targets: Mutex::new(Vec::new()),
        })
    }

    async fn read_inventory(&self) -> Result<InventoryReadings, String> {
        // These methods are independent and the transport correlates concurrent
        // JSON-RPC calls, so one slow inventory query need not delay the rest.
        let (system, pools, datasets) = tokio::join!(
            self.client.call(METHOD_SYSTEM_INFO, json!([])),
            self.client.call(METHOD_POOL_QUERY, json!([])),
            self.client
                .call(METHOD_DATASET_QUERY, dataset_query_params()),
        );
        // Host identity and pool health are the connector's availability
        // boundary. Dataset telemetry enriches that reading, but an API key
        // without DATASET_READ (or one inaccessible dataset) must not make an
        // otherwise reachable TrueNAS host look offline.
        let system = system.map_err(|error| error.to_string())?;
        let pools = map_pool_readings(pools.map_err(|error| error.to_string())?)?;
        let mut warnings = Vec::new();
        let mut datasets = match datasets {
            Ok(value) => match map_dataset_readings(value) {
                Ok(datasets) => datasets,
                Err(error) => {
                    warnings.push(error);
                    Vec::new()
                }
            },
            Err(error) => {
                warnings.push(format!("{METHOD_DATASET_QUERY} failed: {error}"));
                Vec::new()
            }
        };

        let counts = join_all(datasets.iter().map(|dataset| {
            self.client
                .call(METHOD_SNAPSHOT_COUNT, json!([dataset.path.as_str()]))
        }))
        .await;
        for (dataset, count) in datasets.iter_mut().zip(counts) {
            match count {
                Ok(value) => match value.as_u64() {
                    Some(count) => dataset.snapshot_count = Some(count),
                    None => warnings.push(format!(
                        "{METHOD_SNAPSHOT_COUNT} returned no unsigned integer for `{}`",
                        dataset.path
                    )),
                },
                Err(error) => warnings.push(format!(
                    "{METHOD_SNAPSHOT_COUNT} failed for `{}`: {error}",
                    dataset.path
                )),
            }
        }

        let host = map_host_readings_from_pools(system, &pools)?;
        let inventory = InventoryReadings {
            host,
            pools,
            datasets,
            warnings,
        };
        self.remember_targets(inventory.sub_targets());
        Ok(inventory)
    }

    async fn list_sub_targets_live(&self) -> Result<Vec<SubTarget>, ConnectorError> {
        let (pools, datasets) = tokio::join!(
            self.client.call(METHOD_POOL_QUERY, json!([])),
            self.client
                .call(METHOD_DATASET_QUERY, dataset_query_params()),
        );
        let pools = map_pool_readings(pools.map_err(connector_error)?).map_err(internal_error)?;
        let datasets =
            map_dataset_readings(datasets.map_err(connector_error)?).map_err(internal_error)?;
        let targets = sub_targets(&pools, &datasets);
        self.remember_targets(targets.clone());
        Ok(targets)
    }

    fn remember_targets(&self, targets: Vec<SubTarget>) {
        *self
            .known_targets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = targets;
    }
}

#[async_trait]
impl loom_core::connector::Connector for TrueNasConnector {
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
        let inventory = match self.read_inventory().await {
            Ok(readings) => readings,
            Err(error) => return Ok(unavailable_status(&error.to_string())),
        };

        let mut details = Value::Object(Map::new());
        for (id, value) in [
            (DATA_POINT_POOL_COUNT, json!(inventory.host.pool_count)),
            (
                DATA_POINT_TOTAL_CAPACITY_BYTES,
                json!(inventory.host.total_capacity_bytes),
            ),
            (
                DATA_POINT_USED_CAPACITY_BYTES,
                json!(inventory.host.used_capacity_bytes),
            ),
            (
                DATA_POINT_FREE_CAPACITY_BYTES,
                json!(inventory.host.free_capacity_bytes),
            ),
            (DATA_POINT_TRUENAS_VERSION, json!(inventory.host.version)),
        ] {
            set_detail(&mut details, None, id, value);
        }

        // Known trade-off: every poll reads every pool and dataset, including a
        // snapshot count per dataset, whether or not a placement displays it.
        // Typical homelabs have a modest inventory; target-aware polling can be
        // revisited if this becomes a demonstrated cost.
        for pool in &inventory.pools {
            let target_id = pool_target_id(&pool.name);
            for (id, value) in [
                (DATA_POINT_STATUS, json!(pool.status)),
                (DATA_POINT_USED_BYTES, json!(pool.used_bytes)),
                (DATA_POINT_FREE_BYTES, json!(pool.free_bytes)),
                (DATA_POINT_CAPACITY_PERCENT, json!(pool.capacity_percent)),
            ] {
                set_detail(&mut details, Some(&target_id), id, value);
            }
        }
        for dataset in &inventory.datasets {
            let target_id = dataset_target_id(&dataset.path);
            for (id, value) in [
                (DATA_POINT_USED_BYTES, json!(dataset.used_bytes)),
                (DATA_POINT_AVAILABLE_BYTES, json!(dataset.available_bytes)),
                (
                    DATA_POINT_COMPRESSION_RATIO,
                    json!(dataset.compression_ratio),
                ),
            ] {
                set_detail(&mut details, Some(&target_id), id, value);
            }
            if let Some(snapshot_count) = dataset.snapshot_count {
                set_detail(
                    &mut details,
                    Some(&target_id),
                    DATA_POINT_SNAPSHOT_COUNT,
                    json!(snapshot_count),
                );
            }
        }
        if !inventory.warnings.is_empty() {
            set_detail(
                &mut details,
                None,
                "error",
                json!(inventory.warnings.join("; ")),
            );
        }

        Ok(ConnectorStatus::new(inventory.health(), details))
    }

    async fn actions(&self) -> Vec<ConnectorAction> {
        let Ok(targets) = self.list_sub_targets_live().await else {
            return Vec::new();
        };
        targets
            .into_iter()
            .filter(|target| target.kind == "pool")
            .map(|target| pool_scrub_action(&target.id))
            .collect()
    }

    async fn execute_action(
        &self,
        action_id: &str,
        target_id: Option<&str>,
        _params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        if action_id != ACTION_START_SCRUB {
            return Err(ConnectorError::invalid_action(action_id));
        }
        let Some(pool) = target_id.and_then(|target| target.strip_prefix("pool:")) else {
            return Err(ConnectorError::invalid_action(action_id));
        };
        if pool.is_empty() {
            return Err(ConnectorError::invalid_action(action_id));
        }

        let job_id = self
            .client
            .start_job(METHOD_POOL_SCRUB, json!([pool, "START"]))
            .await
            .map_err(connector_error)?;
        let result = ActionResult::ok(format!(
            "Scrub started for pool {pool}. TrueNAS is running it asynchronously; this does not mean the scrub has completed."
        ));
        Ok(match job_id {
            Some(job_id) => result.with_payload(json!({ "jobId": job_id })),
            None => result,
        })
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
        let targets = self
            .known_targets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        host_data_points()
            .into_iter()
            .chain(
                targets
                    .into_iter()
                    .flat_map(|target| match target.kind.as_str() {
                        "pool" => pool_data_points(&target.id),
                        "dataset" => dataset_data_points(&target.id),
                        _ => Vec::new(),
                    }),
            )
            .collect()
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

    fn default_layout_for(&self, target_id: Option<&str>) -> WidgetLayout {
        match target_id {
            Some(target) if target.starts_with("pool:") => WidgetLayout::new(vec![
                WidgetBinding::display(DATA_POINT_STATUS, DisplayWidgetType::StatusDot),
                WidgetBinding::display(DATA_POINT_CAPACITY_PERCENT, DisplayWidgetType::ProgressBar)
                    .with_config(json!({ "min": 0, "max": 100 })),
                WidgetBinding::action(ACTION_START_SCRUB, ActionWidgetType::Button),
            ]),
            Some(target) if target.starts_with("dataset:") => WidgetLayout::new(vec![
                WidgetBinding::display(DATA_POINT_USED_BYTES, DisplayWidgetType::StatTile),
                WidgetBinding::display(DATA_POINT_COMPRESSION_RATIO, DisplayWidgetType::StatTile),
                WidgetBinding::display(DATA_POINT_SNAPSHOT_COUNT, DisplayWidgetType::StatTile),
            ]),
            _ => self.default_layout(),
        }
    }

    fn network_target(&self) -> Option<NetworkTarget> {
        Some(NetworkTarget::new(
            self.config.host.trim_matches(['[', ']']),
            443,
        ))
    }
}

fn host_data_points() -> Vec<DataPointDescriptor> {
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

fn pool_data_points(target_id: &str) -> Vec<DataPointDescriptor> {
    // `pool.query.scan` is explicitly the active scrub/resilver operation and
    // is null when none is running. It is not a reliable last-scrub record, so
    // this pass deliberately does not publish a misleading `lastScrubStatus`.
    vec![
        DataPointDescriptor::new(DATA_POINT_STATUS, "Status", DataPointValueType::String)
            .for_target(target_id),
        DataPointDescriptor::new(DATA_POINT_USED_BYTES, "Used", DataPointValueType::Number)
            .with_unit("bytes")
            .for_target(target_id),
        DataPointDescriptor::new(DATA_POINT_FREE_BYTES, "Free", DataPointValueType::Number)
            .with_unit("bytes")
            .for_target(target_id),
        DataPointDescriptor::new(
            DATA_POINT_CAPACITY_PERCENT,
            "Capacity",
            DataPointValueType::Number,
        )
        .with_unit("%")
        .for_target(target_id),
    ]
}

fn dataset_data_points(target_id: &str) -> Vec<DataPointDescriptor> {
    vec![
        DataPointDescriptor::new(DATA_POINT_USED_BYTES, "Used", DataPointValueType::Number)
            .with_unit("bytes")
            .for_target(target_id),
        DataPointDescriptor::new(
            DATA_POINT_AVAILABLE_BYTES,
            "Available",
            DataPointValueType::Number,
        )
        .with_unit("bytes")
        .for_target(target_id),
        DataPointDescriptor::new(
            DATA_POINT_COMPRESSION_RATIO,
            "Compression ratio",
            DataPointValueType::Number,
        )
        .for_target(target_id),
        DataPointDescriptor::new(
            DATA_POINT_SNAPSHOT_COUNT,
            "Snapshots",
            DataPointValueType::Number,
        )
        .for_target(target_id),
    ]
}

fn pool_scrub_action(target_id: &str) -> ConnectorAction {
    ConnectorAction::simple(ACTION_START_SCRUB, "Start scrub")
        .for_target(target_id)
        .with_description(
            "Starts a background pool scrub. This can substantially increase storage I/O while it runs.",
        )
        .disruptive()
}

#[derive(Debug, PartialEq, Eq)]
struct HostReadings {
    version: String,
    pool_count: u64,
    total_capacity_bytes: u64,
    used_capacity_bytes: u64,
    free_capacity_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct PoolReadings {
    name: String,
    status: String,
    size_bytes: u64,
    used_bytes: u64,
    free_bytes: u64,
    capacity_percent: f64,
}

#[derive(Debug, Clone, PartialEq)]
struct DatasetReadings {
    path: String,
    used_bytes: u64,
    available_bytes: u64,
    compression_ratio: f64,
    snapshot_count: Option<u64>,
}

#[derive(Debug, PartialEq)]
struct InventoryReadings {
    host: HostReadings,
    pools: Vec<PoolReadings>,
    datasets: Vec<DatasetReadings>,
    warnings: Vec<String>,
}

impl InventoryReadings {
    fn sub_targets(&self) -> Vec<SubTarget> {
        sub_targets(&self.pools, &self.datasets)
    }

    fn health(&self) -> HealthState {
        let initial = if self.warnings.is_empty() {
            HealthState::Healthy
        } else {
            HealthState::Degraded
        };
        self.pools.iter().fold(initial, |overall, pool| {
            worse_health(overall, pool_health(&pool.status))
        })
    }
}

#[cfg(test)]
fn map_host_readings(system: Value, pools: Value) -> Result<HostReadings, String> {
    let pools = map_pool_readings(pools)?;
    map_host_readings_from_pools(system, &pools)
}

fn map_host_readings_from_pools(
    system: Value,
    pools: &[PoolReadings],
) -> Result<HostReadings, String> {
    let version = required_string(&system, "version", METHOD_SYSTEM_INFO)?.to_owned();
    // Hostname is a required system.info field in the stable schema. Validate
    // it even though this minimal dashboard does not yet publish it as a data
    // point, so a response from the wrong method shape cannot look healthy.
    required_string(&system, "hostname", METHOD_SYSTEM_INFO)?;

    let mut total_capacity_bytes = 0_u64;
    let mut used_capacity_bytes = 0_u64;
    let mut free_capacity_bytes = 0_u64;
    for pool in pools {
        total_capacity_bytes = checked_capacity_sum(total_capacity_bytes, pool.size_bytes)?;
        used_capacity_bytes = checked_capacity_sum(used_capacity_bytes, pool.used_bytes)?;
        free_capacity_bytes = checked_capacity_sum(free_capacity_bytes, pool.free_bytes)?;
    }

    Ok(HostReadings {
        version,
        pool_count: pools.len() as u64,
        total_capacity_bytes,
        used_capacity_bytes,
        free_capacity_bytes,
    })
}

fn map_pool_readings(value: Value) -> Result<Vec<PoolReadings>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{METHOD_POOL_QUERY} did not return an array"))?
        .iter()
        .map(|pool| {
            let name = required_string(pool, "name", METHOD_POOL_QUERY)?.to_owned();
            let status = required_string(pool, "status", METHOD_POOL_QUERY)?.to_owned();
            let size = optional_bytes(pool, "size", METHOD_POOL_QUERY)?;
            let used_bytes = optional_bytes(pool, "allocated", METHOD_POOL_QUERY)?;
            let free_bytes = optional_bytes(pool, "free", METHOD_POOL_QUERY)?;
            let capacity_percent = if size == 0 {
                0.0
            } else {
                used_bytes as f64 / size as f64 * 100.0
            };
            Ok(PoolReadings {
                name,
                status,
                size_bytes: size,
                used_bytes,
                free_bytes,
                capacity_percent,
            })
        })
        .collect()
}

fn map_dataset_readings(value: Value) -> Result<Vec<DatasetReadings>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{METHOD_DATASET_QUERY} did not return an array"))?
        .iter()
        .map(|dataset| {
            Ok(DatasetReadings {
                path: required_string(dataset, "id", METHOD_DATASET_QUERY)?.to_owned(),
                used_bytes: dataset_property_u64(dataset, "used")?,
                available_bytes: dataset_property_u64(dataset, "available")?,
                compression_ratio: dataset_property_number(dataset, "compressratio")?,
                snapshot_count: None,
            })
        })
        .collect()
}

fn dataset_property_u64(dataset: &Value, key: &str) -> Result<u64, String> {
    let property = dataset.get(key).ok_or_else(|| {
        format!("{METHOD_DATASET_QUERY} returned no `{key}` property for a dataset")
    })?;
    property
        .get("parsed")
        .and_then(Value::as_u64)
        .or_else(|| {
            property
                .get("rawvalue")
                .and_then(Value::as_str)
                .and_then(|value| value.parse().ok())
        })
        .ok_or_else(|| {
            format!("{METHOD_DATASET_QUERY} returned no numeric `{key}` property for a dataset")
        })
}

fn dataset_property_number(dataset: &Value, key: &str) -> Result<f64, String> {
    let property = dataset.get(key).ok_or_else(|| {
        format!("{METHOD_DATASET_QUERY} returned no `{key}` property for a dataset")
    })?;
    ["parsed", "rawvalue", "value"]
        .into_iter()
        .find_map(|field| {
            let value = property.get(field)?;
            value.as_f64().or_else(|| {
                value
                    .as_str()?
                    .trim()
                    .trim_end_matches(['x', 'X'])
                    .parse()
                    .ok()
            })
        })
        .ok_or_else(|| {
            format!("{METHOD_DATASET_QUERY} returned no numeric `{key}` property for a dataset")
        })
}

fn dataset_query_params() -> Value {
    json!([[], {
        "extra": {
            "flat": true,
            "retrieve_user_props": false,
            "properties": ["used", "available", "compressratio"]
        }
    }])
}

fn sub_targets(pools: &[PoolReadings], datasets: &[DatasetReadings]) -> Vec<SubTarget> {
    pools
        .iter()
        .map(|pool| SubTarget::new(pool_target_id(&pool.name), &pool.name).of_kind("pool"))
        .chain(datasets.iter().map(|dataset| {
            SubTarget::new(dataset_target_id(&dataset.path), &dataset.path).of_kind("dataset")
        }))
        .collect()
}

fn pool_target_id(name: &str) -> String {
    format!("pool:{name}")
}

fn dataset_target_id(path: &str) -> String {
    format!("dataset:{path}")
}

fn pool_health(status: &str) -> HealthState {
    match status.to_ascii_uppercase().as_str() {
        "ONLINE" => HealthState::Healthy,
        "DEGRADED" => HealthState::Degraded,
        "FAULTED" | "OFFLINE" | "UNAVAIL" | "REMOVED" => HealthState::Down,
        _ => HealthState::Unknown,
    }
}

fn worse_health(left: HealthState, right: HealthState) -> HealthState {
    fn severity(health: HealthState) -> u8 {
        match health {
            HealthState::Healthy => 0,
            HealthState::Unknown => 1,
            HealthState::Degraded => 2,
            HealthState::Down => 3,
        }
    }
    if severity(right) > severity(left) {
        right
    } else {
        left
    }
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
        TrueNasError::RpcError { code, message } => {
            ConnectorError::Internal(format!("TrueNAS returned RPC error {code}: {message}"))
        }
    }
}

fn internal_error(error: String) -> ConnectorError {
    ConnectorError::Internal(error)
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
                { "name": "tank", "status": "ONLINE", "size": 1000, "allocated": 400, "free": 600 },
                { "name": "backup", "status": "ONLINE", "size": 2000, "allocated": 750, "free": 1250 }
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
            json!([{ "name": "tank", "status": "ONLINE", "size": null, "allocated": null, "free": null }]),
        )
        .expect("nullable fields are documented");
        assert_eq!(readings.pool_count, 1);
        assert_eq!(readings.total_capacity_bytes, 0);
    }

    #[test]
    fn host_descriptors_and_layout_are_consistent() {
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
    fn documented_pool_fields_map_capacity_and_health() {
        let pools = map_pool_readings(json!([{
            "name": "tank",
            "status": "DEGRADED",
            "size": 1000,
            "allocated": 375,
            "free": 625,
            "scan": null
        }]))
        .expect("documented pool.query shape");

        assert_eq!(pools.len(), 1);
        assert_eq!(pools[0].name, "tank");
        assert_eq!(pools[0].used_bytes, 375);
        assert_eq!(pools[0].free_bytes, 625);
        assert_eq!(pools[0].capacity_percent, 37.5);
        assert_eq!(pool_health(&pools[0].status), HealthState::Degraded);
    }

    #[test]
    fn pool_health_states_follow_the_platform_convention() {
        assert_eq!(pool_health("ONLINE"), HealthState::Healthy);
        assert_eq!(pool_health("degraded"), HealthState::Degraded);
        assert_eq!(pool_health("FAULTED"), HealthState::Down);
        assert_eq!(pool_health("OFFLINE"), HealthState::Down);
        assert_eq!(pool_health("future-state"), HealthState::Unknown);
    }

    #[test]
    fn documented_dataset_properties_map_from_parsed_or_raw_values() {
        let datasets = map_dataset_readings(json!([{
            "id": "tank/apps",
            "used": { "parsed": 4096, "rawvalue": "4096", "value": "4 KiB" },
            "available": { "parsed": 8192, "rawvalue": "8192", "value": "8 KiB" },
            "compressratio": { "parsed": "1.25x", "rawvalue": "1.25", "value": "1.25x" }
        }]))
        .expect("documented pool.dataset.query shape");

        assert_eq!(datasets.len(), 1);
        assert_eq!(datasets[0].path, "tank/apps");
        assert_eq!(datasets[0].used_bytes, 4096);
        assert_eq!(datasets[0].available_bytes, 8192);
        assert_eq!(datasets[0].compression_ratio, 1.25);
    }

    #[test]
    fn pool_and_dataset_targets_use_stable_prefixed_ids() {
        let pools = vec![PoolReadings {
            name: "tank".to_owned(),
            status: "ONLINE".to_owned(),
            size_bytes: 100,
            used_bytes: 25,
            free_bytes: 75,
            capacity_percent: 25.0,
        }];
        let datasets = vec![DatasetReadings {
            path: "tank/media/movies".to_owned(),
            used_bytes: 10,
            available_bytes: 90,
            compression_ratio: 1.1,
            snapshot_count: Some(2),
        }];

        assert_eq!(
            sub_targets(&pools, &datasets),
            vec![
                SubTarget::new("pool:tank", "tank").of_kind("pool"),
                SubTarget::new("dataset:tank/media/movies", "tank/media/movies").of_kind("dataset"),
            ]
        );
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

    #[test]
    fn optional_dataset_failures_degrade_but_do_not_mark_the_host_down() {
        let inventory = InventoryReadings {
            host: HostReadings {
                version: "25.10".to_owned(),
                pool_count: 1,
                total_capacity_bytes: 100,
                used_capacity_bytes: 25,
                free_capacity_bytes: 75,
            },
            pools: vec![PoolReadings {
                name: "tank".to_owned(),
                status: "ONLINE".to_owned(),
                size_bytes: 100,
                used_bytes: 25,
                free_bytes: 75,
                capacity_percent: 25.0,
            }],
            datasets: Vec::new(),
            warnings: vec!["pool.dataset.query failed: permission denied".to_owned()],
        };

        assert_eq!(inventory.health(), HealthState::Degraded);
    }

    #[test]
    fn a_failed_pool_still_marks_the_connector_down_when_telemetry_is_partial() {
        let inventory = InventoryReadings {
            host: HostReadings {
                version: "25.10".to_owned(),
                pool_count: 1,
                total_capacity_bytes: 100,
                used_capacity_bytes: 25,
                free_capacity_bytes: 75,
            },
            pools: vec![PoolReadings {
                name: "tank".to_owned(),
                status: "FAULTED".to_owned(),
                size_bytes: 100,
                used_bytes: 25,
                free_bytes: 75,
                capacity_percent: 25.0,
            }],
            datasets: Vec::new(),
            warnings: vec!["snapshot count unavailable".to_owned()],
        };

        assert_eq!(inventory.health(), HealthState::Down);
    }
}
