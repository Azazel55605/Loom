use std::{
    collections::HashMap,
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use reqwest::{Method, StatusCode};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

const DEFAULT_BASE_URL: &str = "https://openapi.api.govee.com/router/api/v1";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// Errors preserved at the Govee HTTP boundary without ever including the key.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum GoveeError {
    #[error("could not connect to Govee: {0}")]
    ConnectionFailed(String),
    #[error("Govee authentication failed: {0}")]
    AuthFailed(String),
    #[error("Govee rate limit reached: {0}")]
    RateLimited(String),
    #[error("Govee API failed with code {code}: {message}")]
    ApiError { code: i64, message: String },
}

/// One device returned by `GET /user/devices`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct GoveeDevice {
    pub sku: String,
    pub device: String,
    #[serde(default)]
    pub device_name: String,
    #[serde(rename = "type", default)]
    pub device_type: String,
    #[serde(default)]
    pub capabilities: Vec<GoveeCapability>,
}

/// Capability descriptor carried by a Govee device.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct GoveeCapability {
    #[serde(rename = "type")]
    pub kind: String,
    pub instance: String,
    #[serde(default)]
    pub parameters: Value,
}

/// A capability and its current state.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct GoveeCapabilityState {
    #[serde(rename = "type")]
    pub kind: String,
    pub instance: String,
    pub state: GoveeStateValue,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct GoveeStateValue {
    pub value: Value,
}

/// A dynamic scene option. `value` is intentionally opaque and round-tripped.
#[derive(Debug, Clone, PartialEq)]
pub struct GoveeScene {
    pub name: String,
    pub capability_type: String,
    pub instance: String,
    pub value: Value,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    code: i64,
    #[serde(default, alias = "msg")]
    message: String,
    data: Option<T>,
    payload: Option<T>,
}

#[derive(Debug, Deserialize)]
struct StatePayload {
    #[serde(default)]
    capabilities: Vec<GoveeCapabilityState>,
}

#[derive(Debug, Deserialize)]
struct ScenePayload {
    #[serde(default)]
    capabilities: Vec<SceneCapability>,
}

#[derive(Debug, Deserialize)]
struct SceneCapability {
    #[serde(rename = "type")]
    kind: String,
    instance: String,
    parameters: SceneParameters,
}

#[derive(Debug, Deserialize)]
struct SceneParameters {
    #[serde(default)]
    options: Vec<SceneOption>,
}

#[derive(Debug, Deserialize)]
struct SceneOption {
    name: String,
    value: Value,
}

struct RateGate {
    disabled: bool,
    last_account_devices: Mutex<Option<Instant>>,
    last_account_control: Mutex<Option<Instant>>,
    last_device_control: Mutex<HashMap<String, Instant>>,
    last_device_state: Mutex<HashMap<String, Instant>>,
    last_device_scenes: Mutex<HashMap<String, Instant>>,
}

impl Default for RateGate {
    fn default() -> Self {
        Self {
            disabled: false,
            last_account_devices: Mutex::new(None),
            last_account_control: Mutex::new(None),
            last_device_control: Mutex::new(HashMap::new()),
            last_device_state: Mutex::new(HashMap::new()),
            last_device_scenes: Mutex::new(HashMap::new()),
        }
    }
}

impl RateGate {
    async fn account_devices(&self) {
        if self.disabled {
            return;
        }
        wait_slot(&self.last_account_devices, Duration::from_secs(2)).await;
    }

    async fn control(&self, device: &str) {
        if self.disabled {
            return;
        }
        // Published sustained budgets are 12 requests/second per account and
        // 2/second per device. Serial gates deliberately avoid consuming the
        // separately documented burst allowance during ordinary Loom use.
        wait_slot(&self.last_account_control, Duration::from_millis(84)).await;
        wait_keyed_slot(
            &self.last_device_control,
            device,
            Duration::from_millis(500),
        )
        .await;
    }

    async fn state(&self, device: &str) {
        if self.disabled {
            return;
        }
        wait_keyed_slot(&self.last_device_state, device, Duration::from_secs(2)).await;
    }

    async fn scenes(&self, device: &str) {
        if self.disabled {
            return;
        }
        wait_keyed_slot(&self.last_device_scenes, device, Duration::from_secs(2)).await;
    }
}

async fn wait_slot(slot: &Mutex<Option<Instant>>, interval: Duration) {
    let mut slot = slot.lock().await;
    if let Some(last) = *slot {
        tokio::time::sleep(interval.saturating_sub(last.elapsed())).await;
    }
    *slot = Some(Instant::now());
}

async fn wait_keyed_slot(slots: &Mutex<HashMap<String, Instant>>, key: &str, interval: Duration) {
    let mut slots = slots.lock().await;
    if let Some(last) = slots.get(key) {
        tokio::time::sleep(interval.saturating_sub(last.elapsed())).await;
    }
    slots.insert(key.to_owned(), Instant::now());
}

/// Client for Govee's fixed cloud endpoint.
#[derive(Clone)]
pub struct GoveeClient {
    api_key: Arc<str>,
    base_url: Arc<str>,
    http: reqwest::Client,
    rate_gate: Arc<RateGate>,
}

impl fmt::Debug for GoveeClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("GoveeClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl GoveeClient {
    pub fn connect(api_key: &str) -> Result<Self, GoveeError> {
        Self::connect_at(api_key, DEFAULT_BASE_URL)
    }

    pub(crate) fn connect_at(api_key: &str, base_url: &str) -> Result<Self, GoveeError> {
        if api_key.trim().is_empty() {
            return Err(GoveeError::AuthFailed("the API key is empty".to_owned()));
        }
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|error| GoveeError::ConnectionFailed(error.to_string()))?;
        Ok(Self {
            api_key: Arc::from(api_key.trim()),
            base_url: Arc::from(base_url.trim_end_matches('/')),
            http,
            rate_gate: Arc::new(RateGate::default()),
        })
    }

    #[cfg(test)]
    pub(crate) fn connect_at_unlimited(api_key: &str, base_url: &str) -> Result<Self, GoveeError> {
        let mut client = Self::connect_at(api_key, base_url)?;
        client.rate_gate = Arc::new(RateGate {
            disabled: true,
            ..RateGate::default()
        });
        Ok(client)
    }

    pub async fn devices(&self) -> Result<Vec<GoveeDevice>, GoveeError> {
        self.rate_gate.account_devices().await;
        self.request(Method::GET, "user/devices", None).await
    }

    pub async fn state(
        &self,
        device: &GoveeDevice,
    ) -> Result<Vec<GoveeCapabilityState>, GoveeError> {
        self.rate_gate.state(&device.device).await;
        let payload: StatePayload = self
            .request(Method::POST, "device/state", Some(device_request(device)))
            .await?;
        Ok(payload.capabilities)
    }

    pub async fn scenes(&self, device: &GoveeDevice) -> Result<Vec<GoveeScene>, GoveeError> {
        self.rate_gate.scenes(&device.device).await;
        let payload: ScenePayload = self
            .request(Method::POST, "device/scenes", Some(device_request(device)))
            .await?;
        Ok(payload
            .capabilities
            .into_iter()
            .flat_map(|capability| {
                capability
                    .parameters
                    .options
                    .into_iter()
                    .map(move |option| GoveeScene {
                        name: option.name,
                        capability_type: capability.kind.clone(),
                        instance: capability.instance.clone(),
                        value: option.value,
                    })
            })
            .collect())
    }

    pub async fn control(
        &self,
        device: &GoveeDevice,
        capability_type: &str,
        instance: &str,
        value: Value,
    ) -> Result<String, GoveeError> {
        self.rate_gate.control(&device.device).await;
        let request_id = Uuid::new_v4().to_string();
        let body = json!({
            "requestId": request_id,
            "payload": {
                "sku": device.sku,
                "device": device.device,
                "capability": {
                    "type": capability_type,
                    "instance": instance,
                    "value": value
                }
            }
        });
        self.request_optional::<Value>(Method::POST, "device/control", Some(body))
            .await?;
        Ok(request_id)
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<T, GoveeError> {
        self.request_optional(method, path, body)
            .await?
            .ok_or_else(|| GoveeError::ApiError {
                code: 200,
                message: "successful response contained no data or payload".to_owned(),
            })
    }

    async fn request_optional<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Option<T>, GoveeError> {
        let mut request = self
            .http
            .request(method, format!("{}/{}", self.base_url, path))
            .header("Govee-API-Key", self.api_key.as_ref());
        if let Some(body) = body {
            request = request.json(&body);
        }
        let response = request
            .send()
            .await
            .map_err(|error| GoveeError::ConnectionFailed(error.without_url().to_string()))?;
        let status = response.status();
        let bytes = response
            .bytes()
            .await
            .map_err(|error| GoveeError::ApiError {
                code: status.as_u16().into(),
                message: format!("could not read response: {error}"),
            })?;
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(GoveeError::AuthFailed(format!(
                "the service returned HTTP {}",
                status.as_u16()
            )));
        }
        if status == StatusCode::TOO_MANY_REQUESTS {
            return Err(GoveeError::RateLimited(
                "the service returned HTTP 429; wait before retrying".to_owned(),
            ));
        }
        if !status.is_success() {
            return Err(GoveeError::ApiError {
                code: status.as_u16().into(),
                message: format!("the service returned HTTP {}", status.as_u16()),
            });
        }
        let envelope: Envelope<T> =
            serde_json::from_slice(&bytes).map_err(|error| GoveeError::ApiError {
                code: status.as_u16().into(),
                message: format!("response contained malformed JSON: {error}"),
            })?;
        if matches!(envelope.code, 401 | 403) {
            return Err(GoveeError::AuthFailed(envelope.message));
        }
        if envelope.code == 429 {
            return Err(GoveeError::RateLimited(envelope.message));
        }
        if envelope.code != 200 {
            return Err(GoveeError::ApiError {
                code: envelope.code,
                message: envelope.message,
            });
        }
        Ok(envelope.data.or(envelope.payload))
    }
}

fn device_request(device: &GoveeDevice) -> Value {
    json!({
        "requestId": Uuid::new_v4().to_string(),
        "payload": { "sku": device.sku, "device": device.device }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_and_state_response_shapes_match_the_router_api() {
        let devices: Envelope<Vec<GoveeDevice>> = serde_json::from_value(json!({
            "code": 200,
            "message": "success",
            "data": [{
                "sku": "H6000", "device": "AA:BB", "deviceName": "Desk",
                "type": "devices.types.light",
                "capabilities": [{
                    "type": "devices.capabilities.range", "instance": "brightness",
                    "parameters": { "range": { "min": 1, "max": 100, "precision": 1 } }
                }]
            }]
        }))
        .expect("device response");
        let device = &devices.data.expect("data")[0];
        assert_eq!(device.device_name, "Desk");
        assert_eq!(device.capabilities[0].parameters["range"]["max"], 100);

        let state: Envelope<StatePayload> = serde_json::from_value(json!({
            "code": 200, "msg": "success", "payload": {
                "capabilities": [{ "type": "devices.capabilities.on_off",
                    "instance": "powerSwitch", "state": { "value": 1 } }]
            }
        }))
        .expect("state response");
        assert_eq!(
            state.payload.expect("payload").capabilities[0].state.value,
            1
        );
    }

    #[test]
    fn scene_values_remain_opaque() {
        let data: ScenePayload = serde_json::from_value(json!({ "capabilities": [{
            "type": "devices.capabilities.dynamic_scene", "instance": "lightScene",
            "parameters": { "options": [{ "name": "Sunset", "value": { "paramId": 7, "id": 2 } }] }
        }]}))
        .expect("scene response");
        let capability = &data.capabilities[0];
        assert_eq!(
            capability.parameters.options[0].value,
            json!({ "paramId": 7, "id": 2 })
        );
    }

    #[test]
    fn debug_output_never_contains_the_api_key() {
        let client = GoveeClient::connect("highly-secret").expect("client");
        assert!(!format!("{client:?}").contains("highly-secret"));
    }
}
