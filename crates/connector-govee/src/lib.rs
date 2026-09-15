//! Govee cloud connector using the capability-driven Router API.

mod client;
mod config;
mod connector;

pub use client::{GoveeCapability, GoveeClient, GoveeDevice, GoveeError};
pub use config::{config_schema, GoveeConnectorConfig};
pub use connector::{GoveeConnector, DISPLAY_NAME, ICON, TYPE_ID};
