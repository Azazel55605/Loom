//! Official local UniFi Network Integration API connector.

mod client;
mod config;
mod connector;

pub use client::{UniFiNetworkClient, UniFiNetworkError};
pub use config::{config_schema, UniFiNetworkConfig};
pub use connector::{
    UniFiNetworkConnector, ACTION_RESTART, DATA_POINT_CLIENT_COUNT, DATA_POINT_DEVICE_COUNT,
    DATA_POINT_MODEL, DATA_POINT_ONLINE_DEVICE_COUNT, DATA_POINT_STATE, DATA_POINT_UPTIME,
    DISPLAY_NAME, ICON, TYPE_ID,
};
