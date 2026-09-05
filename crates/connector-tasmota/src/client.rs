use std::{fmt, sync::Arc, time::Duration};

use reqwest::{StatusCode, Url};
use serde_json::Value;
use thiserror::Error;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const STATUS_COMMAND: &str = "Status 0";

/// Failures at the Tasmota HTTP command boundary.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum TasmotaError {
    #[error("could not connect to Tasmota: {0}")]
    ConnectionFailed(String),
    #[error("Tasmota authentication failed: {0}")]
    AuthFailed(String),
    #[error("Tasmota command API failed: {message}")]
    ApiError { message: String },
}

/// Optional energy-monitoring readings returned by compatible hardware.
#[derive(Debug, Clone, PartialEq)]
pub struct EnergyReading {
    pub power_watts: Option<f64>,
    pub voltage_volts: Option<f64>,
    pub current_amps: Option<f64>,
    pub today_kwh: Option<f64>,
    pub total_kwh: Option<f64>,
}

/// The useful subset of the combined `Status 0` response.
#[derive(Debug, Clone, PartialEq)]
pub struct TasmotaStatus {
    pub power_state: bool,
    pub wifi_signal_percent: f64,
    pub uptime: String,
    pub firmware_version: String,
    pub energy: Option<EnergyReading>,
}

/// HTTP client for one Tasmota device.
#[derive(Clone)]
pub struct TasmotaClient {
    base_url: Arc<str>,
    username: Option<Arc<str>>,
    password: Option<Arc<str>>,
    http: reqwest::Client,
}

impl fmt::Debug for TasmotaClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TasmotaClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl TasmotaClient {
    /// Constructs a client. The caller verifies it with [`Self::status`].
    pub fn connect(
        host: &str,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Self, TasmotaError> {
        let base_url = if host.starts_with("http://") {
            host.trim_end_matches('/').to_owned()
        } else {
            format!("http://{}", host.trim().trim_end_matches('/'))
        };
        Url::parse(&base_url).map_err(|error| {
            TasmotaError::ConnectionFailed(format!("the configured host is invalid: {error}"))
        })?;
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| TasmotaError::ConnectionFailed(error.to_string()))?;
        Ok(Self {
            base_url: Arc::from(base_url),
            username: username.map(Arc::from),
            password: password.map(Arc::from),
            http,
        })
    }

    /// Fetches the combined status groups in one HTTP command call.
    pub async fn status(&self) -> Result<TasmotaStatus, TasmotaError> {
        parse_status(self.command(STATUS_COMMAND).await?)
    }

    /// Sets the first power output and returns the state acknowledged by Tasmota.
    pub async fn set_power(&self, on: bool) -> Result<bool, TasmotaError> {
        let command = if on { "Power On" } else { "Power Off" };
        let response = self.command(command).await?;
        response
            .get("POWER")
            .or_else(|| response.get("POWER1"))
            .and_then(parse_power_value)
            .ok_or_else(|| TasmotaError::ApiError {
                message: format!("{command} returned no recognizable POWER state"),
            })
    }

    async fn command(&self, command: &str) -> Result<Value, TasmotaError> {
        let endpoint = self.command_url(command)?;
        let response = self
            .http
            .get(endpoint)
            .send()
            .await
            .map_err(|error| TasmotaError::ConnectionFailed(error.without_url().to_string()))?;
        let status = response.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(TasmotaError::AuthFailed(format!(
                "the device returned HTTP {}",
                status.as_u16()
            )));
        }
        if !status.is_success() {
            return Err(TasmotaError::ApiError {
                message: format!("the device returned HTTP {}", status.as_u16()),
            });
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| TasmotaError::ApiError {
                message: format!("could not read the response body: {error}"),
            })?;
        let value: Value =
            serde_json::from_slice(&bytes).map_err(|error| TasmotaError::ApiError {
                message: format!("the device returned malformed JSON: {error}"),
            })?;
        if let Some(warning) = value.get("WARNING").and_then(Value::as_str) {
            if warning.to_ascii_lowercase().contains("user")
                || warning.to_ascii_lowercase().contains("password")
            {
                return Err(TasmotaError::AuthFailed(warning.to_owned()));
            }
            return Err(TasmotaError::ApiError {
                message: warning.to_owned(),
            });
        }
        Ok(value)
    }

    fn command_url(&self, command: &str) -> Result<Url, TasmotaError> {
        let mut url = Url::parse(&format!("{}/cm", self.base_url)).map_err(|error| {
            TasmotaError::ConnectionFailed(format!("the configured host is invalid: {error}"))
        })?;
        {
            let mut query = url.query_pairs_mut();
            if let Some(username) = self.username.as_deref() {
                query.append_pair("user", username);
            }
            if let Some(password) = self.password.as_deref() {
                query.append_pair("password", password);
            }
            query.append_pair("cmnd", command);
        }
        Ok(url)
    }
}

pub(crate) fn parse_status(value: Value) -> Result<TasmotaStatus, TasmotaError> {
    let status = value.get("Status");
    let live = value.get("StatusSTS");
    let power_state = status
        .and_then(|status| status.get("Power"))
        .and_then(parse_power_value)
        .or_else(|| live.and_then(find_live_power))
        .ok_or_else(|| malformed("Status 0 returned no recognizable power state"))?;
    let wifi_signal_percent = live
        .and_then(|live| live.get("Wifi"))
        .and_then(|wifi| wifi.get("RSSI"))
        .and_then(number_value)
        .filter(|rssi| (0.0..=100.0).contains(rssi))
        .ok_or_else(|| malformed("Status 0 returned no valid StatusSTS.Wifi.RSSI"))?;
    let uptime = live
        .and_then(|live| live.get("Uptime"))
        .or_else(|| value.pointer("/StatusPRM/Uptime"))
        .and_then(Value::as_str)
        .filter(|uptime| !uptime.is_empty())
        .ok_or_else(|| malformed("Status 0 returned no uptime"))?
        .to_owned();
    let firmware_version = value
        .pointer("/StatusFWR/Version")
        .and_then(Value::as_str)
        .filter(|version| !version.is_empty())
        .ok_or_else(|| malformed("Status 0 returned no StatusFWR.Version"))?
        .to_owned();
    let energy = value.pointer("/StatusSNS/ENERGY").and_then(|energy| {
        energy.as_object().map(|_| EnergyReading {
            power_watts: energy.get("Power").and_then(number_value),
            voltage_volts: energy.get("Voltage").and_then(number_value),
            current_amps: energy.get("Current").and_then(number_value),
            today_kwh: energy.get("Today").and_then(number_value),
            total_kwh: energy.get("Total").and_then(number_value),
        })
    });

    Ok(TasmotaStatus {
        power_state,
        wifi_signal_percent,
        uptime,
        firmware_version,
        energy,
    })
}

fn find_live_power(live: &Value) -> Option<bool> {
    live.get("POWER")
        .or_else(|| live.get("POWER1"))
        .and_then(parse_power_value)
}

fn parse_power_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::Number(value) => value.as_u64().and_then(|value| match value {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        }),
        Value::String(value) => match value.to_ascii_uppercase().as_str() {
            "OFF" | "0" | "FALSE" => Some(false),
            "ON" | "1" | "TRUE" => Some(true),
            _ => None,
        },
        _ => None,
    }
}

fn number_value(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_array()?.first()?.as_f64())
        .filter(|value| value.is_finite())
}

fn malformed(message: impl Into<String>) -> TasmotaError {
    TasmotaError::ApiError {
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn command_url_uses_the_documented_path_and_encoded_query_auth() {
        let client = TasmotaClient::connect(
            "plug.example:8080",
            Some("admin"),
            Some("not-a-real password"),
        )
        .expect("client");
        let url = client.command_url("Power Toggle").expect("command URL");
        assert_eq!(url.path(), "/cm");
        let pairs = url.query_pairs().collect::<Vec<_>>();
        assert!(pairs
            .iter()
            .any(|(key, value)| key == "user" && value == "admin"));
        assert!(pairs
            .iter()
            .any(|(key, value)| key == "password" && value == "not-a-real password"));
        assert!(pairs
            .iter()
            .any(|(key, value)| key == "cmnd" && value == "Power Toggle"));
    }

    #[test]
    fn current_status_zero_shape_maps_power_wifi_firmware_uptime_and_energy() {
        let status = parse_status(json!({
            "Status": { "Power": "1" },
            "StatusPRM": { "Uptime": "2T03:04:05" },
            "StatusFWR": { "Version": "14.4.1(tasmota32)" },
            "StatusSNS": {
                "Time": "2026-01-01T12:00:00",
                "ENERGY": {
                    "Total": 92.413,
                    "Yesterday": 0.123,
                    "Today": 0.052,
                    "Power": 77,
                    "Voltage": 230,
                    "Current": 0.334
                }
            },
            "StatusSTS": {
                "Uptime": "2T03:04:05",
                "POWER1": "ON",
                "Wifi": { "RSSI": 78, "Signal": -61 }
            }
        }))
        .expect("documented response");

        assert!(status.power_state);
        assert_eq!(status.wifi_signal_percent, 78.0);
        assert_eq!(status.uptime, "2T03:04:05");
        assert_eq!(status.firmware_version, "14.4.1(tasmota32)");
        let energy = status.energy.expect("energy-capable plug");
        assert_eq!(energy.power_watts, Some(77.0));
        assert_eq!(energy.voltage_volts, Some(230.0));
        assert_eq!(energy.current_amps, Some(0.334));
        assert_eq!(energy.today_kwh, Some(0.052));
        assert_eq!(energy.total_kwh, Some(92.413));
    }

    #[test]
    fn a_non_metering_device_has_no_energy_reading_instead_of_fake_zeroes() {
        let status = parse_status(json!({
            "Status": { "Power": 0 },
            "StatusFWR": { "Version": "14.4.1" },
            "StatusSTS": {
                "Uptime": "0T01:02:03",
                "POWER": "OFF",
                "Wifi": { "RSSI": 55 }
            }
        }))
        .expect("non-metering response");

        assert!(!status.power_state);
        assert_eq!(status.energy, None);
    }

    #[test]
    fn energy_arrays_from_multi_channel_firmware_use_the_first_plug_channel() {
        let status = parse_status(json!({
            "Status": { "Power": 1 },
            "StatusFWR": { "Version": "14.4.1" },
            "StatusSNS": { "ENERGY": { "Power": [12, 4], "Current": [0.05, 0.02] } },
            "StatusSTS": { "Uptime": "0T00:01:00", "Wifi": { "RSSI": 80 } }
        }))
        .expect("multi-channel response");
        let energy = status.energy.expect("energy");
        assert_eq!(energy.power_watts, Some(12.0));
        assert_eq!(energy.current_amps, Some(0.05));
    }
}
