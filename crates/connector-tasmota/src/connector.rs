use async_trait::async_trait;
use loom_core::connector::{
    details::set_detail, ActionResult, ActionWidgetType, ConnectorAction, ConnectorError,
    ConnectorMetadata, ConnectorStatus, DataPointDescriptor, DataPointValueType, DisplayField,
    DisplayWidgetType, HealthState, NetworkTarget, WidgetBinding, WidgetLayout,
};
use serde_json::{json, Map, Value};

use crate::{client::EnergyReading, config::TasmotaConnectorConfig, TasmotaClient, TasmotaError};

pub const TYPE_ID: &str = "tasmota";
pub const DISPLAY_NAME: &str = "Tasmota";
pub const ICON: &str = "lucide:plug";

pub const DATA_POINT_POWER_STATE: &str = "powerState";
pub const DATA_POINT_WIFI_SIGNAL_PERCENT: &str = "wifiSignalPercent";
pub const DATA_POINT_UPTIME: &str = "uptime";
pub const DATA_POINT_FIRMWARE_VERSION: &str = "firmwareVersion";
pub const DATA_POINT_POWER_WATTS: &str = "powerWatts";
pub const DATA_POINT_VOLTAGE_VOLTS: &str = "voltageVolts";
pub const DATA_POINT_CURRENT_AMPS: &str = "currentAmps";
pub const DATA_POINT_ENERGY_TODAY_KWH: &str = "energyTodayKwh";
pub const ACTION_SET_POWER: &str = "setPower";

/// One configured Tasmota smart plug.
pub struct TasmotaConnector {
    config: TasmotaConnectorConfig,
    client: TasmotaClient,
    supports_energy: bool,
}

impl TasmotaConnector {
    /// Validates configuration and verifies the device with one combined status call.
    pub async fn from_config_value(value: Value) -> Result<Self, ConnectorError> {
        let config = TasmotaConnectorConfig::from_value(value)?;
        // Tasmota's compile-time WEB_USERNAME defaults to `admin`. The exposed
        // configuration therefore needs only the optional WebPassword; custom
        // firmware with a different username is outside this first connector pass.
        let username = config.password.as_ref().map(|_| "admin");
        let client = TasmotaClient::connect(&config.host, username, config.password.as_deref())
            .map_err(connector_error)?;
        let initial = client.status().await.map_err(connector_error)?;
        Ok(Self {
            config,
            client,
            supports_energy: initial.energy.is_some(),
        })
    }

    fn status_details(&self, status: crate::TasmotaStatus) -> Value {
        let mut details = Value::Object(Map::new());
        for (id, value) in [
            (DATA_POINT_POWER_STATE, json!(status.power_state)),
            (
                DATA_POINT_WIFI_SIGNAL_PERCENT,
                json!(status.wifi_signal_percent),
            ),
            (DATA_POINT_UPTIME, json!(status.uptime)),
            (DATA_POINT_FIRMWARE_VERSION, json!(status.firmware_version)),
        ] {
            set_detail(&mut details, None, id, value);
        }
        if let Some(energy) = status.energy {
            set_energy_details(&mut details, energy);
        }
        details
    }
}

#[async_trait]
impl loom_core::connector::Connector for TasmotaConnector {
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
        match self.client.status().await {
            Ok(status) => Ok(ConnectorStatus::new(
                HealthState::Healthy,
                self.status_details(status),
            )
            .with_target_health(String::new(), HealthState::Healthy)),
            Err(error) => {
                let mut details = Value::Object(Map::new());
                set_detail(&mut details, None, "error", json!(error.to_string()));
                Ok(ConnectorStatus::new(HealthState::Down, details)
                    .with_target_health(String::new(), HealthState::Down))
            }
        }
    }

    async fn actions(&self) -> Vec<ConnectorAction> {
        vec![ConnectorAction {
            id: ACTION_SET_POWER.to_owned(),
            target_id: None,
            label: "Set power".to_owned(),
            description: Some("Turn the smart plug's first power output on or off.".to_owned()),
            params_schema: json!({
                "type": "object",
                "properties": {
                    "on": { "type": "boolean" }
                },
                "required": ["on"],
                "additionalProperties": false
            }),
            is_disruptive: false,
            snapshot_data_point_ids: vec![DATA_POINT_POWER_STATE.to_owned()],
        }]
    }

    async fn execute_action(
        &self,
        action_id: &str,
        target_id: Option<&str>,
        params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        if target_id.is_some() || action_id != ACTION_SET_POWER {
            return Err(ConnectorError::invalid_action(action_id));
        }
        let on = params.get("on").and_then(Value::as_bool).ok_or_else(|| {
            ConnectorError::InvalidParams {
                action_id: ACTION_SET_POWER.to_owned(),
                reason: "`on` must be a boolean".to_owned(),
            }
        })?;
        let acknowledged = self.client.set_power(on).await.map_err(connector_error)?;
        if acknowledged != on {
            return Ok(ActionResult::failed(format!(
                "Tasmota acknowledged power as {} instead of {}",
                state_label(acknowledged),
                state_label(on)
            ))
            .with_payload(json!({ "powerState": acknowledged })));
        }
        Ok(
            ActionResult::ok(format!("Tasmota power turned {}", state_label(on)))
                .with_payload(json!({ "powerState": on })),
        )
    }

    fn config_schema(&self) -> Value {
        crate::config_schema()
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

    fn display_fields(&self) -> Vec<DisplayField> {
        vec![DisplayField::new("Tasmota host", self.config.host.clone())]
    }

    fn data_points(&self) -> Vec<DataPointDescriptor> {
        let mut points = vec![
            DataPointDescriptor::new(DATA_POINT_POWER_STATE, "Power", DataPointValueType::Bool),
            DataPointDescriptor::new(
                DATA_POINT_WIFI_SIGNAL_PERCENT,
                "Wi-Fi signal",
                DataPointValueType::Number,
            )
            .with_unit("%"),
            DataPointDescriptor::new(DATA_POINT_UPTIME, "Uptime", DataPointValueType::String),
            DataPointDescriptor::new(
                DATA_POINT_FIRMWARE_VERSION,
                "Firmware",
                DataPointValueType::String,
            ),
        ];
        if self.supports_energy {
            points.extend([
                DataPointDescriptor::new(
                    DATA_POINT_POWER_WATTS,
                    "Power draw",
                    DataPointValueType::Number,
                )
                .with_unit("W"),
                DataPointDescriptor::new(
                    DATA_POINT_VOLTAGE_VOLTS,
                    "Voltage",
                    DataPointValueType::Number,
                )
                .with_unit("V"),
                DataPointDescriptor::new(
                    DATA_POINT_CURRENT_AMPS,
                    "Current",
                    DataPointValueType::Number,
                )
                .with_unit("A"),
                DataPointDescriptor::new(
                    DATA_POINT_ENERGY_TODAY_KWH,
                    "Energy today",
                    DataPointValueType::Number,
                )
                .with_unit("kWh"),
            ]);
        }
        points
    }

    fn default_layout(&self) -> WidgetLayout {
        let mut bindings = vec![
            WidgetBinding::action(ACTION_SET_POWER, ActionWidgetType::Toggle).with_config(json!({
                "paramName": "on",
                "stateDataPointId": DATA_POINT_POWER_STATE
            })),
            WidgetBinding::display(DATA_POINT_WIFI_SIGNAL_PERCENT, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_UPTIME, DisplayWidgetType::StatTile),
        ];
        if self.supports_energy {
            bindings.extend([
                WidgetBinding::display(DATA_POINT_POWER_WATTS, DisplayWidgetType::StatTile),
                WidgetBinding::display(DATA_POINT_VOLTAGE_VOLTS, DisplayWidgetType::StatTile),
                WidgetBinding::display(DATA_POINT_CURRENT_AMPS, DisplayWidgetType::StatTile),
                WidgetBinding::display(DATA_POINT_ENERGY_TODAY_KWH, DisplayWidgetType::StatTile),
            ]);
        }
        WidgetLayout::new(bindings)
    }

    fn network_target(&self) -> Option<NetworkTarget> {
        self.config.network_target()
    }
}

fn set_energy_details(details: &mut Value, energy: EnergyReading) {
    for (id, value) in [
        (DATA_POINT_POWER_WATTS, energy.power_watts),
        (DATA_POINT_VOLTAGE_VOLTS, energy.voltage_volts),
        (DATA_POINT_CURRENT_AMPS, energy.current_amps),
        (DATA_POINT_ENERGY_TODAY_KWH, energy.today_kwh),
    ] {
        // A missing ENERGY member means the firmware/hardware did not report
        // it. Omit it so clients show "no data"; zero remains a real reading.
        if let Some(value) = value {
            set_detail(details, None, id, json!(value));
        }
    }
}

fn connector_error(error: TasmotaError) -> ConnectorError {
    match error {
        TasmotaError::ConnectionFailed(reason) => ConnectorError::unreachable(reason),
        TasmotaError::AuthFailed(reason) => ConnectorError::AuthFailed { reason },
        TasmotaError::ApiError { message } => {
            ConnectorError::Internal(format!("Tasmota command API failed: {message}"))
        }
    }
}

fn state_label(on: bool) -> &'static str {
    if on {
        "on"
    } else {
        "off"
    }
}

#[cfg(test)]
mod tests {
    use loom_core::connector::{Connector, WidgetBinding};

    use super::*;

    #[test]
    fn energy_details_keep_real_zeroes_and_omit_unreported_values() {
        let mut details = Value::Object(Map::new());
        set_energy_details(
            &mut details,
            EnergyReading {
                power_watts: Some(0.0),
                voltage_volts: Some(230.0),
                current_amps: None,
                today_kwh: Some(0.0),
                total_kwh: Some(12.0),
            },
        );
        assert_eq!(details[""][DATA_POINT_POWER_WATTS], json!(0.0));
        assert_eq!(details[""][DATA_POINT_VOLTAGE_VOLTS], json!(230.0));
        assert!(details[""]
            .as_object()
            .is_some_and(|values| !values.contains_key(DATA_POINT_CURRENT_AMPS)));
    }

    #[test]
    fn metering_layout_adds_only_declared_energy_bindings() {
        let config = TasmotaConnectorConfig {
            host: "plug.example".to_owned(),
            password: None,
        };
        let client = TasmotaClient::connect(&config.host, None, None).expect("client");
        let connector = TasmotaConnector {
            config,
            client,
            supports_energy: true,
        };
        let points = connector.data_points();
        let ids = points
            .iter()
            .map(|point| point.id.as_str())
            .collect::<Vec<_>>();
        assert!(ids.contains(&DATA_POINT_POWER_WATTS));
        assert!(connector
            .default_layout()
            .bindings
            .iter()
            .all(|binding| match binding {
                WidgetBinding::Display { data_point_id, .. } =>
                    ids.contains(&data_point_id.as_str()),
                WidgetBinding::Action { action_id, .. } => action_id == ACTION_SET_POWER,
                WidgetBinding::ResourceKindDisplay { .. } => false,
            }));
    }
}
