//! Pi-hole v6 REST connector: authenticated statistics and DNS blocking control.

mod client;
mod config;
mod connector;

pub use client::{PiHoleClient, PiHoleError};
pub use config::{config_schema, PiHoleConnectorConfig};
pub use connector::{
    PiHoleConnector, ACTION_SET_BLOCKING, DATA_POINT_BLOCKING_ENABLED, DATA_POINT_BLOCK_PERCENTAGE,
    DATA_POINT_DOMAINS_ON_BLOCKLIST, DATA_POINT_QUERIES_BLOCKED_TODAY, DATA_POINT_QUERIES_HISTORY,
    DATA_POINT_QUERIES_TODAY, DATA_POINT_UNIQUE_CLIENTS, DISPLAY_NAME, ICON, TYPE_ID,
};
