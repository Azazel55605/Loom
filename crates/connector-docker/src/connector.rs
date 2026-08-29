//! One Docker connection with host-level and per-container views.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use bollard::models::{
    ContainerCpuStats, ContainerInspectResponse, ContainerStatsResponse, ContainerSummary,
};
use bollard::query_parameters::{
    ListContainersOptionsBuilder, ListImagesOptionsBuilder, ListNetworksOptions,
    ListVolumesOptions, LogsOptionsBuilder, StatsOptionsBuilder,
};
use bollard::Docker;
use chrono::{DateTime, Utc};
use futures_util::{stream, StreamExt};
use loom_core::connector::{
    details::set_detail, ActionResult, ActionWidgetType, ApplicableTarget, CapabilityRequirement,
    CapabilityStatus, ChartType, ColumnDescriptor, ColumnValueType, ConnectionTestResult,
    Connector, ConnectorAction, ConnectorError, ConnectorMetadata, ConnectorStatus,
    DataPointDescriptor, DataPointValueType, DisplayField, DisplayWidgetType, HealthState,
    NetworkTarget, ResourceItem, ResourceKindDescriptor, SetupGuide, SetupGuideToggle,
    SetupGuideVariant, SubTarget, UpdateCheckResult, WidgetBinding, WidgetLayout,
};
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::config::{config_schema, DockerConnectorConfig};
use crate::metrics::{cpu_percent, format_uptime, health_for_state};
use crate::registry::{HttpRegistry, RegistryTransport};
use crate::updates::{apply_update, check_container, configured_image_ref, UpdateCache};

/// The connector type id this registers under.
pub const TYPE_ID: &str = "docker";

/// Human-facing name for the type picker.
pub const DISPLAY_NAME: &str = "Docker";

/// Icon reference, in the `ConnectorMetadata::icon` convention. Vendored under
/// `packages/ui-kit/src/assets/icons/brand` — see `docs/THIRD_PARTY_ICONS.md`
/// for its license and attribution.
///
/// A `const` rather than a literal in [`DockerConnector::metadata`] because the
/// type registry needs the same value *before* any instance exists: a Docker
/// connector cannot be default-constructed to be asked, so the registration
/// would otherwise carry a hand-copied duplicate that could drift from what the
/// connector actually reports.
pub const ICON: &str = "brand:docker";

/// The action id that replaces a container's image — in either direction.
///
/// Takes `{ "targetImageRef": string }`. Given a newer reference it is an
/// update; given the reference the action log recorded before the last update
/// it is a rollback. There is deliberately no second action for the second
/// case: they are the same operation with a different argument, and a separate
/// `rollback` would need its own state to know what to roll back *to* — state
/// the action log already holds.
pub const ACTION_APPLY_UPDATE: &str = "applyUpdate";

/// Resource kind listing containers with an update waiting.
pub const RESOURCE_KIND_UPDATES: &str = "updates";

/// The kind-level action that applies every waiting update in turn.
pub const ACTION_UPDATE_ALL: &str = "updateAll";

/// Data point ids. Public because a dashboard layout stores them, so a rename
/// is a breaking change to saved layouts and should be visible as one.
pub const DATA_POINT_STATUS: &str = "status";
pub const DATA_POINT_CPU_PERCENT: &str = "cpuPercent";
pub const DATA_POINT_CPU_HISTORY: &str = "cpuHistory";
pub const DATA_POINT_MEMORY_USAGE_BYTES: &str = "memoryUsageBytes";
pub const DATA_POINT_MEMORY_HISTORY: &str = "memoryHistory";
pub const DATA_POINT_UPTIME: &str = "uptime";
pub const DATA_POINT_LOGS: &str = "logs";
/// The image reference a container was created from.
///
/// A data point rather than an internal lookup, because that is what makes it
/// snapshottable: [`ACTION_APPLY_UPDATE`] declares this id in
/// `snapshot_data_point_ids`, so the platform records what the container was
/// running *before* an update, in the action log, with no Docker-specific
/// bookkeeping. That recorded value is what a rollback later passes back in as
/// `targetImageRef`.
pub const DATA_POINT_IMAGE_REF: &str = "currentImageRef";
pub const DATA_POINT_TOTAL_CONTAINERS: &str = "totalContainers";
pub const DATA_POINT_RUNNING_CONTAINERS: &str = "runningContainers";
pub const DATA_POINT_STOPPED_CONTAINERS: &str = "stoppedContainers";
pub const DATA_POINT_TOTAL_IMAGES: &str = "totalImages";
pub const DATA_POINT_DISK_USAGE_BYTES: &str = "diskUsageBytes";
/// The image half of [`DATA_POINT_DISK_USAGE_BYTES`], reported separately
/// because it is the part a user can act on: images are what the Images table
/// prunes, and "29 GB of Docker" does not tell anyone whether pruning would
/// help.
pub const DATA_POINT_IMAGE_DISK_USAGE_BYTES: &str = "imageDiskUsageBytes";
pub const DATA_POINT_DOCKER_VERSION: &str = "dockerVersion";

/// Action ids.
pub const ACTION_START: &str = "start";
pub const ACTION_STOP: &str = "stop";
pub const ACTION_RESTART: &str = "restart";
pub const ACTION_PAUSE: &str = "pause";
pub const ACTION_UNPAUSE: &str = "unpause";

const CAPABILITY_LIST_CONTAINERS: &str = "list-containers";
const CAPABILITY_READ_LOGS: &str = "read-logs";
const CAPABILITY_START: &str = "start-containers";
const CAPABILITY_STOP: &str = "stop-containers";
const CAPABILITY_RESTART: &str = "restart-containers";
const CAPABILITY_PAUSE: &str = "pause-containers";
const CAPABILITY_UNPAUSE: &str = "unpause-containers";
const CAPABILITY_HOST_SUMMARY: &str = "host-summary";
const CAPABILITY_LIST_IMAGES: &str = "list-images";
const CAPABILITY_PULL_IMAGE: &str = "pull-image";
const CAPABILITY_DELETE_IMAGE: &str = "delete-image";
const CAPABILITY_PRUNE_IMAGES: &str = "prune-images";
const CAPABILITY_LIST_VOLUMES: &str = "list-volumes";
const CAPABILITY_CREATE_VOLUME: &str = "create-volume";
const CAPABILITY_DELETE_VOLUME: &str = "delete-volume";
const CAPABILITY_LIST_NETWORKS: &str = "list-networks";
const CAPABILITY_CREATE_NETWORK: &str = "create-network";
const CAPABILITY_DELETE_NETWORK: &str = "delete-network";
const CAPABILITY_LIST_UPDATES: &str = "list-updates";
const CAPABILITY_APPLY_UPDATE: &str = "apply-update";

// What LinuxServer's socket-proxy actually gates writes on.
//
// Its HAProxy rules contain exactly one method gate:
//
// ```text
// http-request deny unless METH_GET || { env(POST) -m bool }
// ```
//
// It sits **after** the per-action container rules (which is why `ALLOW_START`
// and friends work with `POST=0` — an earlier `http-request allow`
// short-circuits) and **before** every category rule (`IMAGES`, `VOLUMES`,
// `NETWORKS`, `CONTAINERS`, …). So `POST` is not a POST-verb toggle at all: it
// is an **any-method-but-GET** master gate, and `DELETE` is covered by it
// despite never being named. There is no per-category write toggle and no
// `DELETE` toggle to offer instead.
//
// Verified against `lscr.io/linuxserver/socket-proxy:latest`: with
// `IMAGES=VOLUMES=NETWORKS=1` and `POST=0`, the three `GET` listings answer
// `200` while every `DELETE` and `POST` on the same paths answers `403`;
// adding `POST=1` turns those into the daemon's own `404`/`201`/`400`.
//
// Hence every write capability below requires its category toggle **and**
// `post`, and every read capability requires only its category toggle.

fn setup_toggle(
    key: &str,
    env_var: &str,
    label: &str,
    description: &str,
    default: bool,
    recommended: bool,
) -> SetupGuideToggle {
    SetupGuideToggle {
        key: key.to_owned(),
        env_var: env_var.to_owned(),
        label: label.to_owned(),
        description: description.to_owned(),
        default,
        recommended,
    }
}

fn capability_requirement(
    key: &str,
    label: &str,
    required_toggle_keys: &[&str],
) -> CapabilityRequirement {
    CapabilityRequirement {
        capability_key: key.to_owned(),
        label: label.to_owned(),
        required_toggle_keys: required_toggle_keys
            .iter()
            .map(|key| (*key).to_owned())
            .collect(),
    }
}

/// Type-level setup paths; available before a Docker endpoint can be reached.
pub fn setup_guide() -> SetupGuide {
    SetupGuide {
        variants: vec![
            SetupGuideVariant {
                id: "socket".to_owned(),
                label: "Direct socket".to_owned(),
                description: "Use this only when Loom's web-backend runs on the same machine as the Docker daemon. Mounting the Docker socket grants that backend full, unrestricted, effectively root-equivalent control of the host. Adding :ro to the bind mount does not restrict Docker API calls, so this example does not imply that it does."
                    .to_owned(),
                template: "services:\n  web-backend:\n    volumes:\n      - /var/run/docker.sock:/var/run/docker.sock\n\n# Enter this in Loom (default):\ndockerHost: {{dockerHost}}\n# Expected default value: unix:///var/run/docker.sock"
                    .to_owned(),
                toggles: Vec::new(),
                capability_requirements: Vec::new(),
            },
            SetupGuideVariant {
                id: "proxy".to_owned(),
                label: "Via socket proxy".to_owned(),
                description: "Uses LinuxServer's socket-proxy, whose current rules provide separate opt-in gates for logs, archive, export, process, and lifecycle endpoints. CVE-2026-78122 names Tecnativa docker-socket-proxy through 0.5.0; no published advisory against LinuxServer's image was found when this guide was verified, and its current image includes the finer-grained read gates. A Docker proxy remains highly privileged: keep it reachable only by Loom and review upstream release notes before updating."
                    .to_owned(),
                template: "services:\n  socket-proxy:\n    image: lscr.io/linuxserver/socket-proxy:latest\n    environment:\n      PING: \"{{PING}}\"\n      VERSION: \"{{VERSION}}\"\n      CONTAINERS: \"{{CONTAINERS}}\"\n      ALLOW_LOGS: \"{{ALLOW_LOGS}}\"\n      ALLOW_START: \"{{ALLOW_START}}\"\n      ALLOW_STOP: \"{{ALLOW_STOP}}\"\n      ALLOW_RESTARTS: \"{{ALLOW_RESTARTS}}\"\n      ALLOW_PAUSE: \"{{ALLOW_PAUSE}}\"\n      ALLOW_UNPAUSE: \"{{ALLOW_UNPAUSE}}\"\n      INFO: \"{{INFO}}\"\n      SYSTEM: \"{{SYSTEM}}\"\n      IMAGES: \"{{IMAGES}}\"\n      VOLUMES: \"{{VOLUMES}}\"\n      NETWORKS: \"{{NETWORKS}}\"\n      # Deleting an image, volume or network, and pulling one, are not GET\n      # requests, and POST is this proxy's only method gate. The three vars\n      # above alone give read-only browsing.\n      POST: \"{{POST}}\"\n      # Loom does not use these sensitive read endpoints. Keep them denied.\n      ALLOW_ARCHIVE: \"0\"\n      ALLOW_CHANGES: \"0\"\n      ALLOW_EXPORT: \"0\"\n      ALLOW_TOP: \"0\"\n    volumes:\n      - /var/run/docker.sock:/var/run/docker.sock:ro\n    read_only: true\n    tmpfs:\n      - /run\n    networks:\n      - loom-docker-api\n\n  web-backend:\n    networks:\n      - loom-docker-api\n\nnetworks:\n  loom-docker-api:\n    internal: true\n\n# Enter this in Loom:\ndockerHost: tcp://socket-proxy:2375\n\n# Do not publish port 2375. If the proxy and Loom must be on different hosts,\n# expose it only through a VPN, firewall allowlist, or authenticated TLS tunnel.\n# Never expose this unauthenticated plain-HTTP proxy on 0.0.0.0."
                    .to_owned(),
                toggles: vec![
                    setup_toggle(
                        "ping",
                        "PING",
                        "Allow ping",
                        "Keeps Loom's lightweight reachability check available. Upstream default: on.",
                        true,
                        true,
                    ),
                    setup_toggle(
                        "version",
                        "VERSION",
                        "Allow version",
                        "Lets Loom verify the daemon version during setup and display it in the host summary. Upstream default: on.",
                        true,
                        true,
                    ),
                    setup_toggle(
                        "containers",
                        "CONTAINERS",
                        "Allow container access",
                        "Required for listing, inspecting, and stats. LinuxServer default: off; this guide enables it because those are Loom's core container features. Sensitive subpaths remain behind the separate ALLOW_* gates below.",
                        true,
                        true,
                    ),
                    setup_toggle(
                        "allowLogs",
                        "ALLOW_LOGS",
                        "Allow logs",
                        "Allows only the container logs subpath. LinuxServer default: off; recommended because Loom's logs data point uses it.",
                        true,
                        true,
                    ),
                    setup_toggle(
                        "allowStart",
                        "ALLOW_START",
                        "Allow start",
                        "Allows Loom's start action. LinuxServer default: off; this action-specific gate works while POST remains off.",
                        true,
                        true,
                    ),
                    setup_toggle(
                        "allowStop",
                        "ALLOW_STOP",
                        "Allow stop",
                        "Allows Loom's stop action. LinuxServer default: off; this action-specific gate works while POST remains off.",
                        true,
                        true,
                    ),
                    setup_toggle(
                        "allowRestarts",
                        "ALLOW_RESTARTS",
                        "Allow restarts",
                        "Allows restart and kill, and also permits stop in LinuxServer's current rules. Default: off; keep this disruptive action opt-in.",
                        false,
                        false,
                    ),
                    setup_toggle(
                        "allowPause",
                        "ALLOW_PAUSE",
                        "Allow pause",
                        "Allows Loom's pause action. LinuxServer default: off; this action-specific gate works while POST remains off.",
                        true,
                        true,
                    ),
                    setup_toggle(
                        "allowUnpause",
                        "ALLOW_UNPAUSE",
                        "Allow unpause",
                        "Allows Loom's resume action. LinuxServer default: off; this action-specific gate works while POST remains off.",
                        true,
                        true,
                    ),
                    setup_toggle(
                        "info",
                        "INFO",
                        "Allow host information",
                        "Enables Docker host container and image totals used by Loom's host-summary view. LinuxServer default: off; leave it off for per-container-only views.",
                        false,
                        false,
                    ),
                    setup_toggle(
                        "system",
                        "SYSTEM",
                        "Allow disk-usage information",
                        "Enables /system/df, which Loom uses for Docker disk usage. LinuxServer default: off; leave it off unless you want the host-summary view.",
                        false,
                        false,
                    ),
                    setup_toggle(
                        "images",
                        "IMAGES",
                        "Allow image access",
                        "Enables Loom's Images table: browsing the daemon's images, and — with POST also on — pulling and deleting them. Also what per-container update checking reads to compare a running image against its registry. LinuxServer default: off.",
                        false,
                        false,
                    ),
                    setup_toggle(
                        "networks",
                        "NETWORKS",
                        "Allow network access",
                        "Enables Loom's Networks table: browsing the daemon's networks, and — with POST also on — creating and deleting them. LinuxServer default: off.",
                        false,
                        false,
                    ),
                    setup_toggle(
                        "volumes",
                        "VOLUMES",
                        "Allow volume access",
                        "Enables Loom's Volumes table: browsing the daemon's volumes, and — with POST also on — creating and deleting them. LinuxServer default: off.",
                        false,
                        false,
                    ),
                    setup_toggle(
                        "post",
                        "POST",
                        "Allow other write requests",
                        "LinuxServer default: off. Despite its name this gates every method that is not GET, DELETE included, and it is the only such gate the proxy has. Loom's container lifecycle actions use their narrower ALLOW_* gates and work with POST=0; pulling and deleting images, and creating and deleting volumes and networks, cannot — they need this as well as their own category toggle above.",
                        false,
                        false,
                    ),
                ],
                capability_requirements: vec![
                    capability_requirement(
                        CAPABILITY_LIST_CONTAINERS,
                        "List containers",
                        &["containers"],
                    ),
                    capability_requirement(
                        CAPABILITY_READ_LOGS,
                        "Read container logs",
                        &["containers", "allowLogs"],
                    ),
                    capability_requirement(
                        CAPABILITY_START,
                        "Start containers",
                        &["containers", "allowStart"],
                    ),
                    capability_requirement(
                        CAPABILITY_STOP,
                        "Stop containers",
                        &["containers", "allowStop"],
                    ),
                    capability_requirement(
                        CAPABILITY_RESTART,
                        "Restart containers",
                        &["containers", "allowRestarts"],
                    ),
                    capability_requirement(
                        CAPABILITY_PAUSE,
                        "Pause containers",
                        &["containers", "allowPause"],
                    ),
                    capability_requirement(
                        CAPABILITY_UNPAUSE,
                        "Resume containers",
                        &["containers", "allowUnpause"],
                    ),
                    capability_requirement(
                        CAPABILITY_HOST_SUMMARY,
                        "View host summary",
                        &["info", "system", "version"],
                    ),
                    // `containers` as well as `images`: the images table's
                    // "Used by" column is a container listing, so a proxy with
                    // IMAGES but not CONTAINERS lists images that all claim
                    // nothing is using them.
                    capability_requirement(
                        CAPABILITY_LIST_IMAGES,
                        "Browse images",
                        &["containers", "images"],
                    ),
                    capability_requirement(
                        CAPABILITY_PULL_IMAGE,
                        "Pull images",
                        &["containers", "images", "post"],
                    ),
                    capability_requirement(
                        CAPABILITY_DELETE_IMAGE,
                        "Delete images",
                        &["containers", "images", "post"],
                    ),
                    capability_requirement(
                        CAPABILITY_PRUNE_IMAGES,
                        "Prune unused images",
                        &["containers", "images", "post"],
                    ),
                    capability_requirement(CAPABILITY_LIST_VOLUMES, "Browse volumes", &["volumes"]),
                    capability_requirement(
                        CAPABILITY_CREATE_VOLUME,
                        "Create volumes",
                        &["volumes", "post"],
                    ),
                    capability_requirement(
                        CAPABILITY_DELETE_VOLUME,
                        "Delete volumes",
                        &["volumes", "post"],
                    ),
                    capability_requirement(
                        CAPABILITY_LIST_NETWORKS,
                        "Browse networks",
                        &["networks"],
                    ),
                    capability_requirement(
                        CAPABILITY_CREATE_NETWORK,
                        "Create networks",
                        &["networks", "post"],
                    ),
                    capability_requirement(
                        CAPABILITY_DELETE_NETWORK,
                        "Delete networks",
                        &["networks", "post"],
                    ),
                    // An update check inspects the container (CONTAINERS) and
                    // its local image (IMAGES) before it asks a registry
                    // anything. It writes nothing, so it needs no POST.
                    capability_requirement(
                        CAPABILITY_LIST_UPDATES,
                        "Check for container updates",
                        &["containers", "images"],
                    ),
                    // Applying one pulls (POST /images/create), removes the old
                    // container (DELETE) and creates the replacement (POST) —
                    // three non-GET requests across two categories.
                    capability_requirement(
                        CAPABILITY_APPLY_UPDATE,
                        "Apply container updates",
                        &["containers", "images", "post"],
                    ),
                ],
            },
        ],
    }
}

/// How many samples each history data point keeps.
///
/// Fifty, matching `DebugConnector`. At the default poll interval that is a
/// window of several minutes — enough for a chart to show a shape, short
/// enough that the buffer costs nothing and that a restarted Loom is not
/// missing hours of history anyone was relying on. This is a live view, not a
/// metrics store: a homelab that wants retention wants Prometheus, and Loom
/// should not pretend otherwise by keeping ten thousand points in memory.
pub const HISTORY_CAPACITY: usize = 50;

/// How many log lines the `logs` data point carries.
pub const LOG_TAIL_LINES: usize = 20;

/// Slow daemon-wide values do not need five-second freshness.
///
/// `/system/df` can return megabytes and take several seconds even through a
/// local socket proxy. Fetching it on every ordinary status poll both loads the
/// daemon and turns an otherwise quick health check into a likely client
/// timeout on a remote host. Version is effectively static for the same
/// process lifetime, so it shares this conservative refresh window.
const HOST_DETAILS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);

/// Keep a large host from opening an unbounded burst through its socket proxy.
const CONTAINER_POLL_CONCURRENCY: usize = 4;

/// One point in a history buffer.
///
/// Serialized as `{ "timestamp": "…", "value": … }`, which is the shape the
/// `timeSeries` value type promises and the chart widget reads.
#[derive(Debug, Clone, Serialize)]
struct HistorySample {
    timestamp: DateTime<Utc>,
    value: f64,
}

/// The rolling buffers, which are the only state this connector carries
/// between polls.
#[derive(Debug, Default)]
struct History {
    cpu: VecDeque<HistorySample>,
    memory: VecDeque<HistorySample>,
    /// The one-shot Docker stats endpoint has no useful `precpu_stats`.
    /// Retaining the previous cumulative counters here gives the next poll the
    /// same delta without holding one HTTP request open for two sample cycles.
    previous_cpu: Option<ContainerCpuStats>,
}

impl History {
    /// Appends both samples, dropping the oldest once the buffer is full.
    fn record(&mut self, cpu: f64, memory: f64, at: DateTime<Utc>) {
        for (buffer, value) in [(&mut self.cpu, cpu), (&mut self.memory, memory)] {
            if buffer.len() == HISTORY_CAPACITY {
                buffer.pop_front();
            }
            buffer.push_back(HistorySample {
                timestamp: at,
                value,
            });
        }
    }
}

#[derive(Debug, Clone)]
struct CachedHostDetails {
    disk_usage: i64,
    image_disk_usage: i64,
    version: String,
    errors: Vec<String>,
    refreshed_at: Instant,
}

/// Monitors one Docker host and every addressable container below it.
pub struct DockerConnector {
    config: DockerConnectorConfig,
    /// Reads — inspect, stats, logs — on a short timeout, so a host that has
    /// gone away fails a poll quickly instead of stacking them up.
    docker: Docker,
    /// Lifecycle actions, on a much longer one. `docker stop` legitimately
    /// blocks for the container's whole stop grace period, and a client that
    /// gave up first would report a failure for a stop that succeeded. Two
    /// clients rather than one compromise timeout, because there is no single
    /// value that is both "fail fast" and "wait out a shutdown".
    control: Docker,
    /// `std::sync::Mutex`, not tokio's: every critical section here is a few
    /// pushes with no `await` inside it, so an async mutex would buy nothing
    /// and cost a scheduler hop per poll.
    history: Arc<Mutex<HashMap<String, History>>>,
    /// Last successful cheap enumeration. `data_points()` is intentionally
    /// synchronous in the shared trait, so live target discovery refreshes
    /// this cache before descriptors are read.
    known_targets: Arc<Mutex<Vec<SubTarget>>>,
    /// Cached because Docker's disk-usage endpoint is far more expensive than
    /// the five-second health cadence and can overwhelm a remote socket proxy.
    host_details: Arc<Mutex<Option<CachedHostDetails>>>,
    /// What the last update check found, per container.
    ///
    /// Held here rather than only in the backend because the browsable
    /// `updates` table is a connector-declared resource kind, and a listing
    /// that re-queried the registry per browse would spend someone else's rate
    /// limit on a page refresh. The scheduler's checks fill this in; the table
    /// reports what they found and says when.
    update_cache: Arc<Mutex<UpdateCache>>,
    /// HTTPS client for registry queries. `None` when one could not be built,
    /// which makes update checking unavailable rather than the connector
    /// unusable — a Docker host is still worth monitoring without it.
    registry: Option<Arc<dyn RegistryTransport>>,
}

/// Hand-written because `bollard::Docker` is not `Debug`, and because the
/// interesting part of a connector in a log line is which Docker endpoint
/// it points at — not the state of an HTTP connection pool.
impl std::fmt::Debug for DockerConnector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DockerConnector")
            .field("docker_host", &self.config.docker_host)
            .finish_non_exhaustive()
    }
}

impl DockerConnector {
    /// Builds both Docker clients without contacting the configured endpoint.
    fn prepare(config: DockerConnectorConfig) -> Result<Self, ConnectorError> {
        let docker = config.connect()?;
        let control = config.connect_for_control()?;
        Ok(Self {
            config,
            docker,
            control,
            history: Arc::new(Mutex::new(HashMap::new())),
            known_targets: Arc::new(Mutex::new(Vec::new())),
            host_details: Arc::new(Mutex::new(None)),
            update_cache: Arc::new(Mutex::new(UpdateCache::new())),
            registry: HttpRegistry::new()
                .ok()
                .map(|client| Arc::new(client) as Arc<dyn RegistryTransport>),
        })
    }

    /// Builds a connector and proves it can be used.
    ///
    /// The daemon is pinged and its cheap container list is read here so a bad
    /// endpoint is refused while the setup form is still open, and the
    /// synchronous descriptor cache starts with the current targets.
    pub async fn connect(config: DockerConnectorConfig) -> Result<Self, ConnectorError> {
        let connector = Self::prepare(config)?;
        connector.docker.ping().await.map_err(|error| {
            ConnectorError::unreachable(format!(
                "could not reach the Docker host at {}: {error}",
                connector.config.docker_host
            ))
        })?;
        connector.list_sub_targets_live().await?;
        Ok(connector)
    }

    /// Convenience for the registry factory: parse, then connect.
    pub async fn from_config_value(config: Value) -> Result<Self, ConnectorError> {
        Self::connect(DockerConnectorConfig::from_value(config)?).await
    }

    /// Builds the throwaway connector used by the setup connection check.
    ///
    /// Unlike [`Self::connect`], this deliberately performs no API call before
    /// returning. A restrictive socket proxy may allow ping while denying
    /// containers, info, or system endpoints; constructing through the normal
    /// factory would turn that useful capability result into an early 400.
    pub fn from_config_value_for_connection_test(config: Value) -> Result<Self, ConnectorError> {
        Self::prepare(DockerConnectorConfig::from_value(config)?)
    }

    /// Every container with an update waiting, as the last check left it.
    ///
    /// Read from the cache rather than checked live: a browse must not spend
    /// registry budget, and "when this was last checked" is a column in the
    /// table precisely so the reading's age is visible rather than implied.
    fn outdated_containers(&self) -> Vec<(String, crate::updates::UpdateReading)> {
        let cache = self
            .update_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let mut rows: Vec<(String, crate::updates::UpdateReading)> = cache
            .iter()
            .filter(|(_, reading)| reading.available)
            .map(|(name, reading)| (name.clone(), reading.clone()))
            .collect();
        // Sorted by name so a table does not reshuffle between refreshes; a
        // `HashMap`'s order varies per process.
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        rows
    }

    /// Applies every waiting update, one container at a time.
    ///
    /// **Sequential on purpose, and not only for the registry's sake.** Pulling
    /// several images at once on a home server saturates the link the services
    /// themselves are answering on, and recreating several containers at once
    /// takes down things that depend on each other simultaneously. One at a
    /// time is slower and is the behaviour someone would choose if asked.
    ///
    /// A container that fails does not stop the rest: the point of "update all"
    /// is to get through the list, and the per-container outcome is reported in
    /// the summary and recorded, invocation by invocation, in the action log.
    async fn update_all(&self) -> Result<ActionResult, ConnectorError> {
        let waiting = self.outdated_containers();
        if waiting.is_empty() {
            return Ok(ActionResult::ok("No containers have a waiting update."));
        }

        let mut applied = Vec::new();
        let mut failed = Vec::new();
        for (name, reading) in waiting {
            let Some(target) = reading.latest_ref.as_deref().or(Some(&reading.current_ref)) else {
                continue;
            };
            match apply_update(&self.control, &name, target).await {
                Ok(result) if result.success => {
                    self.update_cache
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .remove(&name);
                    applied.push(name);
                }
                Ok(result) => failed.push(format!("{name} ({})", result.message)),
                Err(error) => failed.push(format!("{name} ({error})")),
            }
        }

        let message = match (applied.len(), failed.len()) {
            (updated, 0) => format!("Updated {updated} container(s): {}.", applied.join(", ")),
            (0, _) => format!("No container could be updated: {}.", failed.join("; ")),
            (updated, _) => format!(
                "Updated {updated} container(s): {}. Failed: {}.",
                applied.join(", "),
                failed.join("; ")
            ),
        };

        Ok(ActionResult {
            success: failed.is_empty(),
            message,
            payload: Some(json!({ "applied": applied, "failed": failed })),
        })
    }

    /// One immediate sample of cumulative container stats.
    ///
    /// Uses Docker's one-shot form so one container does not hold the proxy
    /// connection open for one or two collection cycles. The previous
    /// cumulative counters live in [`History`], making the second and later
    /// polls just as measurable without multiplying poll duration by the
    /// number of containers. The first sample is deliberately 0% because no
    /// interval exists yet.
    async fn sample_stats(
        &self,
        container_name: &str,
    ) -> Result<Option<ContainerStatsResponse>, ConnectorError> {
        let options = StatsOptionsBuilder::new()
            .stream(false)
            .one_shot(true)
            .build();
        let mut stream = self.docker.stats(container_name, Some(options));

        match stream.next().await {
            Some(Ok(sample)) => Ok(Some(sample)),
            // Docker closes the stats stream without a sample for a container
            // that is not running. That is not an error — it is the answer.
            None => Ok(None),
            Some(Err(error)) => Err(ConnectorError::unreachable(format!(
                "reading container stats failed: {error}"
            ))),
        }
    }

    /// The last [`LOG_TAIL_LINES`] lines, stdout and stderr, newline-joined.
    ///
    /// A best-effort read: a container with no log driver, or one whose driver
    /// does not support reading back (`none`, some syslog setups), answers with
    /// an error that is not a reason to fail the whole poll. The message
    /// becomes the data point's value so it is visible in the log pane rather
    /// than swallowed.
    ///
    /// Joined into one `String` rather than sent as an array because the
    /// declared value type is `String`, and a second wire shape for one type is
    /// how a renderer and a connector quietly disagree.
    async fn tail_logs(&self, container_name: &str) -> String {
        let options = LogsOptionsBuilder::new()
            .stdout(true)
            .stderr(true)
            .follow(false)
            .tail(&LOG_TAIL_LINES.to_string())
            .build();

        let mut stream = self.docker.logs(container_name, Some(options));
        let mut lines: Vec<String> = Vec::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(output) => lines.push(output.to_string()),
                Err(error) => return format!("logs unavailable: {error}"),
            }
        }

        // Docker frames arrive with their own newlines and do not necessarily
        // align with lines, so the pieces are concatenated and split once
        // rather than joined per frame.
        let joined = lines.concat();
        joined.trim_end_matches('\n').to_owned()
    }

    /// Runs one lifecycle operation, mapping Docker's answer onto the
    /// reached-and-declined / could-not-reach split the trait draws.
    async fn run_lifecycle(
        &self,
        action_id: &str,
        name: &str,
    ) -> Result<ActionResult, ConnectorError> {
        // `self.control`, never `self.docker`: see the field docs.
        //
        // No explicit stop timeout is passed, so Docker applies the container's
        // own `StopTimeout`. Overriding it here would mean Loom deciding how
        // long someone's database gets to flush, which is a decision that
        // belongs to whoever configured the container.
        let outcome = match action_id {
            ACTION_START => self.control.start_container(name, None).await,
            ACTION_STOP => self.control.stop_container(name, None).await,
            ACTION_RESTART => self.control.restart_container(name, None).await,
            ACTION_PAUSE => self.control.pause_container(name).await,
            ACTION_UNPAUSE => self.control.unpause_container(name).await,
            other => return Err(ConnectorError::invalid_action(other)),
        };

        match outcome {
            Ok(()) => Ok(ActionResult::ok(format!("{name}: {action_id} succeeded"))),
            // The daemon answered and refused — "container already started",
            // "container is not paused". Its own words are the useful ones, so
            // they are passed through rather than replaced with a generic
            // failure, and this is `success: false` rather than `Err` because
            // Loom did reach the service.
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code,
                message,
            }) => Ok(ActionResult::failed(format!(
                "{name}: Docker refused {action_id} ({status_code}): {}",
                message.trim()
            ))
            .with_payload(json!({ "statusCode": status_code }))),
            // Anything else is transport: the request never got an answer.
            Err(error) => Err(ConnectorError::unreachable(format!(
                "{action_id} could not be sent to Docker: {error}"
            ))),
        }
    }

    async fn list_sub_targets_live(&self) -> Result<Vec<SubTarget>, ConnectorError> {
        let options = ListContainersOptionsBuilder::new().all(true).build();
        let containers = self
            .docker
            .list_containers(Some(options))
            .await
            .map_err(|error| {
                ConnectorError::unreachable(format!(
                    "listing containers on {} failed: {error}",
                    self.config.docker_host
                ))
            })?;
        let targets: Vec<SubTarget> = containers
            .into_iter()
            .filter_map(sub_target_from_summary)
            .collect();
        *self
            .known_targets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = targets.clone();
        Ok(targets)
    }
}

fn sub_target_from_summary(container: ContainerSummary) -> Option<SubTarget> {
    let id = container
        .names
        .and_then(|names| names.into_iter().next())
        .map(|name| name.trim_start_matches('/').to_owned())
        .filter(|name| !name.is_empty())
        .or_else(|| container.id.map(|id| id.chars().take(12).collect()))?;
    let label = container
        .image
        .filter(|image| !image.is_empty() && image != &id)
        .map_or_else(|| id.clone(), |image| format!("{id} ({image})"));
    Some(SubTarget { id, label })
}

/// The human half of a failed inspect, for the `error` detail on a poll.
///
/// Not `inspect_failure(..).to_string()`, which would prefix a missing
/// container with "invalid parameters for action configuration:" — accurate
/// when a *factory* refuses a configuration, and nonsense on a dashboard tile
/// three days later. The variant carries the routing; only the reason is worth
/// showing here.
fn poll_failure_reason(
    config: &DockerConnectorConfig,
    container_name: &str,
    error: &bollard::errors::Error,
) -> String {
    match inspect_failure(config, container_name, error) {
        ConnectorError::InvalidParams { reason, .. }
        | ConnectorError::Unreachable { reason }
        | ConnectorError::InvalidConfig { reason }
        | ConnectorError::AuthFailed { reason } => reason,
        other => other.to_string(),
    }
}

/// Describes a failed optional host reading without implying that Docker itself
/// is unreachable.
///
/// These messages are surfaced beside a Degraded badge. A bare Bollard
/// `Timeout error` made an otherwise working socket proxy look disconnected,
/// even though container status and actions were still available.
fn optional_host_read_failure(label: &str, error: &bollard::errors::Error) -> String {
    match error {
        bollard::errors::Error::RequestTimeoutError => {
            format!("{label} timed out. Container status and actions remain available.")
        }
        other => format!("{label} is unavailable: {other}"),
    }
}

/// Turns a failed inspect into the error that names the right thing to fix.
fn inspect_failure(
    config: &DockerConnectorConfig,
    container_name: &str,
    error: &bollard::errors::Error,
) -> ConnectorError {
    match error {
        // 404 means we got all the way to the daemon and it does not have this
        // container — the network is fine and the name is not.
        bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        } => ConnectorError::InvalidParams {
            action_id: "configuration".to_owned(),
            reason: format!(
                "the Docker host at {} has no container named {:?}. Check `docker ps -a` for the \
                 exact name or id.",
                config.docker_host, container_name
            ),
        },
        // Any other status is the daemon objecting to us rather than to the
        // name — a socket-proxy denying the endpoint, most often.
        bollard::errors::Error::DockerResponseServerError {
            status_code,
            message,
        } => ConnectorError::unreachable(format!(
            "the Docker host at {} answered {status_code}: {}",
            config.docker_host,
            message.trim()
        )),
        other => ConnectorError::unreachable(format!(
            "could not reach the Docker host at {}: {other}",
            config.docker_host
        )),
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

fn unavailable_capability(key: &str, label: &str, note: impl Into<String>) -> CapabilityStatus {
    CapabilityStatus {
        key: key.to_owned(),
        label: label.to_owned(),
        available: false,
        note: Some(note.into()),
    }
}

fn proxy_read_capability(
    key: &str,
    label: &str,
    relevant_env_vars: &str,
    result: Result<(), bollard::errors::Error>,
) -> CapabilityStatus {
    match result {
        Ok(()) => available_capability(key, label),
        Err(bollard::errors::Error::DockerResponseServerError {
            status_code: 403, ..
        }) => unavailable_capability(
            key,
            label,
            format!("Proxy configuration does not permit this — check {relevant_env_vars}."),
        ),
        Err(error) => unavailable_capability(key, label, format!("Read probe failed: {error}")),
    }
}

fn write_capability(key: &str, label: &str, relevant_env_vars: &str) -> CapabilityStatus {
    unavailable_capability(
        key,
        label,
        format!(
            "Cannot be safely verified without performing an action. Confirm {relevant_env_vars} if you need this."
        ),
    )
}

/// One capability that needs several reads to all succeed.
///
/// Borrows its probes rather than taking them, because `bollard::errors::Error`
/// is not `Clone` and one probe can legitimately decide two capabilities — a
/// container listing gates both "list containers" and, with the image listing,
/// "check for updates".
///
/// A `403` from any of them is the proxy declining, which names the toggles to
/// look at; anything else is reported as the transport failure it was, because
/// "check your configuration" is unhelpful advice for a connection that broke.
fn combine_read_probes(
    key: &str,
    label: &str,
    relevant_env_vars: &str,
    probes: &[&Result<(), bollard::errors::Error>],
) -> CapabilityStatus {
    if probes.iter().all(|probe| probe.is_ok()) {
        return available_capability(key, label);
    }
    if probes.iter().any(|probe| {
        matches!(
            probe,
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code: 403,
                ..
            })
        )
    }) {
        return unavailable_capability(
            key,
            label,
            format!("Proxy configuration does not permit this — check {relevant_env_vars}."),
        );
    }

    let errors = probes
        .iter()
        .filter_map(|probe| probe.as_ref().err())
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("; ");
    unavailable_capability(key, label, format!("Read probe failed: {errors}"))
}

impl DockerConnector {
    /// Probes the logs route without reading logs from a real container.
    ///
    /// A permitted route reaches Docker and returns 404 for the deliberately
    /// nonexistent id; a socket proxy denial returns 403 first. This verifies
    /// route access without exposing any container's log content.
    async fn probe_logs_route(&self) -> Result<(), bollard::errors::Error> {
        let options = LogsOptionsBuilder::new()
            .stdout(true)
            .stderr(true)
            .follow(false)
            .tail("1")
            .build();
        let mut stream = self
            .docker
            .logs("loom-capability-probe-does-not-exist", Some(options));
        while let Some(result) = stream.next().await {
            match result {
                Ok(_) => {}
                Err(bollard::errors::Error::DockerResponseServerError {
                    status_code: 404, ..
                }) => return Ok(()),
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Connector for DockerConnector {
    /// One poll: host summary plus inspect, stats, and logs for every container.
    ///
    /// # Why a dead daemon is `Ok(Down)` and not `Err`
    ///
    /// A failure to reach Docker is reported as a `Down` status carrying an
    /// `error` message, not as `Err`. The trait's `Err` arm means "the check
    /// could not be carried out", which this technically is — but for *this*
    /// connector the useful thing on a dashboard is a tile that says the
    /// container is not available and why, rather than a tile that blanks out
    /// to "no reading". The declared data points are still all present with
    /// their unavailable values, so a saved layout keeps rendering instead of
    /// collapsing to skeletons the moment a host goes away.
    ///
    /// Note that `details` therefore carries one key that is **not** a declared
    /// data point: `error`. It is deliberately not a data point, because it
    /// should not be bindable to a widget — it is diagnostic text, shown in the
    /// detail view, not something a user arranges on a dashboard.
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
        let mut status = self.host_status().await;
        let targets = match self.list_sub_targets_live().await {
            Ok(targets) => targets,
            Err(error) => {
                set_detail(&mut status.details, None, "error", json!(error.to_string()));
                status.health = HealthState::Down;
                return Ok(status);
            }
        };

        // Known trade-off: every poll fetches full detail for every container,
        // even when no active placement displays it. That is reasonable for a
        // typical homelab count; target-aware polling can be revisited if this
        // becomes a demonstrated cost.
        let target_values = stream::iter(targets.into_iter().map(|target| async move {
            let values = match self.docker.inspect_container(&target.id, None).await {
                Ok(inspect) => self.status_from(&target.id, inspect).await.1,
                Err(error) => {
                    unavailable_details(&poll_failure_reason(&self.config, &target.id, &error))
                }
            };
            (target.id, values)
        }))
        .buffer_unordered(CONTAINER_POLL_CONCURRENCY)
        .collect::<Vec<_>>()
        .await;

        for (target_id, values) in target_values {
            // Instance health describes the daemon connection, not the
            // least-healthy container. A deliberately stopped container is a
            // valid sub-target state; its target-scoped `status` detail tells
            // the placement without making the whole Docker host appear Down.
            if let Value::Object(values) = values {
                for (id, value) in values {
                    set_detail(&mut status.details, Some(&target_id), &id, value);
                }
            }
        }

        status.last_checked = Utc::now();
        Ok(status)
    }

    async fn test_connection(&self) -> ConnectionTestResult {
        if let Err(error) = self.docker.ping().await {
            return ConnectionTestResult {
                reachable: false,
                capabilities: Vec::new(),
                message: Some(format!(
                    "could not reach the Docker host at {}: {error}",
                    self.config.docker_host
                )),
            };
        }
        if let Err(error) = self.docker.version().await {
            return ConnectionTestResult {
                reachable: false,
                capabilities: Vec::new(),
                message: Some(format!(
                    "Docker answered ping but its version endpoint failed: {error}"
                )),
            };
        }

        if self.config.docker_host.starts_with("unix://") {
            return ConnectionTestResult {
                reachable: true,
                capabilities: vec![
                    available_capability(CAPABILITY_LIST_CONTAINERS, "List containers"),
                    available_capability(CAPABILITY_READ_LOGS, "Read container logs"),
                    available_capability(CAPABILITY_START, "Start containers"),
                    available_capability(CAPABILITY_STOP, "Stop containers"),
                    available_capability(CAPABILITY_RESTART, "Restart containers"),
                    available_capability(CAPABILITY_PAUSE, "Pause containers"),
                    available_capability(CAPABILITY_UNPAUSE, "Resume containers"),
                    available_capability(CAPABILITY_HOST_SUMMARY, "View host summary"),
                    available_capability(CAPABILITY_LIST_IMAGES, "Browse images"),
                    available_capability(CAPABILITY_PULL_IMAGE, "Pull images"),
                    available_capability(CAPABILITY_DELETE_IMAGE, "Delete images"),
                    available_capability(CAPABILITY_PRUNE_IMAGES, "Prune unused images"),
                    available_capability(CAPABILITY_LIST_VOLUMES, "Browse volumes"),
                    available_capability(CAPABILITY_CREATE_VOLUME, "Create volumes"),
                    available_capability(CAPABILITY_DELETE_VOLUME, "Delete volumes"),
                    available_capability(CAPABILITY_LIST_NETWORKS, "Browse networks"),
                    available_capability(CAPABILITY_CREATE_NETWORK, "Create networks"),
                    available_capability(CAPABILITY_DELETE_NETWORK, "Delete networks"),
                    available_capability(CAPABILITY_LIST_UPDATES, "Check for container updates"),
                    available_capability(CAPABILITY_APPLY_UPDATE, "Apply container updates"),
                ],
                message: Some(
                    "The raw Docker socket is reachable and grants unrestricted Docker API access."
                        .to_owned(),
                ),
            };
        }

        let list_options = ListContainersOptionsBuilder::new().all(true).build();
        let image_options = ListImagesOptionsBuilder::new().all(false).build();
        let (containers, logs, info, system, images, volumes, networks) = tokio::join!(
            self.docker.list_containers(Some(list_options)),
            self.probe_logs_route(),
            self.docker.info(),
            self.docker.df(None),
            // The same three listings the resource browser makes, so what the
            // test reports and what the tables will do cannot disagree.
            self.docker.list_images(Some(image_options)),
            self.docker.list_volumes(None::<ListVolumesOptions>),
            self.docker.list_networks(None::<ListNetworksOptions>),
        );
        let containers = containers.map(|_| ());
        let info = info.map(|_| ());
        let system = system.map(|_| ());
        let images = images.map(|_| ());
        let volumes = volumes.map(|_| ());
        let networks = networks.map(|_| ());
        // Update checking reads a container and its local image before it asks
        // a registry anything, so it is available exactly when both of those
        // probes were — no extra request to decide it.
        let updates = combine_read_probes(
            CAPABILITY_LIST_UPDATES,
            "Check for container updates",
            "CONTAINERS and IMAGES",
            &[&containers, &images],
        );

        ConnectionTestResult {
            reachable: true,
            capabilities: vec![
                proxy_read_capability(
                    CAPABILITY_LIST_CONTAINERS,
                    "List containers",
                    "CONTAINERS",
                    containers,
                ),
                proxy_read_capability(
                    CAPABILITY_READ_LOGS,
                    "Read container logs",
                    "CONTAINERS and ALLOW_LOGS",
                    logs,
                ),
                write_capability(
                    CAPABILITY_START,
                    "Start containers",
                    "CONTAINERS and ALLOW_START",
                ),
                write_capability(
                    CAPABILITY_STOP,
                    "Stop containers",
                    "CONTAINERS and ALLOW_STOP",
                ),
                write_capability(
                    CAPABILITY_RESTART,
                    "Restart containers",
                    "CONTAINERS and ALLOW_RESTARTS",
                ),
                write_capability(
                    CAPABILITY_PAUSE,
                    "Pause containers",
                    "CONTAINERS and ALLOW_PAUSE",
                ),
                write_capability(
                    CAPABILITY_UNPAUSE,
                    "Resume containers",
                    "CONTAINERS and ALLOW_UNPAUSE",
                ),
                combine_read_probes(
                    CAPABILITY_HOST_SUMMARY,
                    "View host summary",
                    "INFO and SYSTEM",
                    &[&info, &system],
                ),
                proxy_read_capability(CAPABILITY_LIST_IMAGES, "Browse images", "IMAGES", images),
                // Writes stay declarative, per the rule the lifecycle actions
                // already follow: the only way to prove a delete is permitted
                // is to delete something. The note names both gates, because
                // the category toggle on its own is not enough — POST is this
                // proxy's only method gate and it covers DELETE too.
                write_capability(CAPABILITY_PULL_IMAGE, "Pull images", "IMAGES and POST"),
                write_capability(CAPABILITY_DELETE_IMAGE, "Delete images", "IMAGES and POST"),
                write_capability(
                    CAPABILITY_PRUNE_IMAGES,
                    "Prune unused images",
                    "IMAGES and POST",
                ),
                proxy_read_capability(
                    CAPABILITY_LIST_VOLUMES,
                    "Browse volumes",
                    "VOLUMES",
                    volumes,
                ),
                write_capability(CAPABILITY_CREATE_VOLUME, "Create volumes", "VOLUMES and POST"),
                write_capability(CAPABILITY_DELETE_VOLUME, "Delete volumes", "VOLUMES and POST"),
                proxy_read_capability(
                    CAPABILITY_LIST_NETWORKS,
                    "Browse networks",
                    "NETWORKS",
                    networks,
                ),
                write_capability(
                    CAPABILITY_CREATE_NETWORK,
                    "Create networks",
                    "NETWORKS and POST",
                ),
                write_capability(
                    CAPABILITY_DELETE_NETWORK,
                    "Delete networks",
                    "NETWORKS and POST",
                ),
                updates,
                write_capability(
                    CAPABILITY_APPLY_UPDATE,
                    "Apply container updates",
                    "CONTAINERS, IMAGES and POST",
                ),
            ],
            message: Some(
                "Docker is reachable through TCP. Read capabilities were probed; write capabilities were not exercised."
                    .to_owned(),
            ),
        }
    }

    async fn actions(&self) -> Vec<ConnectorAction> {
        // Offered unconditionally rather than filtered by current state. The
        // list is cached by clients and the state can change between the two
        // calls, so a start button that vanished the instant a container came
        // up would only ever be *stale*, never correct — Docker's own refusal
        // is the authoritative answer, and it arrives as a clear message.
        let Ok(targets) = self.list_sub_targets_live().await else {
            return Vec::new();
        };
        targets
            .into_iter()
            .flat_map(|target| container_actions(&target.id))
            .collect()
    }

    async fn execute_action(
        &self,
        action_id: &str,
        target_id: Option<&str>,
        params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        // The one host-scoped action: "update everything that is behind" is a
        // question about the host, not about any one container.
        if action_id == ACTION_UPDATE_ALL {
            return self.update_all().await;
        }

        // The host inventory's actions are host-scoped too: an image, a volume
        // and a network belong to the daemon, not to any one container, so they
        // are routed before `target_id` is required below.
        if crate::resources::owns_action(action_id) {
            return crate::resources::execute(
                &self.control,
                self.registry.as_deref(),
                action_id,
                &params,
            )
            .await;
        }

        let Some(target_id) = target_id else {
            return Err(ConnectorError::invalid_action(action_id));
        };

        if action_id == ACTION_APPLY_UPDATE {
            let target_image_ref = params
                .get("targetImageRef")
                .and_then(Value::as_str)
                .ok_or_else(|| ConnectorError::InvalidParams {
                    action_id: action_id.to_owned(),
                    reason: "expected a non-empty string `targetImageRef`".to_owned(),
                })?;
            let result = apply_update(&self.control, target_id, target_image_ref).await?;
            if result.success {
                // The cached reading described the container that has just been
                // replaced. Dropping it is more honest than leaving a stale
                // "update available" row pointing at an image that is now
                // running; the next scheduled check refills it.
                self.update_cache
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .remove(target_id);
            }
            return Ok(result);
        }

        // The lifecycle actions are parameterless, so `params` is ignored
        // rather than validated for them.
        self.run_lifecycle(action_id, target_id).await
    }

    /// Only for a container, never for the host.
    ///
    /// A host runs no single image, so there is no version of "is this out of
    /// date?" that has one answer. Saying `false` at the host level is not a
    /// limitation being admitted — it is the accurate answer to a question that
    /// does not apply, and it keeps a client from offering a control that could
    /// only ever report nothing.
    fn supports_update_checking(&self) -> bool {
        true
    }

    async fn check_for_updates(
        &self,
        target_id: Option<&str>,
    ) -> Result<UpdateCheckResult, ConnectorError> {
        let Some(container_name) = target_id else {
            return Ok(UpdateCheckResult::up_to_date());
        };
        let Some(registry) = self.registry.as_ref() else {
            return Err(ConnectorError::Internal(
                "no HTTPS client is available, so registries cannot be queried".to_owned(),
            ));
        };

        let reading = check_container(&self.docker, registry.as_ref(), container_name).await?;
        let result = reading.as_result();
        self.update_cache
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(container_name.to_owned(), reading);
        Ok(result)
    }

    /// One browsable table: the containers with an update waiting.
    ///
    /// Host-scoped, because the interesting view is *across* containers — "what
    /// on this host is behind?" — and a per-container table with at most one
    /// row in it would be a worse answer to a question nobody asked.
    ///
    /// The row action is [`ACTION_APPLY_UPDATE`] with no `targetImageRef`
    /// filled in: the descriptor says what the action needs, and the caller
    /// fills it from the row it is acting on. The kind action applies every
    /// waiting update in turn.
    fn resource_kinds(&self) -> Vec<ResourceKindDescriptor> {
        let mut kinds = vec![ResourceKindDescriptor::new(
            RESOURCE_KIND_UPDATES,
            "Updates available",
            vec![
                // Keyed `targetId`, not `container`: a row in a host-scoped table
                // has to be able to say which sub-target its actions address,
                // and this is the platform's name for that.
                ColumnDescriptor::new("targetId", "Container", ColumnValueType::Text),
                ColumnDescriptor::new("currentRef", "Running", ColumnValueType::Text),
                // Named after `applyUpdate`'s parameter, not after the concept:
                // a client that sees a column key matching a parameter can answer
                // that parameter from the row, which turns "apply this one" into
                // a single click instead of a dialog asking for a value printed
                // in the cell beside the button.
                ColumnDescriptor::new("targetImageRef", "Available", ColumnValueType::Text),
                ColumnDescriptor::new("checkedAt", "Checked", ColumnValueType::Timestamp),
            ],
        )
        .with_row_actions(vec![apply_update_action()])
        .with_kind_actions(vec![ConnectorAction {
            id: ACTION_UPDATE_ALL.to_owned(),
            target_id: None,
            label: "Update all".to_owned(),
            description: Some(
                "Recreate every container with a waiting update, one after another.".to_owned(),
            ),
            params_schema: json!({ "type": "object", "additionalProperties": false }),
            is_disruptive: true,
            snapshot_data_point_ids: Vec::new(),
        }])
        // Declared rather than left to be inferred from the fact that the rows
        // happen to be containers: "what on this host is behind?" has no
        // per-container version, and a client can now leave the tab out of a
        // container's own view instead of showing one that will never fill.
        .applicable_to(ApplicableTarget::HostOnly)];
        kinds.extend(crate::resources::resource_kinds());
        kinds
    }

    async fn list_resource_items(
        &self,
        kind: &str,
        _target_id: Option<&str>,
    ) -> Result<Vec<ResourceItem>, ConnectorError> {
        // The three inventory tables all want the same container listing for
        // their "used by" column, so it is read once here and handed down.
        match kind {
            crate::resources::RESOURCE_KIND_IMAGES => {
                let usage = crate::resources::usage(&self.docker).await;
                return crate::resources::list_images(&self.docker, &usage).await;
            }
            crate::resources::RESOURCE_KIND_VOLUMES => {
                let usage = crate::resources::usage(&self.docker).await;
                return crate::resources::list_volumes(&self.docker, &usage).await;
            }
            crate::resources::RESOURCE_KIND_NETWORKS => {
                let usage = crate::resources::usage(&self.docker).await;
                return crate::resources::list_networks(&self.docker, &usage).await;
            }
            _ => {}
        }

        if kind != RESOURCE_KIND_UPDATES {
            return Ok(Vec::new());
        }

        Ok(self
            .outdated_containers()
            .into_iter()
            .map(|(name, reading)| {
                ResourceItem::new(name.clone())
                    .with_field("targetId", name)
                    .with_field("currentRef", reading.current_ref)
                    .with_field(
                        "targetImageRef",
                        reading.latest_ref.unwrap_or_else(|| "unknown".to_owned()),
                    )
                    .with_field("checkedAt", reading.checked_at.to_rfc3339())
            })
            .collect())
    }

    fn supports_sub_targets(&self) -> bool {
        true
    }

    async fn list_sub_targets(&self) -> Result<Vec<SubTarget>, ConnectorError> {
        self.list_sub_targets_live().await
    }

    fn config_schema(&self) -> Value {
        config_schema()
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
            min_size: (2, 2),
        }
    }

    /// Identity only.
    ///
    /// Written out by hand, as the trait requires: the configuration is the
    /// only other thing available and deriving from it is how a credential
    /// eventually reaches a dashboard. Neither of these two fields is secret —
    /// but that is a property of *this* connector's schema today, not a rule
    /// that survives the next field somebody adds.
    fn display_fields(&self) -> Vec<DisplayField> {
        vec![DisplayField::new(
            "Docker host",
            self.config.docker_host.clone(),
        )]
    }

    fn data_points(&self) -> Vec<DataPointDescriptor> {
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
                    .flat_map(|target| container_data_points(&target.id)),
            )
            .collect()
    }

    /// The Docker endpoint, when probing it would mean anything.
    ///
    /// A `tcp://host:port` endpoint — a remote daemon or a socket proxy — is
    /// worth probing: if it stops answering, whether the name resolves and
    /// whether the port accepts a connection are three different problems with
    /// three different fixes.
    ///
    /// A `unix://` socket is not. It is a file on this machine, reached over no
    /// network; a DNS lookup and a TCP connect have nothing to say about it,
    /// and "the host is reachable" would be a tautology dressed up as a
    /// diagnosis. `connect()` already reports a missing socket precisely, which
    /// is the actual failure in that setup.
    fn network_target(&self) -> Option<NetworkTarget> {
        let host = self
            .config
            .docker_host
            .strip_prefix("tcp://")
            .or_else(|| self.config.docker_host.strip_prefix("http://"))?;

        // Trim anything after the authority: a Docker URI does not normally
        // carry a path, but splitting here means one that does cannot turn into
        // a hostname with a slash in it.
        let authority = host.split(['/', '?']).next().unwrap_or(host);

        // `rsplit_once` rather than `split_once`, so an IPv6 literal in
        // brackets keeps its colons and only the trailing port is taken.
        match authority.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() => match port.parse::<u16>() {
                Ok(port) => Some(NetworkTarget::new(host.trim_matches(['[', ']']), port)),
                // A host with an unparseable port is still a host worth naming;
                // the probe stops after DNS, which is better than nothing.
                Err(_) => Some(NetworkTarget {
                    host: host.trim_matches(['[', ']']).to_owned(),
                    port: None,
                }),
            },
            // Docker over TCP with no port is unusual but legal — the daemon's
            // default is implied. Report the host so DNS is still checked.
            _ if !authority.is_empty() => Some(NetworkTarget {
                host: authority.trim_matches(['[', ']']).to_owned(),
                port: None,
            }),
            _ => None,
        }
    }

    fn default_layout(&self) -> WidgetLayout {
        WidgetLayout::new(vec![
            WidgetBinding::display(DATA_POINT_TOTAL_CONTAINERS, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_RUNNING_CONTAINERS, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_STOPPED_CONTAINERS, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_TOTAL_IMAGES, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_DISK_USAGE_BYTES, DisplayWidgetType::StatTile),
            WidgetBinding::display(
                DATA_POINT_IMAGE_DISK_USAGE_BYTES,
                DisplayWidgetType::StatTile,
            ),
            WidgetBinding::display(DATA_POINT_DOCKER_VERSION, DisplayWidgetType::StatTile),
        ])
    }

    fn default_layout_for(&self, target_id: Option<&str>) -> WidgetLayout {
        let Some(_target_id) = target_id else {
            return self.default_layout();
        };

        WidgetLayout::new(vec![
            WidgetBinding::display(DATA_POINT_STATUS, DisplayWidgetType::StatusDot),
            WidgetBinding::display(
                DATA_POINT_CPU_HISTORY,
                DisplayWidgetType::MetricChart {
                    chart_type: ChartType::Line,
                },
            ),
            WidgetBinding::display(DATA_POINT_MEMORY_USAGE_BYTES, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_LOGS, DisplayWidgetType::LogStream),
            WidgetBinding::action(ACTION_START, ActionWidgetType::Button),
            WidgetBinding::action(ACTION_STOP, ActionWidgetType::Button),
            WidgetBinding::action(ACTION_RESTART, ActionWidgetType::Button),
            // `pause` and `unpause` are deliberately not here. They are real
            // actions and stay in `actions()`, so anyone who wants them adds a
            // button in the binding editor — but a default layout with five
            // lifecycle buttons is a wall of controls for two that most people
            // never press.
        ])
    }
}

fn host_data_points() -> Vec<DataPointDescriptor> {
    vec![
        DataPointDescriptor::new(
            DATA_POINT_TOTAL_CONTAINERS,
            "Containers",
            DataPointValueType::Number,
        ),
        DataPointDescriptor::new(
            DATA_POINT_RUNNING_CONTAINERS,
            "Running containers",
            DataPointValueType::Number,
        ),
        DataPointDescriptor::new(
            DATA_POINT_STOPPED_CONTAINERS,
            "Stopped containers",
            DataPointValueType::Number,
        ),
        DataPointDescriptor::new(
            DATA_POINT_TOTAL_IMAGES,
            "Images",
            DataPointValueType::Number,
        ),
        DataPointDescriptor::new(
            DATA_POINT_DISK_USAGE_BYTES,
            "Docker disk usage",
            DataPointValueType::Number,
        )
        .with_unit("bytes"),
        DataPointDescriptor::new(
            DATA_POINT_IMAGE_DISK_USAGE_BYTES,
            "Image storage",
            DataPointValueType::Number,
        )
        .with_unit("bytes"),
        DataPointDescriptor::new(
            DATA_POINT_DOCKER_VERSION,
            "Docker version",
            DataPointValueType::String,
        ),
    ]
}

fn container_data_points(target_id: &str) -> Vec<DataPointDescriptor> {
    vec![
        DataPointDescriptor::new(DATA_POINT_STATUS, "State", DataPointValueType::String)
            .for_target(target_id),
        DataPointDescriptor::new(DATA_POINT_CPU_PERCENT, "CPU", DataPointValueType::Number)
            .with_unit("%")
            .for_target(target_id),
        DataPointDescriptor::new(
            DATA_POINT_CPU_HISTORY,
            "CPU history",
            DataPointValueType::TimeSeries,
        )
        .with_unit("%")
        .for_target(target_id),
        DataPointDescriptor::new(
            DATA_POINT_MEMORY_USAGE_BYTES,
            "Memory",
            DataPointValueType::Number,
        )
        .with_unit("bytes")
        .for_target(target_id),
        DataPointDescriptor::new(
            DATA_POINT_MEMORY_HISTORY,
            "Memory history",
            DataPointValueType::TimeSeries,
        )
        .with_unit("bytes")
        .for_target(target_id),
        DataPointDescriptor::new(DATA_POINT_UPTIME, "Uptime", DataPointValueType::String)
            .for_target(target_id),
        DataPointDescriptor::new(DATA_POINT_IMAGE_REF, "Image", DataPointValueType::String)
            .for_target(target_id),
        DataPointDescriptor::new(DATA_POINT_LOGS, "Recent logs", DataPointValueType::String)
            .for_target(target_id),
    ]
}

/// The image-replacing action, as a descriptor.
///
/// `snapshot_data_point_ids` is the whole rollback story. Naming
/// [`DATA_POINT_IMAGE_REF`] here makes the platform record what the container
/// was running immediately before the action, on the action-log entry, without
/// this connector storing anything: a later rollback is the same action invoked
/// with that recorded value. See `docs/adr/0022-action-log-and-update-checking.md`.
fn apply_update_action() -> ConnectorAction {
    ConnectorAction {
        id: ACTION_APPLY_UPDATE.to_owned(),
        target_id: None,
        label: "Apply update".to_owned(),
        description: Some(
            "Pull the named image and recreate this container on it, keeping its \
             configuration. Also the way back: pass the reference the container ran before."
                .to_owned(),
        ),
        params_schema: json!({
            "type": "object",
            "properties": {
                "targetImageRef": {
                    "type": "string",
                    "minLength": 1,
                    "description": "Image reference to recreate the container from, such as \
                                    `example/app:2.0`."
                }
            },
            "required": ["targetImageRef"],
            "additionalProperties": false
        }),
        is_disruptive: true,
        snapshot_data_point_ids: vec![DATA_POINT_IMAGE_REF.to_owned()],
    }
}

fn container_actions(target_id: &str) -> Vec<ConnectorAction> {
    vec![
        ConnectorAction::simple(ACTION_START, "Start")
            .with_description("Start the container.")
            .for_target(target_id),
        ConnectorAction::simple(ACTION_STOP, "Stop")
            .with_description("Stop the container, giving it time to shut down.")
            .for_target(target_id),
        // The only disruptive one. `start`, `stop`, `pause` and `unpause`
        // move to the requested stable state; restart is the temporary
        // disappearance the operation overlay explains.
        ConnectorAction::simple(ACTION_RESTART, "Restart")
            .with_description("Stop and start the container.")
            .disruptive()
            .for_target(target_id),
        ConnectorAction::simple(ACTION_PAUSE, "Pause")
            .with_description("Freeze every process in the container without stopping it.")
            .for_target(target_id),
        ConnectorAction::simple(ACTION_UNPAUSE, "Resume")
            .with_description("Resume a paused container.")
            .for_target(target_id),
        apply_update_action().for_target(target_id),
    ]
}

impl DockerConnector {
    /// Reads the daemon-wide summary used by host mode.
    async fn host_status(&self) -> ConnectorStatus {
        let info = match self.docker.info().await {
            Ok(info) => info,
            Err(error) => {
                return ConnectorStatus::new(
                    HealthState::Down,
                    unavailable_host_details(&format!(
                        "reading Docker host information from {} failed: {error}",
                        self.config.docker_host
                    )),
                );
            }
        };
        let slow_details = self.host_details().await;
        let partial_errors = slow_details.errors;

        let health = if partial_errors.is_empty() {
            HealthState::Healthy
        } else {
            HealthState::Degraded
        };
        let mut details = Value::Object(Map::new());
        for (id, value) in [
            (
                DATA_POINT_TOTAL_CONTAINERS,
                json!(info.containers.unwrap_or_default()),
            ),
            (
                DATA_POINT_RUNNING_CONTAINERS,
                json!(info.containers_running.unwrap_or_default()),
            ),
            (
                DATA_POINT_STOPPED_CONTAINERS,
                json!(info.containers_stopped.unwrap_or_default()),
            ),
            (
                DATA_POINT_TOTAL_IMAGES,
                json!(info.images.unwrap_or_default()),
            ),
            (DATA_POINT_DISK_USAGE_BYTES, json!(slow_details.disk_usage)),
            (
                DATA_POINT_IMAGE_DISK_USAGE_BYTES,
                json!(slow_details.image_disk_usage),
            ),
            (DATA_POINT_DOCKER_VERSION, json!(slow_details.version)),
        ] {
            set_detail(&mut details, None, id, value);
        }
        if !partial_errors.is_empty() {
            set_detail(
                &mut details,
                None,
                "error",
                json!(partial_errors.join("; ")),
            );
        }

        ConnectorStatus::new(health, details)
    }

    async fn host_details(&self) -> CachedHostDetails {
        if let Some(cached) = self
            .host_details
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .as_ref()
            .filter(|cached| cached.refreshed_at.elapsed() < HOST_DETAILS_REFRESH_INTERVAL)
            .cloned()
        {
            return cached;
        }

        let (usage, version) = tokio::join!(self.docker.df(None), self.docker.version());
        let mut errors = Vec::new();
        // One `/system/df` read answers both numbers. Splitting the image share
        // out costs nothing here and would cost a second call to that
        // deliberately-rate-limited endpoint if it were read anywhere else.
        let (disk_usage, image_disk_usage) = match usage {
            Ok(usage) => {
                let images = usage
                    .image_usage
                    .as_ref()
                    .and_then(|value| value.total_size)
                    .unwrap_or(0);
                let total = [
                    Some(images),
                    usage.container_usage.and_then(|value| value.total_size),
                    usage.volume_usage.and_then(|value| value.total_size),
                    usage.build_cache_usage.and_then(|value| value.total_size),
                ]
                .into_iter()
                .flatten()
                .sum::<i64>();
                (total, images)
            }
            Err(error) => {
                errors.push(optional_host_read_failure("Docker disk usage", &error));
                (0, 0)
            }
        };
        let version = match version {
            Ok(version) => version.version.unwrap_or_else(|| "unknown".to_owned()),
            Err(error) => {
                errors.push(optional_host_read_failure("Docker version check", &error));
                "unavailable".to_owned()
            }
        };
        let refreshed = CachedHostDetails {
            disk_usage,
            image_disk_usage,
            version,
            errors,
            refreshed_at: Instant::now(),
        };
        *self
            .host_details
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(refreshed.clone());
        refreshed
    }

    /// Assembles the status from a successful inspect.
    ///
    /// Split out so the stats and logs calls happen only once the container is
    /// known to exist, and so this half is readable without the error plumbing.
    async fn status_from(
        &self,
        container_name: &str,
        inspect: ContainerInspectResponse,
    ) -> (HealthState, Value) {
        // Read before `state` is taken out of the response, so the snapshot
        // data point and the state come from the same inspect.
        let image_ref = configured_image_ref(&inspect).unwrap_or_else(|| "unknown".to_owned());
        let state = inspect.state.unwrap_or_default();
        let status_enum = state.status;
        let health = health_for_state(status_enum);
        let running = matches!(
            status_enum,
            Some(bollard::models::ContainerStateStatusEnum::RUNNING)
        );

        // Stats and logs are only meaningful for a container that is running,
        // and asking for them otherwise means waiting out the stats collection
        // interval to be told nothing. A stopped container's poll is therefore
        // also the fast one.
        let (current_cpu, memory) = if running {
            match self.sample_stats(container_name).await {
                Ok(Some(sample)) => (
                    sample.cpu_stats,
                    sample
                        .memory_stats
                        .and_then(|memory| memory.usage)
                        .unwrap_or(0) as f64,
                ),
                // A container that stopped between the inspect and the stats
                // call, or a stats read that failed. Neither is worth failing
                // the poll for: the state we already read is the headline.
                Ok(None) | Err(_) => (None, 0.0),
            }
        } else {
            (None, 0.0)
        };

        let now = Utc::now();
        // History is recorded only while running, so a stopped container leaves
        // a gap in its chart rather than a flat line at zero that looks like a
        // measurement.
        let (cpu, cpu_history, memory_history) = {
            let mut histories = self.history.lock().unwrap_or_else(|poisoned| {
                // A panic in another thread while holding this lock cannot
                // corrupt a ring buffer of numbers, so recovering is strictly
                // better than propagating the panic into every later poll.
                poisoned.into_inner()
            });
            let history = histories.entry(container_name.to_owned()).or_default();
            let cpu = cpu_percent(current_cpu.as_ref(), history.previous_cpu.as_ref());
            if running {
                history.previous_cpu = current_cpu;
                history.record(cpu, memory, now);
            } else {
                history.previous_cpu = None;
            }
            (
                cpu,
                serde_json::to_value(&history.cpu).unwrap_or(Value::Null),
                serde_json::to_value(&history.memory).unwrap_or(Value::Null),
            )
        };

        // Read whether or not the container is running: for a stopped one, the
        // last lines before it exited are usually the reason it exited, which
        // is the single most useful thing a dashboard can show at that moment.
        let logs = self.tail_logs(container_name).await;

        let mut details = Map::new();
        details.insert(
            DATA_POINT_STATUS.to_owned(),
            json!(status_enum.map_or_else(|| "unknown".to_owned(), |state| state.to_string())),
        );
        details.insert(DATA_POINT_CPU_PERCENT.to_owned(), json!(cpu));
        details.insert(DATA_POINT_CPU_HISTORY.to_owned(), cpu_history);
        details.insert(DATA_POINT_MEMORY_USAGE_BYTES.to_owned(), json!(memory));
        details.insert(DATA_POINT_MEMORY_HISTORY.to_owned(), memory_history);
        details.insert(
            DATA_POINT_UPTIME.to_owned(),
            json!(format_uptime(state.started_at.as_deref(), running, now)),
        );
        details.insert(DATA_POINT_LOGS.to_owned(), json!(logs));
        // Reported on every poll so the platform's pre-action snapshot has
        // something current to record — see `ACTION_APPLY_UPDATE`.
        details.insert(DATA_POINT_IMAGE_REF.to_owned(), json!(image_ref));

        (health, Value::Object(details))
    }
}

/// Every declared data point, at its "we could not find out" value, plus the
/// reason.
///
/// A layout keeps rendering through an outage this way, showing an unavailable
/// container rather than collapsing to loading skeletons.
fn unavailable_details(reason: &str) -> Value {
    json!({
        DATA_POINT_STATUS: "unavailable",
        DATA_POINT_CPU_PERCENT: 0.0,
        DATA_POINT_CPU_HISTORY: [],
        DATA_POINT_MEMORY_USAGE_BYTES: 0.0,
        DATA_POINT_MEMORY_HISTORY: [],
        DATA_POINT_UPTIME: "not running",
        DATA_POINT_LOGS: "",
        DATA_POINT_IMAGE_REF: "unknown",
        // Not a declared data point on purpose — see `status`.
        "error": reason,
    })
}

/// Host-mode values when a daemon-wide read could not be completed.
fn unavailable_host_details(reason: &str) -> Value {
    let mut details = Value::Object(Map::new());
    for (id, value) in [
        (DATA_POINT_TOTAL_CONTAINERS, json!(0)),
        (DATA_POINT_RUNNING_CONTAINERS, json!(0)),
        (DATA_POINT_STOPPED_CONTAINERS, json!(0)),
        (DATA_POINT_TOTAL_IMAGES, json!(0)),
        (DATA_POINT_DISK_USAGE_BYTES, json!(0)),
        (DATA_POINT_DOCKER_VERSION, json!("unavailable")),
        ("error", json!(reason)),
    ] {
        set_detail(&mut details, None, id, value);
    }
    details
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DockerConnectorConfig;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[derive(Clone, Copy)]
    struct MockProxyPermissions {
        containers: bool,
        logs: bool,
        info: bool,
        system: bool,
        images: bool,
        volumes: bool,
        networks: bool,
    }

    /// A proxy configured the way the guide's defaults leave it: containers and
    /// their logs, nothing from the host inventory.
    fn containers_only() -> MockProxyPermissions {
        MockProxyPermissions {
            containers: true,
            logs: true,
            info: false,
            system: false,
            images: false,
            volumes: false,
            networks: false,
        }
    }

    fn everything() -> MockProxyPermissions {
        MockProxyPermissions {
            containers: true,
            logs: true,
            info: true,
            system: true,
            images: true,
            volumes: true,
            networks: true,
        }
    }

    async fn mock_proxy(permissions: MockProxyPermissions) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock proxy");
        let address = listener.local_addr().expect("mock address");
        tokio::spawn(async move {
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    break;
                };
                tokio::spawn(async move {
                    let mut request = vec![0; 8192];
                    let Ok(read) = socket.read(&mut request).await else {
                        return;
                    };
                    let request = String::from_utf8_lossy(&request[..read]);
                    let path = request
                        .lines()
                        .next()
                        .and_then(|line| line.split_whitespace().nth(1))
                        .unwrap_or("/");

                    let (status, body) = if path.ends_with("/_ping") {
                        ("200 OK", "OK")
                    } else if path.ends_with("/version") {
                        ("200 OK", "{}")
                    } else if path.contains("/containers/json") {
                        if permissions.containers {
                            ("200 OK", "[]")
                        } else {
                            ("403 Forbidden", r#"{"message":"forbidden"}"#)
                        }
                    } else if path.contains("/logs") {
                        if permissions.logs {
                            ("404 Not Found", r#"{"message":"no such container"}"#)
                        } else {
                            ("403 Forbidden", r#"{"message":"forbidden"}"#)
                        }
                    } else if path.ends_with("/info") {
                        if permissions.info {
                            ("200 OK", "{}")
                        } else {
                            ("403 Forbidden", r#"{"message":"forbidden"}"#)
                        }
                    } else if path.ends_with("/system/df") {
                        if permissions.system {
                            ("200 OK", "{}")
                        } else {
                            ("403 Forbidden", r#"{"message":"forbidden"}"#)
                        }
                    } else if path.contains("/images/json") {
                        if permissions.images {
                            ("200 OK", "[]")
                        } else {
                            ("403 Forbidden", r#"{"message":"forbidden"}"#)
                        }
                    } else if path.contains("/volumes") {
                        if permissions.volumes {
                            ("200 OK", r#"{"Volumes":[]}"#)
                        } else {
                            ("403 Forbidden", r#"{"message":"forbidden"}"#)
                        }
                    } else if path.contains("/networks") {
                        if permissions.networks {
                            ("200 OK", "[]")
                        } else {
                            ("403 Forbidden", r#"{"message":"forbidden"}"#)
                        }
                    } else {
                        ("404 Not Found", r#"{"message":"not found"}"#)
                    };
                    let response = format!(
                        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        format!("tcp://{address}")
    }

    /// Builds a descriptive connector without touching Docker.
    fn detached(docker_host: &str, targets: &[&str]) -> DockerConnector {
        let config = DockerConnectorConfig {
            docker_host: docker_host.to_owned(),
            ..DockerConnectorConfig::default()
        };
        let docker = config.connect().expect("building a client does no I/O");
        DockerConnector {
            docker: docker.clone(),
            control: docker,
            config,
            history: Arc::new(Mutex::new(HashMap::new())),
            known_targets: Arc::new(Mutex::new(
                targets
                    .iter()
                    .map(|id| SubTarget {
                        id: (*id).to_owned(),
                        label: (*id).to_owned(),
                    })
                    .collect(),
            )),
            host_details: Arc::new(Mutex::new(None)),
            update_cache: Arc::new(Mutex::new(UpdateCache::new())),
            registry: None,
        }
    }

    #[test]
    fn a_tcp_endpoint_is_worth_probing() {
        assert_eq!(
            detached("tcp://docker-proxy.example:2375", &[]).network_target(),
            Some(NetworkTarget::new("docker-proxy.example", 2375))
        );
        assert_eq!(
            detached("http://192.0.2.10:2376", &[]).network_target(),
            Some(NetworkTarget::new("192.0.2.10", 2376))
        );
        // An IPv6 literal keeps its own colons; only the trailing port is taken.
        assert_eq!(
            detached("tcp://[2001:db8::1]:2375", &[]).network_target(),
            Some(NetworkTarget::new("2001:db8::1", 2375))
        );
    }

    #[test]
    fn a_local_socket_has_nothing_to_probe() {
        // A DNS lookup and a TCP connect have nothing to say about a file.
        assert_eq!(
            detached("unix:///var/run/docker.sock", &[]).network_target(),
            None
        );
    }

    #[test]
    fn a_host_without_a_usable_port_still_reports_its_host() {
        // DNS is still worth checking even when there is nowhere to connect.
        for host in ["tcp://docker.example", "tcp://docker.example:not-a-port"] {
            let target = detached(host, &[])
                .network_target()
                .unwrap_or_else(|| panic!("{host} names a host"));
            assert_eq!(target.host, "docker.example");
            assert_eq!(target.port, None, "{host} names no usable port");
        }
    }

    #[test]
    fn the_setup_guide_matches_the_verified_proxy_gates() {
        let guide = setup_guide();
        assert_eq!(guide.variants.len(), 2);
        let socket = &guide.variants[0];
        assert_eq!(socket.id, "socket");
        assert!(socket.toggles.is_empty());
        assert!(socket.description.contains("root-equivalent"));
        assert!(socket.description.contains(":ro"));

        let proxy = &guide.variants[1];
        assert_eq!(proxy.id, "proxy");
        let env_vars = proxy
            .toggles
            .iter()
            .map(|toggle| toggle.env_var.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            env_vars,
            vec![
                "PING",
                "VERSION",
                "CONTAINERS",
                "ALLOW_LOGS",
                "ALLOW_START",
                "ALLOW_STOP",
                "ALLOW_RESTARTS",
                "ALLOW_PAUSE",
                "ALLOW_UNPAUSE",
                "INFO",
                "SYSTEM",
                "IMAGES",
                "NETWORKS",
                "VOLUMES",
                "POST",
            ]
        );
        // Every toggle the guide offers has to appear in the compose snippet it
        // renders, or someone follows the instructions and gets a proxy that
        // denies the feature they just switched on. This is the assertion that
        // would have caught the images/volumes/networks gap when the resource
        // kinds were added — see `docs/adr/0025-capabilities-are-part-of-adding-a-feature.md`.
        for toggle in &proxy.toggles {
            assert!(
                proxy.template.contains(&format!(
                    "{}: \"{{{{{}}}}}\"",
                    toggle.env_var, toggle.env_var
                )),
                "{} is a toggle but is not in the rendered compose file",
                toggle.env_var
            );
        }
        assert!(proxy
            .template
            .contains("lscr.io/linuxserver/socket-proxy:latest"));
        assert!(proxy.template.contains("ALLOW_ARCHIVE: \"0\""));
        assert!(proxy.template.contains("ALLOW_EXPORT: \"0\""));
        assert!(proxy.template.contains("ALLOW_TOP: \"0\""));
        assert!(proxy.template.contains("internal: true"));
        assert!(!proxy.template.contains("ports:"));
        assert!(proxy.description.contains("CVE-2026-78122"));

        let requirements = proxy
            .capability_requirements
            .iter()
            .map(|requirement| {
                (
                    requirement.capability_key.as_str(),
                    requirement
                        .required_toggle_keys
                        .iter()
                        .map(String::as_str)
                        .collect::<Vec<_>>(),
                )
            })
            .collect::<Vec<_>>();
        assert_eq!(
            requirements,
            vec![
                (CAPABILITY_LIST_CONTAINERS, vec!["containers"]),
                (CAPABILITY_READ_LOGS, vec!["containers", "allowLogs"]),
                (CAPABILITY_START, vec!["containers", "allowStart"]),
                (CAPABILITY_STOP, vec!["containers", "allowStop"]),
                (CAPABILITY_RESTART, vec!["containers", "allowRestarts"]),
                (CAPABILITY_PAUSE, vec!["containers", "allowPause"]),
                (CAPABILITY_UNPAUSE, vec!["containers", "allowUnpause"]),
                (CAPABILITY_HOST_SUMMARY, vec!["info", "system", "version"]),
                (CAPABILITY_LIST_IMAGES, vec!["containers", "images"]),
                (CAPABILITY_PULL_IMAGE, vec!["containers", "images", "post"]),
                (
                    CAPABILITY_DELETE_IMAGE,
                    vec!["containers", "images", "post"]
                ),
                (
                    CAPABILITY_PRUNE_IMAGES,
                    vec!["containers", "images", "post"]
                ),
                (CAPABILITY_LIST_VOLUMES, vec!["volumes"]),
                (CAPABILITY_CREATE_VOLUME, vec!["volumes", "post"]),
                (CAPABILITY_DELETE_VOLUME, vec!["volumes", "post"]),
                (CAPABILITY_LIST_NETWORKS, vec!["networks"]),
                (CAPABILITY_CREATE_NETWORK, vec!["networks", "post"]),
                (CAPABILITY_DELETE_NETWORK, vec!["networks", "post"]),
                (CAPABILITY_LIST_UPDATES, vec!["containers", "images"]),
                (
                    CAPABILITY_APPLY_UPDATE,
                    vec!["containers", "images", "post"]
                ),
            ]
        );

        // Every requirement names toggles that exist, and every write
        // requirement includes `post`. The second is the empirical finding:
        // LinuxServer's only method gate is `deny unless METH_GET || POST`, so
        // a category toggle alone never permits a delete, a create or a pull.
        let toggle_keys = proxy
            .toggles
            .iter()
            .map(|toggle| toggle.key.as_str())
            .collect::<Vec<_>>();
        for requirement in &proxy.capability_requirements {
            for key in &requirement.required_toggle_keys {
                assert!(
                    toggle_keys.contains(&key.as_str()),
                    "{} requires `{key}`, which is not a toggle",
                    requirement.capability_key
                );
            }
            let is_write = matches!(
                requirement.capability_key.as_str(),
                CAPABILITY_PULL_IMAGE
                    | CAPABILITY_DELETE_IMAGE
                    | CAPABILITY_PRUNE_IMAGES
                    | CAPABILITY_CREATE_VOLUME
                    | CAPABILITY_DELETE_VOLUME
                    | CAPABILITY_CREATE_NETWORK
                    | CAPABILITY_DELETE_NETWORK
                    | CAPABILITY_APPLY_UPDATE
            );
            assert_eq!(
                is_write,
                requirement
                    .required_toggle_keys
                    .iter()
                    .any(|key| key == "post"),
                "{} disagrees with the verified POST gate",
                requirement.capability_key
            );
        }
    }

    /// Every action and resource kind this connector publishes has to be
    /// reachable through the proxy variant, which means every one of them needs
    /// a capability requirement. Nothing enforced this before, which is exactly
    /// how the images/volumes/networks kinds shipped with none.
    #[tokio::test]
    async fn every_declared_resource_kind_has_a_capability_requirement() {
        let connector = detached("tcp://docker-proxy.example:2375", &["web"]);
        let guide = setup_guide();
        let proxy = &guide.variants[1];
        let declared = proxy
            .capability_requirements
            .iter()
            .map(|requirement| requirement.capability_key.as_str())
            .collect::<Vec<_>>();

        // The mapping is deliberately written out rather than derived: a kind
        // added without a thought about the proxy should fail to compile here,
        // not silently inherit a neighbour's capability.
        let expected = [
            (RESOURCE_KIND_UPDATES, CAPABILITY_LIST_UPDATES),
            (
                crate::resources::RESOURCE_KIND_IMAGES,
                CAPABILITY_LIST_IMAGES,
            ),
            (
                crate::resources::RESOURCE_KIND_VOLUMES,
                CAPABILITY_LIST_VOLUMES,
            ),
            (
                crate::resources::RESOURCE_KIND_NETWORKS,
                CAPABILITY_LIST_NETWORKS,
            ),
        ];
        for kind in connector.resource_kinds() {
            let capability = expected
                .iter()
                .find(|(name, _)| *name == kind.kind)
                .unwrap_or_else(|| panic!("resource kind `{}` has no capability", kind.kind))
                .1;
            assert!(
                declared.contains(&capability),
                "`{}` maps to {capability}, which the proxy variant does not declare",
                kind.kind
            );
        }
    }

    /// Looks capabilities up by key rather than by position: the list grows
    /// every time a feature is added, and an index-based assertion silently
    /// starts checking a different capability when it does.
    fn capability<'a>(result: &'a ConnectionTestResult, key: &str) -> &'a CapabilityStatus {
        result
            .capabilities
            .iter()
            .find(|capability| capability.key == key)
            .unwrap_or_else(|| panic!("the test result never mentions {key}"))
    }

    #[tokio::test]
    async fn tcp_connection_test_live_probes_reads_but_never_claims_writes() {
        let host = mock_proxy(everything()).await;
        let result = detached(&host, &[]).test_connection().await;
        assert!(result.reachable);

        // Everything that can be proved by reading, was.
        for key in [
            CAPABILITY_LIST_CONTAINERS,
            CAPABILITY_READ_LOGS,
            CAPABILITY_HOST_SUMMARY,
            CAPABILITY_LIST_IMAGES,
            CAPABILITY_LIST_VOLUMES,
            CAPABILITY_LIST_NETWORKS,
            CAPABILITY_LIST_UPDATES,
        ] {
            assert!(
                capability(&result, key).available,
                "{key} should be probed available"
            );
        }

        // Everything that would take an action to prove, stays unproven — a
        // permitted-looking button that turns out to be denied is worse than
        // one the test declined to promise.
        for key in [
            CAPABILITY_START,
            CAPABILITY_STOP,
            CAPABILITY_RESTART,
            CAPABILITY_PAUSE,
            CAPABILITY_UNPAUSE,
            CAPABILITY_PULL_IMAGE,
            CAPABILITY_DELETE_IMAGE,
            CAPABILITY_PRUNE_IMAGES,
            CAPABILITY_CREATE_VOLUME,
            CAPABILITY_DELETE_VOLUME,
            CAPABILITY_CREATE_NETWORK,
            CAPABILITY_DELETE_NETWORK,
            CAPABILITY_APPLY_UPDATE,
        ] {
            let status = capability(&result, key);
            assert!(!status.available, "{key} must not be claimed");
            assert!(status
                .note
                .as_deref()
                .is_some_and(|note| note.contains("without performing an action")));
        }

        // Every declarative requirement has a live row to match against.
        let guide = setup_guide();
        for requirement in &guide.variants[1].capability_requirements {
            capability(&result, &requirement.capability_key);
        }
    }

    #[tokio::test]
    async fn tcp_connection_test_reports_each_restricted_read() {
        let host = mock_proxy(MockProxyPermissions {
            containers: false,
            logs: false,
            info: true,
            system: false,
            images: false,
            volumes: false,
            networks: false,
        })
        .await;
        let result = detached(&host, &[]).test_connection().await;
        assert!(result.reachable, "PING and VERSION remain available");

        // Each denied read names the toggle that would fix it, and names its
        // own — "check CONTAINERS" is useless advice for a denied volume list.
        for (key, expected_note) in [
            (CAPABILITY_LIST_CONTAINERS, "CONTAINERS"),
            (CAPABILITY_READ_LOGS, "ALLOW_LOGS"),
            (CAPABILITY_HOST_SUMMARY, "INFO and SYSTEM"),
            (CAPABILITY_LIST_IMAGES, "IMAGES"),
            (CAPABILITY_LIST_VOLUMES, "VOLUMES"),
            (CAPABILITY_LIST_NETWORKS, "NETWORKS"),
            (CAPABILITY_LIST_UPDATES, "CONTAINERS and IMAGES"),
        ] {
            let status = capability(&result, key);
            assert!(!status.available, "{key} should be denied");
            assert!(
                status
                    .note
                    .as_deref()
                    .is_some_and(|note| note.contains(expected_note)),
                "{key} should point at {expected_note}, said {:?}",
                status.note
            );
        }
    }

    /// The guide's own defaults — containers and logs, no host inventory —
    /// are the configuration most people will actually have, and the three new
    /// tables must say plainly which toggle they are waiting on.
    #[tokio::test]
    async fn a_containers_only_proxy_reports_the_inventory_tables_as_unavailable() {
        let host = mock_proxy(containers_only()).await;
        let result = detached(&host, &[]).test_connection().await;
        assert!(result.reachable);
        assert!(capability(&result, CAPABILITY_LIST_CONTAINERS).available);
        assert!(capability(&result, CAPABILITY_READ_LOGS).available);

        for (key, toggle) in [
            (CAPABILITY_LIST_IMAGES, "IMAGES"),
            (CAPABILITY_LIST_VOLUMES, "VOLUMES"),
            (CAPABILITY_LIST_NETWORKS, "NETWORKS"),
        ] {
            let status = capability(&result, key);
            assert!(!status.available, "{key} needs {toggle}, which is off");
            assert_eq!(
                status.note.as_deref(),
                Some(
                    format!("Proxy configuration does not permit this — check {toggle}.").as_str()
                )
            );
        }

        // Update checking is denied by the image half alone, and says so.
        let updates = capability(&result, CAPABILITY_LIST_UPDATES);
        assert!(!updates.available);
        assert!(updates
            .note
            .as_deref()
            .is_some_and(|note| note.contains("CONTAINERS and IMAGES")));
    }

    #[test]
    fn only_restart_is_marked_disruptive_and_every_action_is_targeted() {
        let actions = container_actions("web");
        assert!(actions
            .iter()
            .all(|action| action.target_id.as_deref() == Some("web")));
        let disruptive: Vec<String> = actions
            .into_iter()
            .filter(|action| action.is_disruptive)
            .map(|action| action.id)
            .collect();
        assert_eq!(
            disruptive,
            vec![ACTION_RESTART.to_owned(), ACTION_APPLY_UPDATE.to_owned()],
            "stop and start take the container where the user asked; restart and an \
             image replacement both take it away and bring it back on their own"
        );

        // The image replacement is also the one action that declares a
        // snapshot, and that declaration is the entire rollback mechanism.
        let apply = container_actions("web")
            .into_iter()
            .find(|action| action.id == ACTION_APPLY_UPDATE)
            .expect("applyUpdate must be offered per container");
        assert_eq!(
            apply.snapshot_data_point_ids,
            vec![DATA_POINT_IMAGE_REF.to_owned()]
        );
        assert_eq!(
            apply.params_schema["required"],
            json!(["targetImageRef"]),
            "the caller must name what to move to, in either direction"
        );
    }

    #[test]
    fn an_optional_timeout_explains_that_the_connector_still_works() {
        assert_eq!(
            optional_host_read_failure(
                "Docker disk usage",
                &bollard::errors::Error::RequestTimeoutError,
            ),
            "Docker disk usage timed out. Container status and actions remain available."
        );
    }

    #[tokio::test]
    async fn the_updates_table_lists_what_the_last_check_found() {
        let connector = detached("tcp://docker-proxy.example:2375", &["web", "db"]);

        let kinds = connector.resource_kinds();
        // The updates table plus the three host-inventory tables.
        assert_eq!(
            kinds
                .iter()
                .map(|kind| kind.kind.as_str())
                .collect::<Vec<_>>(),
            vec![
                RESOURCE_KIND_UPDATES,
                crate::resources::RESOURCE_KIND_IMAGES,
                crate::resources::RESOURCE_KIND_VOLUMES,
                crate::resources::RESOURCE_KIND_NETWORKS,
            ]
        );
        let updates = &kinds[0];
        assert_eq!(updates.kind, RESOURCE_KIND_UPDATES);
        // Host-scoped, declared rather than inferred: a container's own modal
        // must not offer a table that can only ever be about the whole host.
        assert!(kinds
            .iter()
            .all(|kind| kind.applicable_target == ApplicableTarget::HostOnly));
        assert_eq!(
            updates
                .columns
                .iter()
                .map(|column| (column.key.as_str(), column.value_type))
                .collect::<Vec<_>>(),
            vec![
                ("targetId", ColumnValueType::Text),
                ("currentRef", ColumnValueType::Text),
                ("targetImageRef", ColumnValueType::Text),
                ("checkedAt", ColumnValueType::Timestamp),
            ]
        );
        assert_eq!(updates.row_actions[0].id, ACTION_APPLY_UPDATE);
        assert_eq!(updates.kind_actions[0].id, ACTION_UPDATE_ALL);

        // Nothing checked yet is an empty table, not an error.
        assert!(connector
            .list_resource_items(RESOURCE_KIND_UPDATES, None)
            .await
            .expect("an unchecked host browses empty")
            .is_empty());

        let checked_at = Utc::now();
        {
            let mut cache = connector.update_cache.lock().unwrap();
            cache.insert(
                "web".to_owned(),
                crate::updates::UpdateReading {
                    current_ref: "example/app:1.0".to_owned(),
                    available: true,
                    latest_ref: Some("example/app@sha256:aaaa".to_owned()),
                    checked_at,
                },
            );
            // Up to date, and therefore not a row: the table is "what needs
            // attention", not "what was checked".
            cache.insert(
                "db".to_owned(),
                crate::updates::UpdateReading {
                    current_ref: "example/db:16".to_owned(),
                    available: false,
                    latest_ref: None,
                    checked_at,
                },
            );
        }

        let rows = connector
            .list_resource_items(RESOURCE_KIND_UPDATES, None)
            .await
            .expect("listing the updates table");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "web");
        assert_eq!(rows[0].fields["targetId"], json!("web"));
        assert_eq!(rows[0].fields["currentRef"], json!("example/app:1.0"));
        assert_eq!(
            rows[0].fields["targetImageRef"],
            json!("example/app@sha256:aaaa")
        );
        assert_eq!(rows[0].fields["checkedAt"], json!(checked_at.to_rfc3339()));

        // A kind this connector does not declare is empty rather than an error,
        // per the trait's contract.
        assert!(connector
            .list_resource_items("somethingThisConnectorHasNever", None)
            .await
            .expect("an unknown kind is not a failure")
            .is_empty());
    }

    #[tokio::test]
    async fn the_host_target_has_no_image_to_check() {
        let connector = detached("tcp://docker-proxy.example:2375", &["web"]);
        // Supported at the connector level — every container below it can be
        // checked — but the host itself runs no single image, so asking about
        // it answers "nothing available" rather than reaching for a registry.
        assert!(connector.supports_update_checking());
        assert_eq!(
            connector
                .check_for_updates(None)
                .await
                .expect("the host answers without a registry call"),
            UpdateCheckResult::up_to_date()
        );
    }

    #[test]
    fn host_and_container_descriptors_are_tagged_by_target() {
        let connector = detached("tcp://docker-proxy.example:2375", &["web", "db"]);
        assert!(connector.supports_sub_targets());
        assert_eq!(connector.metadata().id, TYPE_ID);
        assert_eq!(connector.metadata().min_size, (2, 2));

        let points = connector.data_points();
        assert_eq!(
            points
                .iter()
                .filter(|point| point.target_id.is_none())
                .count(),
            7
        );
        for target in ["web", "db"] {
            let targeted: Vec<_> = points
                .iter()
                .filter(|point| point.target_id.as_deref() == Some(target))
                .collect();
            assert_eq!(targeted.len(), 8);
        }
        assert_eq!(connector.default_layout_for(None).bindings.len(), 7);
        assert_eq!(connector.default_layout_for(Some("web")).bindings.len(), 7);
    }
}
