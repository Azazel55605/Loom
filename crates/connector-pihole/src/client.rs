use std::{error::Error as _, fmt, sync::Arc, time::Duration};

use reqwest::{Method, StatusCode, Url};
use serde::Deserialize;
use serde_json::{json, Value};
use thiserror::Error;
use tokio::sync::{Mutex, RwLock};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const SID_HEADER: &str = "X-FTL-SID";

/// Failures at the Pi-hole HTTP/session boundary.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum PiHoleError {
    #[error("could not connect to Pi-hole: {0}")]
    ConnectionFailed(String),
    #[error("Pi-hole authentication failed: {0}")]
    AuthFailed(String),
    #[error("Pi-hole API returned HTTP {status}: {message}")]
    ApiError { status: u16, message: String },
}

/// Authenticated Pi-hole v6 REST client with one automatic session refresh.
#[derive(Clone)]
pub struct PiHoleClient {
    base_url: Arc<str>,
    password: Arc<str>,
    http: reqwest::Client,
    sid: Arc<RwLock<String>>,
    auth_gate: Arc<Mutex<()>>,
}

impl fmt::Debug for PiHoleClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PiHoleClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl PiHoleClient {
    /// Builds an HTTP client and establishes the first Pi-hole session.
    pub async fn connect(base_url: &str, password: &str) -> Result<Self, PiHoleError> {
        Self::connect_with_certificate_policy(base_url, password, false).await
    }

    /// Builds an HTTP client with an explicit certificate-validation policy.
    ///
    /// Allowing an invalid certificate only relaxes TLS peer verification; it
    /// does not downgrade an HTTPS URL or send credentials over plaintext.
    pub async fn connect_with_certificate_policy(
        base_url: &str,
        password: &str,
        allow_insecure_cert: bool,
    ) -> Result<Self, PiHoleError> {
        let client = Self::new_with_certificate_policy(base_url, password, allow_insecure_cert)?;
        client.authenticate().await?;
        Ok(client)
    }

    /// Constructs a candidate without contacting Pi-hole. The setup test uses
    /// this so authentication failures become a structured `reachable: false`
    /// result instead of preventing the test connector from being built.
    pub(crate) fn new_with_certificate_policy(
        base_url: &str,
        password: &str,
        allow_insecure_cert: bool,
    ) -> Result<Self, PiHoleError> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .danger_accept_invalid_certs(allow_insecure_cert)
            .build()
            .map_err(|error| PiHoleError::ConnectionFailed(error.to_string()))?;
        Ok(Self::with_http(base_url, password, http))
    }

    fn with_http(base_url: &str, password: &str, http: reqwest::Client) -> Self {
        Self {
            base_url: Arc::from(base_url.trim_end_matches('/')),
            password: Arc::from(password),
            http,
            sid: Arc::new(RwLock::new(String::new())),
            auth_gate: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) async fn authenticate(&self) -> Result<(), PiHoleError> {
        let _gate = self.auth_gate.lock().await;
        self.authenticate_locked().await?;
        drop(_gate);
        Ok(())
    }

    pub(crate) async fn get_json(&self, path: &str) -> Result<Value, PiHoleError> {
        self.request_json(Method::GET, path, None).await
    }

    pub(crate) async fn post_json(&self, path: &str, body: Value) -> Result<Value, PiHoleError> {
        self.request_json(Method::POST, path, Some(body)).await
    }

    pub(crate) async fn post_json_segments(
        &self,
        segments: &[&str],
        body: Value,
    ) -> Result<Value, PiHoleError> {
        self.request_json_at(Method::POST, self.endpoint_segments(segments)?, Some(body))
            .await
    }

    pub(crate) async fn put_json_segments(
        &self,
        segments: &[&str],
        body: Value,
    ) -> Result<Value, PiHoleError> {
        self.request_json_at(Method::PUT, self.endpoint_segments(segments)?, Some(body))
            .await
    }

    pub(crate) async fn delete_segments(&self, segments: &[&str]) -> Result<(), PiHoleError> {
        self.request_json_at(Method::DELETE, self.endpoint_segments(segments)?, None)
            .await
            .map(|_| ())
    }

    async fn request_json(
        &self,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, PiHoleError> {
        self.request_json_at(method, self.endpoint(path), body)
            .await
    }

    async fn request_json_at(
        &self,
        method: Method,
        endpoint: String,
        body: Option<Value>,
    ) -> Result<Value, PiHoleError> {
        let attempted_sid = self.sid.read().await.clone();
        let response = self
            .send(method.clone(), &endpoint, body.as_ref(), &attempted_sid)
            .await?;

        if response.status() != StatusCode::UNAUTHORIZED {
            return decode_json(response).await;
        }

        // Several concurrent reads can observe the same expired SID. Only the
        // first creates a replacement session; the others reuse it after the
        // gate opens rather than consuming Pi-hole's bounded session slots.
        let _gate = self.auth_gate.lock().await;
        if self.sid.read().await.as_str() == attempted_sid {
            self.authenticate_locked().await?;
        }
        let refreshed_sid = self.sid.read().await.clone();
        let retried = self
            .send(method, &endpoint, body.as_ref(), &refreshed_sid)
            .await?;
        if retried.status() == StatusCode::UNAUTHORIZED {
            let message = response_message(retried).await;
            return Err(PiHoleError::AuthFailed(format!(
                "the refreshed session was rejected: {message}"
            )));
        }
        decode_json(retried).await
    }

    async fn authenticate_locked(&self) -> Result<(), PiHoleError> {
        let response = self
            .http
            .post(self.endpoint("auth"))
            .json(&json!({ "password": self.password.as_ref() }))
            .send()
            .await
            .map_err(connection_error)?;
        let status = response.status();
        if status == StatusCode::UNAUTHORIZED || status == StatusCode::TOO_MANY_REQUESTS {
            return Err(PiHoleError::AuthFailed(response_message(response).await));
        }
        if !status.is_success() {
            return Err(PiHoleError::ApiError {
                status: status.as_u16(),
                message: response_message(response).await,
            });
        }

        let payload: AuthResponse =
            response
                .json()
                .await
                .map_err(|error| PiHoleError::ApiError {
                    status: status.as_u16(),
                    message: format!("authentication returned malformed JSON: {error}"),
                })?;
        if !payload.session.valid {
            return Err(PiHoleError::AuthFailed(
                payload
                    .session
                    .message
                    .unwrap_or_else(|| "the returned session is not valid".to_owned()),
            ));
        }
        let sid = payload
            .session
            .sid
            .filter(|sid| !sid.is_empty())
            .ok_or_else(|| {
                PiHoleError::AuthFailed(
                    "authentication succeeded but returned no session ID".to_owned(),
                )
            })?;
        *self.sid.write().await = sid;
        Ok(())
    }

    async fn send(
        &self,
        method: Method,
        endpoint: &str,
        body: Option<&Value>,
        sid: &str,
    ) -> Result<reqwest::Response, PiHoleError> {
        let mut request = self.http.request(method, endpoint).header(SID_HEADER, sid);
        if let Some(body) = body {
            request = request.json(body);
        }
        request.send().await.map_err(connection_error)
    }

    fn endpoint(&self, path: &str) -> String {
        format!("{}/api/{}", self.base_url, path.trim_start_matches('/'))
    }

    fn endpoint_segments(&self, segments: &[&str]) -> Result<String, PiHoleError> {
        let mut url = Url::parse(&format!("{}/api/", self.base_url)).map_err(|error| {
            PiHoleError::ConnectionFailed(format!("the configured Pi-hole URL is invalid: {error}"))
        })?;
        {
            let mut path = url.path_segments_mut().map_err(|()| {
                PiHoleError::ConnectionFailed(
                    "the configured Pi-hole URL cannot contain path segments".to_owned(),
                )
            })?;
            path.pop_if_empty();
            for segment in segments {
                path.push(segment);
            }
        }
        Ok(url.to_string())
    }
}

#[derive(Deserialize)]
struct AuthResponse {
    session: AuthSession,
}

#[derive(Deserialize)]
struct AuthSession {
    valid: bool,
    sid: Option<String>,
    message: Option<String>,
}

#[derive(Deserialize)]
struct ErrorEnvelope {
    error: Option<ApiErrorBody>,
}

#[derive(Deserialize)]
struct ApiErrorBody {
    message: String,
    hint: Option<String>,
}

async fn decode_json(response: reqwest::Response) -> Result<Value, PiHoleError> {
    let status = response.status();
    if !status.is_success() {
        return Err(PiHoleError::ApiError {
            status: status.as_u16(),
            message: response_message(response).await,
        });
    }
    if status == StatusCode::NO_CONTENT {
        return Ok(Value::Null);
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| PiHoleError::ApiError {
            status: status.as_u16(),
            message: format!("could not read the response body: {error}"),
        })?;
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|error| PiHoleError::ApiError {
        status: status.as_u16(),
        message: format!("response was not valid JSON: {error}"),
    })
}

async fn response_message(response: reqwest::Response) -> String {
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    serde_json::from_str::<ErrorEnvelope>(&text)
        .ok()
        .and_then(|payload| payload.error)
        .map(|error| match error.hint.filter(|hint| !hint.is_empty()) {
            Some(hint) => format!("{} ({hint})", error.message),
            None => error.message,
        })
        .filter(|message| !message.is_empty())
        .unwrap_or_else(|| {
            if text.trim().is_empty() {
                status
                    .canonical_reason()
                    .unwrap_or("request failed")
                    .to_owned()
            } else {
                text
            }
        })
}

fn connection_error(error: reqwest::Error) -> PiHoleError {
    let mut reasons = vec![error.to_string()];
    let mut source = error.source();
    while let Some(cause) = source {
        let reason = cause.to_string();
        if !reason.is_empty() && !reasons.iter().any(|existing| existing == &reason) {
            reasons.push(reason);
        }
        source = cause.source();
    }

    let certificate_was_rejected = reasons.iter().any(|reason| {
        let reason = reason.to_ascii_lowercase();
        reason.contains("certificate")
            || reason.contains("unknownissuer")
            || reason.contains("invalidpeercredential")
    });
    let mut message = reasons.join(": ");
    if certificate_was_rejected {
        message.push_str(
            "; TLS certificate validation failed. Enable allowInsecureCert only after verifying the Pi-hole endpoint",
        );
    }
    PiHoleError::ConnectionFailed(message)
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, sync::Arc};

    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        sync::Mutex,
    };

    use super::*;

    #[tokio::test]
    async fn a_401_reauthenticates_once_and_retries_with_the_new_sid() {
        let requests = Arc::new(Mutex::new(Vec::new()));
        let base_url = spawn_server(
            VecDeque::from([
                response(
                    200,
                    r#"{"session":{"valid":true,"sid":"first","message":"correct password"}}"#,
                ),
                response(401, r#"{"error":{"message":"Unauthorized","hint":null}}"#),
                response(
                    200,
                    r#"{"session":{"valid":true,"sid":"second","message":"correct password"}}"#,
                ),
                response(200, r#"{"queries":{"total":42}}"#),
            ]),
            requests.clone(),
        )
        .await;

        let client = PiHoleClient::connect(&base_url, "not-a-real-password")
            .await
            .expect("initial authentication");
        let result = client
            .get_json("stats/summary")
            .await
            .expect("request should be retried after reauthentication");
        assert_eq!(result["queries"]["total"], 42);

        let requests = requests.lock().await;
        assert_eq!(requests.len(), 4);
        assert!(requests[1]
            .to_ascii_lowercase()
            .contains("x-ftl-sid: first"));
        assert!(requests[3]
            .to_ascii_lowercase()
            .contains("x-ftl-sid: second"));
    }

    #[test]
    fn resource_path_segments_are_percent_encoded_independently() {
        let client = PiHoleClient::new_with_certificate_policy(
            "https://pi.hole:8443/proxy",
            "not-a-real-password",
            false,
        )
        .expect("client builder");
        let endpoint = client
            .endpoint_segments(&["domains", "deny", "exact", "path/with space.example"])
            .expect("encoded endpoint");
        assert_eq!(
            endpoint,
            "https://pi.hole:8443/proxy/api/domains/deny/exact/path%2Fwith%20space.example"
        );
    }

    async fn spawn_server(
        responses: VecDeque<String>,
        requests: Arc<Mutex<Vec<String>>>,
    ) -> String {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock server");
        let address = listener.local_addr().expect("mock address");
        tokio::spawn(async move {
            let responses = Arc::new(Mutex::new(responses));
            loop {
                let Ok((mut socket, _)) = listener.accept().await else {
                    return;
                };
                let responses = responses.clone();
                let requests = requests.clone();
                tokio::spawn(async move {
                    let mut buffer = vec![0_u8; 8192];
                    let Ok(size) = socket.read(&mut buffer).await else {
                        return;
                    };
                    requests
                        .lock()
                        .await
                        .push(String::from_utf8_lossy(&buffer[..size]).into_owned());
                    let Some(response) = responses.lock().await.pop_front() else {
                        return;
                    };
                    let _ = socket.write_all(response.as_bytes()).await;
                });
            }
        });
        format!("http://{address}")
    }

    fn response(status: u16, body: &str) -> String {
        let reason = if status == 200 { "OK" } else { "Unauthorized" };
        format!(
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
    }
}
