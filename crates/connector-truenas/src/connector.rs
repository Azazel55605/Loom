//! TrueNAS host, pool, and dataset connector surface.

use std::{collections::HashMap, sync::Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use futures_util::future::join_all;
use loom_core::connector::{
    details::set_detail, ActionResult, ActionWidgetType, ApplicableTarget, ChartType,
    ColumnDescriptor, ColumnValueType, ConnectorAction, ConnectorError, ConnectorMetadata,
    ConnectorStatus, DataPointDescriptor, DataPointValueType, DisplayField, DisplayWidgetType,
    HealthState, NetworkTarget, ResourceItem, ResourceKindDescriptor, SubTarget, WidgetBinding,
    WidgetLayout,
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
pub const DATA_POINT_POOL_STORAGE_BREAKDOWN: &str = "poolStorageBreakdown";
pub const DATA_POINT_ACTIVE_ALERT_COUNT: &str = "activeAlertCount";
pub const DATA_POINT_SYSTEM_UPTIME: &str = "systemUptime";
pub const DATA_POINT_STATUS: &str = "status";
pub const DATA_POINT_USED_BYTES: &str = "usedBytes";
pub const DATA_POINT_FREE_BYTES: &str = "freeBytes";
pub const DATA_POINT_CAPACITY_PERCENT: &str = "capacityPercent";
pub const DATA_POINT_AVAILABLE_BYTES: &str = "availableBytes";
pub const DATA_POINT_COMPRESSION_RATIO: &str = "compressionRatio";
pub const DATA_POINT_SNAPSHOT_COUNT: &str = "snapshotCount";
pub const ACTION_START_SCRUB: &str = "startScrub";
pub const ACTION_CREATE_SNAPSHOT: &str = "createSnapshot";
pub const ACTION_ROLLBACK_SNAPSHOT: &str = "rollback";
pub const ACTION_DELETE_SNAPSHOT: &str = "delete";
pub const ACTION_DISMISS_ALERT: &str = "dismiss";

pub const RESOURCE_KIND_POOLS: &str = "pools";
pub const RESOURCE_KIND_DATASETS: &str = "datasets";
pub const RESOURCE_KIND_SNAPSHOTS: &str = "snapshots";
pub const RESOURCE_KIND_ALERTS: &str = "alerts";

const METHOD_SYSTEM_INFO: &str = "system.info";
const METHOD_POOL_QUERY: &str = "pool.query";
const METHOD_DATASET_QUERY: &str = "pool.dataset.query";
const METHOD_SNAPSHOT_COUNT: &str = "pool.dataset.snapshot_count";
const METHOD_POOL_SCRUB: &str = "pool.scrub.scrub";
const METHOD_SNAPSHOT_QUERY: &str = "pool.snapshot.query";
const METHOD_SNAPSHOT_CREATE: &str = "pool.snapshot.create";
const METHOD_SNAPSHOT_DELETE: &str = "pool.snapshot.delete";
const METHOD_SNAPSHOT_ROLLBACK: &str = "pool.snapshot.rollback";
const METHOD_ALERT_LIST: &str = "alert.list";
const METHOD_ALERT_DISMISS: &str = "alert.dismiss";
const RESOURCE_ID_PARAM: &str = "resourceId";
const DEFAULT_SNAPSHOT_NAMING_SCHEMA: &str = "loom-%Y-%m-%d_%H-%M-%S";

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
        let (system, storage, alerts) = tokio::join!(
            self.client.call(METHOD_SYSTEM_INFO, json!([])),
            self.query_storage_inventory(),
            self.client.call(METHOD_ALERT_LIST, json!([])),
        );
        // Host identity and pool health are the connector's availability
        // boundary. Dataset telemetry enriches that reading, but an API key
        // without DATASET_READ (or one inaccessible dataset) must not make an
        // otherwise reachable TrueNAS host look offline.
        let system = system.map_err(|error| error.to_string())?;
        let pools = storage.pools?;
        let mut warnings = Vec::new();
        let mut datasets = match storage.datasets {
            Ok(datasets) => datasets,
            Err(error) => {
                warnings.push(error);
                Vec::new()
            }
        };
        let active_alert_count = match alerts {
            Ok(value) => match map_active_alert_count(value) {
                Ok(count) => Some(count),
                Err(error) => {
                    warnings.push(error);
                    None
                }
            },
            Err(error) => {
                warnings.push(format!("{METHOD_ALERT_LIST} failed: {error}"));
                None
            }
        };

        warnings.extend(self.populate_snapshot_counts(&mut datasets).await);

        let host = map_host_readings_from_pools(system, &pools)?;
        let inventory = InventoryReadings {
            host,
            pools,
            datasets,
            active_alert_count,
            warnings,
        };
        self.remember_targets(inventory.sub_targets());
        Ok(inventory)
    }

    async fn list_sub_targets_live(&self) -> Result<Vec<SubTarget>, ConnectorError> {
        let storage = self.query_storage_inventory().await;
        let pools = storage.pools.map_err(internal_error)?;
        let datasets = storage.datasets.map_err(internal_error)?;
        let targets = sub_targets(&pools, &datasets);
        self.remember_targets(targets.clone());
        Ok(targets)
    }

    /// Reads pools and datasets once for every consumer of storage inventory.
    ///
    /// Keeping the concurrent calls and response mapping here prevents status,
    /// sub-target discovery, and resource browsing from growing subtly
    /// different query shapes (or one of them regressing into per-item calls).
    async fn query_storage_inventory(&self) -> StorageInventoryResults {
        let (pools, datasets) = tokio::join!(
            self.client.call(METHOD_POOL_QUERY, json!([])),
            self.client
                .call(METHOD_DATASET_QUERY, dataset_query_params()),
        );
        StorageInventoryResults {
            pools: pools
                .map_err(|error| format!("{METHOD_POOL_QUERY} failed: {error}"))
                .and_then(map_pool_readings),
            datasets: datasets
                .map_err(|error| format!("{METHOD_DATASET_QUERY} failed: {error}"))
                .and_then(map_dataset_readings),
        }
    }

    async fn populate_snapshot_counts(&self, datasets: &mut [DatasetReadings]) -> Vec<String> {
        let counts = join_all(datasets.iter().map(|dataset| {
            self.client
                .call(METHOD_SNAPSHOT_COUNT, json!([dataset.path.as_str()]))
        }))
        .await;
        let mut warnings = Vec::new();
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
        warnings
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
            (DATA_POINT_SYSTEM_UPTIME, json!(inventory.host.uptime)),
            (
                DATA_POINT_POOL_STORAGE_BREAKDOWN,
                Value::Array(
                    inventory
                        .pools
                        .iter()
                        .map(|pool| json!({ "label": pool.name, "value": pool.used_bytes }))
                        .collect(),
                ),
            ),
        ] {
            set_detail(&mut details, None, id, value);
        }
        set_detail(
            &mut details,
            None,
            DATA_POINT_ACTIVE_ALERT_COUNT,
            inventory
                .active_alert_count
                .map_or(Value::Null, Value::from),
        );

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

        let health = inventory.health();
        let mut target_health = HashMap::from([(String::new(), health)]);
        for pool in &inventory.pools {
            target_health.insert(pool_target_id(&pool.name), pool_health(&pool.status));
        }
        for dataset in &inventory.datasets {
            target_health.insert(
                dataset_target_id(&dataset.path),
                if dataset.snapshot_count.is_some() {
                    HealthState::Healthy
                } else {
                    HealthState::Degraded
                },
            );
        }

        let mut status = ConnectorStatus::new(health, details);
        status.target_health = target_health;
        Ok(status)
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
        params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        match action_id {
            ACTION_START_SCRUB => {
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
            ACTION_CREATE_SNAPSHOT => {
                let dataset = required_dataset_target(target_id, action_id)?;
                let recursive = params
                    .get("recursive")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let name = params
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty());
                let data = match name {
                    Some(name) => json!({
                        "dataset": dataset,
                        "name": name,
                        "recursive": recursive
                    }),
                    None => json!({
                        "dataset": dataset,
                        "naming_schema": DEFAULT_SNAPSHOT_NAMING_SCHEMA,
                        "recursive": recursive
                    }),
                };
                let created = self
                    .client
                    .call(METHOD_SNAPSHOT_CREATE, json!([data]))
                    .await
                    .map_err(connector_error)?;
                let snapshot_id = required_string(&created, "id", METHOD_SNAPSHOT_CREATE)
                    .map_err(internal_error)?;
                Ok(ActionResult::ok(format!("Created snapshot {snapshot_id}."))
                    .with_payload(json!({ "snapshotId": snapshot_id })))
            }
            ACTION_DELETE_SNAPSHOT | ACTION_ROLLBACK_SNAPSHOT => {
                let dataset = required_dataset_target(target_id, action_id)?;
                let snapshot_id = required_resource_id(action_id, &params)?;
                if !snapshot_belongs_to_dataset(snapshot_id, dataset) {
                    return Err(ConnectorError::InvalidParams {
                        action_id: action_id.to_owned(),
                        reason: format!(
                            "snapshot `{snapshot_id}` does not belong to dataset `{dataset}`"
                        ),
                    });
                }
                if action_id == ACTION_DELETE_SNAPSHOT {
                    self.client
                        .call(
                            METHOD_SNAPSHOT_DELETE,
                            json!([snapshot_id, { "defer": false, "recursive": false }]),
                        )
                        .await
                        .map_err(connector_error)?;
                    Ok(ActionResult::ok(format!("Deleted snapshot {snapshot_id}.")))
                } else {
                    self.client
                        .call(
                            METHOD_SNAPSHOT_ROLLBACK,
                            json!([snapshot_id, {
                                "recursive": false,
                                "recursive_clones": false,
                                "force": false,
                                "recursive_rollback": false
                            }]),
                        )
                        .await
                        .map_err(connector_error)?;
                    Ok(ActionResult::ok(format!(
                        "Rolled dataset {dataset} back to snapshot {snapshot_id}."
                    )))
                }
            }
            ACTION_DISMISS_ALERT => {
                if target_id.is_some() {
                    return Err(ConnectorError::invalid_action(action_id));
                }
                let alert_id = required_resource_id(action_id, &params)?;
                self.client
                    .call(METHOD_ALERT_DISMISS, json!([alert_id]))
                    .await
                    .map_err(connector_error)?;
                Ok(ActionResult::ok("Dismissed the alert."))
            }
            _ => Err(ConnectorError::invalid_action(action_id)),
        }
    }

    fn resource_kinds(&self, target_id: Option<&str>) -> Vec<ResourceKindDescriptor> {
        match target_id {
            None => host_resource_kinds(),
            Some(target) if target.starts_with("dataset:") => {
                vec![snapshots_kind(target)]
            }
            _ => Vec::new(),
        }
    }

    async fn list_resource_items(
        &self,
        kind: &str,
        target_id: Option<&str>,
    ) -> Result<Vec<ResourceItem>, ConnectorError> {
        match (kind, target_id) {
            (RESOURCE_KIND_POOLS, None) => {
                let storage = self.query_storage_inventory().await;
                Ok(pool_resource_items(storage.pools.map_err(internal_error)?))
            }
            (RESOURCE_KIND_DATASETS, None) => {
                let storage = self.query_storage_inventory().await;
                let mut datasets = storage.datasets.map_err(internal_error)?;
                let warnings = self.populate_snapshot_counts(&mut datasets).await;
                if let Some(warning) = warnings.first() {
                    return Err(ConnectorError::Internal(warning.clone()));
                }
                Ok(dataset_resource_items(datasets))
            }
            (RESOURCE_KIND_ALERTS, None) => {
                let alerts = self
                    .client
                    .call(METHOD_ALERT_LIST, json!([]))
                    .await
                    .map_err(connector_error)?;
                map_alert_resource_items(alerts).map_err(internal_error)
            }
            (RESOURCE_KIND_SNAPSHOTS, Some(target)) => {
                let Some(dataset) = target.strip_prefix("dataset:") else {
                    return Ok(Vec::new());
                };
                if dataset.is_empty() {
                    return Ok(Vec::new());
                }
                let snapshots = self
                    .client
                    .call(METHOD_SNAPSHOT_QUERY, snapshot_query_params(dataset))
                    .await
                    .map_err(connector_error)?;
                map_snapshot_resource_items(snapshots).map_err(internal_error)
            }
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
        // `system.info` is already part of the cheap host poll and supplies
        // uptime. Its `physmem` is total installed RAM, not usage, and
        // `loadavg` is scheduler load rather than CPU percentage. True CPU and
        // memory utilization require separate reporting calls, so neither is
        // mislabeled as `cpuPercent`/`memoryUsageBytes` here.
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
            WidgetBinding::display(DATA_POINT_ACTIVE_ALERT_COUNT, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_SYSTEM_UPTIME, DisplayWidgetType::StatTile),
            WidgetBinding::display(
                DATA_POINT_POOL_STORAGE_BREAKDOWN,
                DisplayWidgetType::MetricChart {
                    chart_type: ChartType::Bar,
                },
            ),
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
                WidgetBinding::display(DATA_POINT_STATUS, DisplayWidgetType::StatusDot)
                    .with_config(json!({
                        "colorMap": {
                            "ONLINE": "healthy",
                            "DEGRADED": "degraded",
                            "FAULTED": "down",
                            "OFFLINE": "down",
                            "UNAVAIL": "down",
                            "REMOVED": "down"
                        }
                    })),
                WidgetBinding::display(DATA_POINT_USED_BYTES, DisplayWidgetType::StatTile),
                WidgetBinding::display(DATA_POINT_FREE_BYTES, DisplayWidgetType::StatTile),
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
        DataPointDescriptor::new(
            DATA_POINT_POOL_STORAGE_BREAKDOWN,
            "Used storage by pool",
            DataPointValueType::CategoryBreakdown,
        )
        .with_unit("bytes"),
        DataPointDescriptor::new(
            DATA_POINT_ACTIVE_ALERT_COUNT,
            "Active alerts",
            DataPointValueType::Number,
        ),
        DataPointDescriptor::new(
            DATA_POINT_SYSTEM_UPTIME,
            "System uptime",
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

fn host_resource_kinds() -> Vec<ResourceKindDescriptor> {
    // `startScrub` deliberately does not appear as a Pools-table row action.
    // Scrub already uses the direct sub-target convention (`target_id` is
    // `pool:{name}`), while resource row actions address a row with
    // `params.resourceId` and ordinarily have no target. Accepting both shapes
    // would give `execute_action` two conventions for identical behavior.
    // Keeping Pools browse-only leaves scrub in the pool detail view, where
    // its existing target-scoped contract remains unambiguous.
    vec![
        ResourceKindDescriptor::new(
            RESOURCE_KIND_POOLS,
            "Pools",
            vec![
                ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
                ColumnDescriptor::new("status", "Status", ColumnValueType::Text),
                ColumnDescriptor::new("usedBytes", "Used", ColumnValueType::Bytes),
                ColumnDescriptor::new("freeBytes", "Free", ColumnValueType::Bytes),
            ],
        )
        .applicable_to(ApplicableTarget::HostOnly),
        ResourceKindDescriptor::new(
            RESOURCE_KIND_DATASETS,
            "Datasets",
            vec![
                ColumnDescriptor::new("path", "Path", ColumnValueType::Text),
                ColumnDescriptor::new("pool", "Pool", ColumnValueType::Text),
                ColumnDescriptor::new("usedBytes", "Used", ColumnValueType::Bytes),
                ColumnDescriptor::new("availableBytes", "Available", ColumnValueType::Bytes),
                ColumnDescriptor::new("compressionRatio", "Compression", ColumnValueType::Number),
                ColumnDescriptor::new("snapshotCount", "Snapshots", ColumnValueType::Number),
            ],
        )
        .applicable_to(ApplicableTarget::HostOnly),
        alerts_kind(),
    ]
}

fn snapshots_kind(target_id: &str) -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_SNAPSHOTS,
        "Snapshots",
        vec![
            ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
            ColumnDescriptor::new("created", "Created", ColumnValueType::Timestamp),
            ColumnDescriptor::new("usedBytes", "Used", ColumnValueType::Bytes),
        ],
    )
    .applicable_to(ApplicableTarget::TargetOnly)
    .with_row_actions(vec![
        resource_row_action(
            ACTION_ROLLBACK_SNAPSHOT,
            "Rollback",
            "Revert this dataset to the snapshot. Data written after it was created will be lost.",
            true,
        ),
        resource_row_action(
            ACTION_DELETE_SNAPSHOT,
            "Delete",
            "Permanently delete this snapshot.",
            false,
        ),
    ])
    .with_kind_actions(vec![ConnectorAction {
        id: ACTION_CREATE_SNAPSHOT.to_owned(),
        target_id: Some(target_id.to_owned()),
        label: "Create snapshot".to_owned(),
        description: Some(
            "Create a snapshot now. Leave the name empty to use Loom's timestamp-based name."
                .to_owned(),
        ),
        params_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "title": "Name",
                    "description": "Optional snapshot name. If blank, Loom uses a timestamp-based name."
                },
                "recursive": {
                    "type": "boolean",
                    "title": "Recursive",
                    "description": "Also snapshot child datasets.",
                    "default": false
                }
            },
            "additionalProperties": false
        }),
        is_disruptive: false,
        snapshot_data_point_ids: Vec::new(),
    }])
}

fn alerts_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_ALERTS,
        "Alerts",
        vec![
            ColumnDescriptor::new("level", "Level", ColumnValueType::Text),
            ColumnDescriptor::new("message", "Message", ColumnValueType::Text),
            ColumnDescriptor::new("created", "Created", ColumnValueType::Timestamp),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    .with_row_actions(vec![resource_row_action(
        ACTION_DISMISS_ALERT,
        "Dismiss",
        "Dismiss this active alert in TrueNAS.",
        false,
    )])
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
                RESOURCE_ID_PARAM: {
                    "type": "string",
                    "title": "Resource",
                    "description": "The row this action applies to."
                }
            },
            "required": [RESOURCE_ID_PARAM],
            "additionalProperties": false
        }),
        is_disruptive,
        snapshot_data_point_ids: Vec::new(),
    }
}

struct StorageInventoryResults {
    pools: Result<Vec<PoolReadings>, String>,
    datasets: Result<Vec<DatasetReadings>, String>,
}

#[derive(Debug, PartialEq, Eq)]
struct HostReadings {
    version: String,
    uptime: String,
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
    active_alert_count: Option<u64>,
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
    let uptime = required_string(&system, "uptime", METHOD_SYSTEM_INFO)?.to_owned();
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
        uptime,
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

fn snapshot_query_params(dataset: &str) -> Value {
    json!([
        [["dataset", "=", dataset]],
        {
            "extra": { "properties": ["creation", "used"] },
            "order_by": ["-createtxg"]
        }
    ])
}

fn pool_resource_items(pools: Vec<PoolReadings>) -> Vec<ResourceItem> {
    pools
        .into_iter()
        .map(|pool| {
            ResourceItem::new(pool.name.clone())
                .with_field("targetId", pool_target_id(&pool.name))
                .with_field("name", pool.name)
                .with_field("status", pool.status)
                .with_field("usedBytes", pool.used_bytes)
                .with_field("freeBytes", pool.free_bytes)
        })
        .collect()
}

fn dataset_resource_items(datasets: Vec<DatasetReadings>) -> Vec<ResourceItem> {
    datasets
        .into_iter()
        .map(|dataset| {
            let pool = dataset
                .path
                .split('/')
                .next()
                .unwrap_or_default()
                .to_owned();
            let mut item = ResourceItem::new(dataset.path.clone())
                .with_field("targetId", dataset_target_id(&dataset.path))
                .with_field("path", dataset.path)
                .with_field("pool", pool)
                .with_field("usedBytes", dataset.used_bytes)
                .with_field("availableBytes", dataset.available_bytes)
                .with_field("compressionRatio", dataset.compression_ratio);
            if let Some(snapshot_count) = dataset.snapshot_count {
                item = item.with_field("snapshotCount", snapshot_count);
            }
            item
        })
        .collect()
}

fn map_snapshot_resource_items(value: Value) -> Result<Vec<ResourceItem>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{METHOD_SNAPSHOT_QUERY} did not return an array"))?
        .iter()
        .map(|snapshot| {
            let id = required_string(snapshot, "id", METHOD_SNAPSHOT_QUERY)?.to_owned();
            let name = required_string(snapshot, "snapshot_name", METHOD_SNAPSHOT_QUERY)?;
            let properties = snapshot.get("properties").ok_or_else(|| {
                format!("{METHOD_SNAPSHOT_QUERY} returned no `properties` object")
            })?;
            let creation = snapshot_property_u64(properties, "creation")?;
            let created = DateTime::<Utc>::from_timestamp(
                i64::try_from(creation).map_err(|_| {
                    format!("{METHOD_SNAPSHOT_QUERY} returned an out-of-range creation timestamp")
                })?,
                0,
            )
            .ok_or_else(|| {
                format!("{METHOD_SNAPSHOT_QUERY} returned an invalid creation timestamp")
            })?
            .to_rfc3339();
            let used_bytes = snapshot_property_u64(properties, "used")?;
            Ok(ResourceItem::new(id)
                .with_field("name", name)
                .with_field("created", created)
                .with_field("usedBytes", used_bytes))
        })
        .collect()
}

fn snapshot_property_u64(properties: &Value, key: &str) -> Result<u64, String> {
    let property = properties
        .get(key)
        .ok_or_else(|| format!("{METHOD_SNAPSHOT_QUERY} returned no `{key}` snapshot property"))?;
    property
        .get("parsed")
        .and_then(value_as_u64)
        .or_else(|| property.get("rawvalue").and_then(value_as_u64))
        .ok_or_else(|| {
            format!("{METHOD_SNAPSHOT_QUERY} returned no numeric `{key}` snapshot property")
        })
}

fn value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|value| value.parse().ok()))
}

fn map_alert_resource_items(value: Value) -> Result<Vec<ResourceItem>, String> {
    value
        .as_array()
        .ok_or_else(|| format!("{METHOD_ALERT_LIST} did not return an array"))?
        .iter()
        .filter(|alert| alert.get("dismissed").and_then(Value::as_bool) == Some(false))
        .map(|alert| {
            let uuid = required_string(alert, "uuid", METHOD_ALERT_LIST)?;
            Ok(ResourceItem::new(uuid)
                .with_field("level", required_string(alert, "level", METHOD_ALERT_LIST)?)
                .with_field(
                    "message",
                    required_string(alert, "text", METHOD_ALERT_LIST)?,
                )
                .with_field(
                    "created",
                    required_string(alert, "datetime", METHOD_ALERT_LIST)?,
                ))
        })
        .collect()
}

fn map_active_alert_count(value: Value) -> Result<u64, String> {
    let alerts = value
        .as_array()
        .ok_or_else(|| format!("{METHOD_ALERT_LIST} did not return an array"))?;
    let active = alerts
        .iter()
        .filter(|alert| alert.get("dismissed").and_then(Value::as_bool) == Some(false))
        .count();
    u64::try_from(active).map_err(|_| "TrueNAS alert count exceeded the supported range".to_owned())
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

fn required_dataset_target<'a>(
    target_id: Option<&'a str>,
    action_id: &str,
) -> Result<&'a str, ConnectorError> {
    let Some(dataset) = target_id.and_then(|target| target.strip_prefix("dataset:")) else {
        return Err(ConnectorError::invalid_action(action_id));
    };
    if dataset.is_empty() {
        return Err(ConnectorError::invalid_action(action_id));
    }
    Ok(dataset)
}

fn required_resource_id<'a>(action_id: &str, params: &'a Value) -> Result<&'a str, ConnectorError> {
    params
        .get(RESOURCE_ID_PARAM)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ConnectorError::InvalidParams {
            action_id: action_id.to_owned(),
            reason: format!("`{RESOURCE_ID_PARAM}` must be a non-empty string"),
        })
}

fn snapshot_belongs_to_dataset(snapshot_id: &str, dataset: &str) -> bool {
    snapshot_id
        .strip_prefix(dataset)
        .is_some_and(|suffix| suffix.starts_with('@') && suffix.len() > 1)
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
        (DATA_POINT_POOL_STORAGE_BREAKDOWN, json!([])),
        (DATA_POINT_ACTIVE_ALERT_COUNT, json!(0)),
        (DATA_POINT_SYSTEM_UPTIME, json!("unavailable")),
        ("error", json!(reason)),
    ] {
        set_detail(&mut details, None, id, value);
    }
    ConnectorStatus::new(HealthState::Down, details)
        .with_target_health(String::new(), HealthState::Down)
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
                "hostname": "nas",
                "uptime": "3 days, 04:05:06"
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
                uptime: "3 days, 04:05:06".to_owned(),
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
            json!({ "version": "25.10", "hostname": "nas", "uptime": "1 day" }),
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
            DATA_POINT_POOL_STORAGE_BREAKDOWN,
            DATA_POINT_ACTIVE_ALERT_COUNT,
            DATA_POINT_SYSTEM_UPTIME,
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
    fn only_non_dismissed_alerts_count_as_active() {
        assert_eq!(
            map_active_alert_count(json!([
                { "id": "active", "dismissed": false },
                { "id": "dismissed", "dismissed": true }
            ]))
            .unwrap(),
            1
        );
        assert!(map_active_alert_count(json!({})).is_err());
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
    fn pool_resource_rows_map_capacity_and_target_identity() {
        let rows = pool_resource_items(
            map_pool_readings(json!([{
                "name": "tank",
                "status": "ONLINE",
                "size": 1000,
                "allocated": 400,
                "free": 600
            }]))
            .unwrap(),
        );

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "tank");
        assert_eq!(rows[0].fields.get("targetId"), Some(&json!("pool:tank")));
        assert_eq!(rows[0].fields.get("name"), Some(&json!("tank")));
        assert_eq!(rows[0].fields.get("status"), Some(&json!("ONLINE")));
        assert_eq!(rows[0].fields.get("usedBytes"), Some(&json!(400)));
        assert_eq!(rows[0].fields.get("freeBytes"), Some(&json!(600)));
    }

    #[test]
    fn dataset_resource_rows_map_properties_count_and_target_identity() {
        let rows = dataset_resource_items(vec![DatasetReadings {
            path: "tank/apps".to_owned(),
            used_bytes: 4096,
            available_bytes: 8192,
            compression_ratio: 1.25,
            snapshot_count: Some(3),
        }]);

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "tank/apps");
        assert_eq!(
            rows[0].fields.get("targetId"),
            Some(&json!("dataset:tank/apps"))
        );
        assert_eq!(rows[0].fields.get("pool"), Some(&json!("tank")));
        assert_eq!(rows[0].fields.get("usedBytes"), Some(&json!(4096)));
        assert_eq!(rows[0].fields.get("snapshotCount"), Some(&json!(3)));
    }

    #[test]
    fn snapshot_resource_rows_map_requested_properties() {
        let rows = map_snapshot_resource_items(json!([{
            "id": "tank/apps@before-upgrade",
            "snapshot_name": "before-upgrade",
            "dataset": "tank/apps",
            "createtxg": "123",
            "properties": {
                "creation": {
                    "value": "Wed Jul  3 09:46 2024",
                    "rawvalue": "1720000000",
                    "parsed": 1720000000,
                    "source": "NONE"
                },
                "used": {
                    "value": "2 KiB",
                    "rawvalue": "2048",
                    "parsed": 2048,
                    "source": "NONE"
                }
            }
        }]))
        .expect("documented pool.snapshot.query shape");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "tank/apps@before-upgrade");
        assert_eq!(rows[0].fields.get("name"), Some(&json!("before-upgrade")));
        assert_eq!(rows[0].fields.get("usedBytes"), Some(&json!(2048)));
        assert_eq!(
            rows[0].fields.get("created"),
            Some(&json!("2024-07-03T09:46:40+00:00"))
        );
    }

    #[test]
    fn alert_resource_rows_keep_only_active_alerts() {
        let rows = map_alert_resource_items(json!([
            {
                "uuid": "active-id",
                "level": "WARNING",
                "text": "Pool space is low",
                "datetime": "2026-09-03T10:00:00+00:00",
                "dismissed": false
            },
            {
                "uuid": "dismissed-id",
                "level": "INFO",
                "text": "Already handled",
                "datetime": "2026-09-02T10:00:00+00:00",
                "dismissed": true
            }
        ]))
        .expect("documented alert.list shape");

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "active-id");
        assert_eq!(rows[0].fields.get("level"), Some(&json!("WARNING")));
        assert_eq!(
            rows[0].fields.get("message"),
            Some(&json!("Pool space is low"))
        );
    }

    #[test]
    fn resource_kinds_are_conditioned_by_target_kind() {
        let host: Vec<_> = host_resource_kinds()
            .into_iter()
            .map(|kind| kind.kind)
            .collect();
        assert_eq!(
            host,
            vec![
                RESOURCE_KIND_POOLS,
                RESOURCE_KIND_DATASETS,
                RESOURCE_KIND_ALERTS
            ]
        );

        let dataset = snapshots_kind("dataset:tank/apps");
        assert_eq!(dataset.kind, RESOURCE_KIND_SNAPSHOTS);
        assert_eq!(
            dataset.kind_actions[0].target_id.as_deref(),
            Some("dataset:tank/apps")
        );
        assert!(dataset
            .row_actions
            .iter()
            .any(|action| { action.id == ACTION_ROLLBACK_SNAPSHOT && action.is_disruptive }));
    }

    #[test]
    fn snapshot_ids_cannot_escape_the_scoped_dataset() {
        assert!(snapshot_belongs_to_dataset(
            "tank/apps@before-upgrade",
            "tank/apps"
        ));
        assert!(!snapshot_belongs_to_dataset(
            "tank/apps-child@before-upgrade",
            "tank/apps"
        ));
        assert!(!snapshot_belongs_to_dataset(
            "tank/other@before-upgrade",
            "tank/apps"
        ));
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
                uptime: "1 day".to_owned(),
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
            active_alert_count: None,
            warnings: vec!["pool.dataset.query failed: permission denied".to_owned()],
        };

        assert_eq!(inventory.health(), HealthState::Degraded);
    }

    #[test]
    fn a_failed_pool_still_marks_the_connector_down_when_telemetry_is_partial() {
        let inventory = InventoryReadings {
            host: HostReadings {
                version: "25.10".to_owned(),
                uptime: "1 day".to_owned(),
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
            active_alert_count: Some(0),
            warnings: vec!["snapshot count unavailable".to_owned()],
        };

        assert_eq!(inventory.health(), HealthState::Down);
    }
}
