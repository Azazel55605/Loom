//! Tasmota smart-plug connector over the device's HTTP command API.

mod client;
mod config;
mod connector;

pub use client::{TasmotaClient, TasmotaError, TasmotaStatus};
pub use config::{config_schema, TasmotaConnectorConfig};
pub use connector::{
    TasmotaConnector, ACTION_SET_POWER, DATA_POINT_CURRENT_AMPS, DATA_POINT_ENERGY_TODAY_KWH,
    DATA_POINT_FIRMWARE_VERSION, DATA_POINT_POWER_STATE, DATA_POINT_POWER_WATTS, DATA_POINT_UPTIME,
    DATA_POINT_VOLTAGE_VOLTS, DATA_POINT_WIFI_SIGNAL_PERCENT, DISPLAY_NAME, ICON, TYPE_ID,
};
