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

pub use config::{config_schema, DockerConnectorConfig, DEFAULT_DOCKER_HOST};
pub use connector::{
    setup_guide, DockerConnector, ACTION_PAUSE, ACTION_RESTART, ACTION_START, ACTION_STOP,
    ACTION_UNPAUSE, DATA_POINT_CPU_HISTORY, DATA_POINT_CPU_PERCENT, DATA_POINT_DISK_USAGE_BYTES,
    DATA_POINT_DOCKER_VERSION, DATA_POINT_LOGS, DATA_POINT_MEMORY_HISTORY,
    DATA_POINT_MEMORY_USAGE_BYTES, DATA_POINT_RUNNING_CONTAINERS, DATA_POINT_STATUS,
    DATA_POINT_STOPPED_CONTAINERS, DATA_POINT_TOTAL_CONTAINERS, DATA_POINT_TOTAL_IMAGES,
    DATA_POINT_UPTIME, DISPLAY_NAME, HISTORY_CAPACITY, ICON, LOG_TAIL_LINES, TYPE_ID,
};
pub use metrics::{cpu_percent, format_uptime, health_for_state};
