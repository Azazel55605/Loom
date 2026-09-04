//! Official local UniFi Network Integration API connector.

mod client;
mod config;
mod connector;

pub use client::{UniFiNetworkClient, UniFiNetworkError};
pub use config::{config_schema, UniFiNetworkConfig};
pub use connector::{
    UniFiNetworkConnector, ACTION_AUTHORIZE_GUEST, ACTION_CREATE_VOUCHER, ACTION_CYCLE_POE,
    ACTION_RESTART, ACTION_REVOKE_VOUCHER, DATA_POINT_CLIENT_COUNT, DATA_POINT_DEVICE_COUNT,
    DATA_POINT_MODEL, DATA_POINT_ONLINE_DEVICE_COUNT, DATA_POINT_STATE, DATA_POINT_UPTIME,
    DISPLAY_NAME, ICON, RESOURCE_KIND_CLIENTS, RESOURCE_KIND_PORTS, RESOURCE_KIND_VOUCHERS,
    TYPE_ID,
};
