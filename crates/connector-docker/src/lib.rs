//! Loom's connector for a single Docker container.
//!
//! One instance watches and controls **one container**, named exactly, on one
//! Docker endpoint — a local socket or a `tcp://` host such as a
//! `docker-socket-proxy`. It reports the container's state, CPU and memory use
//! with rolling history, uptime and a log tail, and offers the five lifecycle
//! actions: start, stop, restart, pause, unpause.
//!
//! # Scope of this first version
//!
//! Monitoring and control, done properly. Not included, and tracked as
//! follow-ups rather than gaps to be discovered later:
//!
//! - **Discovery.** Listing a host's containers and proposing an instance per
//!   container belongs to a host-level connector; this one is scoped to a
//!   container it was told the name of.
//! - **A setup guide.** Worth writing once it can be capability-aware — the
//!   advice for a bind-mounted socket, a rootless daemon and a socket proxy
//!   are three different pieces of advice, and one static template that covers
//!   all three helps with none.
//!
//! Both keep their trait defaults rather than returning placeholder content.
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
    DockerConnector, ACTION_PAUSE, ACTION_RESTART, ACTION_START, ACTION_STOP, ACTION_UNPAUSE,
    DATA_POINT_CPU_HISTORY, DATA_POINT_CPU_PERCENT, DATA_POINT_LOGS, DATA_POINT_MEMORY_HISTORY,
    DATA_POINT_MEMORY_USAGE_BYTES, DATA_POINT_STATUS, DATA_POINT_UPTIME, DISPLAY_NAME,
    HISTORY_CAPACITY, ICON, LOG_TAIL_LINES, TYPE_ID,
};
pub use metrics::{cpu_percent, format_uptime, health_for_state};
