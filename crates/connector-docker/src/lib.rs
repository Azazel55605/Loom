//! Loom's connector for a Docker daemon.
//!
//! One instance always represents one Docker connection. With no
//! `containerName` it reports daemon-wide counts, disk usage, and version and
//! can discover the daemon's containers. With an exact `containerName` it
//! preserves the existing container status, history, logs, and lifecycle
//! controls. Host and container are views over the same authority, not
//! separate integration types.
//!
//! # Permissions are not this crate's business
//!
//! As with every connector, nothing here asks *who* is calling. Whether a user
//! may restart a container is decided by `connectors.control` in
//! `crates/web-backend`; this crate executes what it is told. See
//! `crates/core/src/connector/mod.rs`.

mod config;
mod connector;
mod metrics;
mod registry;
mod resources;
mod updates;

pub use config::{
    config_schema, AutoApplyTime, DockerConnectorConfig, DEFAULT_CHECK_INTERVAL_MINUTES,
    DEFAULT_DOCKER_HOST, MIN_CHECK_INTERVAL_MINUTES,
};
pub use connector::{
    setup_guide, DockerConnector, ACTION_APPLY_UPDATE, ACTION_PAUSE, ACTION_RESTART, ACTION_START,
    ACTION_STOP, ACTION_UNPAUSE, ACTION_UPDATE_ALL, DATA_POINT_CPU_HISTORY, DATA_POINT_CPU_PERCENT,
    DATA_POINT_DISK_USAGE_BYTES, DATA_POINT_DOCKER_VERSION, DATA_POINT_IMAGE_REF, DATA_POINT_LOGS,
    DATA_POINT_MEMORY_HISTORY, DATA_POINT_MEMORY_USAGE_BYTES, DATA_POINT_RUNNING_CONTAINERS,
    DATA_POINT_STATUS, DATA_POINT_STOPPED_CONTAINERS, DATA_POINT_TOTAL_CONTAINERS,
    DATA_POINT_TOTAL_IMAGES, DATA_POINT_UPTIME, DISPLAY_NAME, HISTORY_CAPACITY, ICON,
    LOG_TAIL_LINES, RESOURCE_KIND_UPDATES, TYPE_ID,
};
pub use metrics::{cpu_percent, format_uptime, health_for_state};
pub use registry::{current_digest, http_registry, is_outdated, ImageReference, RegistryTransport};
pub use resources::{
    ACTION_CHECK_IMAGE_UPDATE, ACTION_CREATE_NETWORK, ACTION_CREATE_VOLUME, ACTION_DELETE_IMAGE,
    ACTION_DELETE_NETWORK, ACTION_DELETE_VOLUME, ACTION_PULL_IMAGE, RESOURCE_KIND_IMAGES,
    RESOURCE_KIND_NETWORKS, RESOURCE_KIND_VOLUMES,
};
pub use updates::{apply_update, check_container, recreate_body, UpdateReading};
