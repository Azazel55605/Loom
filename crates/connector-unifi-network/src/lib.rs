//! Official local UniFi Network Integration API connector.

mod client;
mod config;
mod connector;

pub use client::{UniFiNetworkClient, UniFiNetworkError};
pub use config::{config_schema, UniFiNetworkConfig};
pub use connector::{
    UniFiNetworkConnector, DATA_POINT_CLIENT_COUNT, DATA_POINT_DEVICE_COUNT,
    DATA_POINT_ONLINE_DEVICE_COUNT, DISPLAY_NAME, ICON, TYPE_ID,
};
