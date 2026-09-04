use std::{error::Error as _, fmt, sync::Arc, time::Duration};

use reqwest::StatusCode;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;
use tokio::sync::Semaphore;

const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const API_KEY_HEADER: &str = "X-API-KEY";
const INTEGRATION_PATH: &str = "proxy/network/integration/v1";
const MAX_CONCURRENT_CALLS: usize = 10;

/// Failures at the official UniFi Network Integration API boundary.
#[derive(Debug, Clone, Error, PartialEq, Eq)]
pub enum UniFiNetworkError {
    #[error("could not connect to UniFi Network: {0}")]
    ConnectionFailed(String),
    #[error("UniFi Network authentication failed: {0}")]
    AuthFailed(String),
    #[error("UniFi Network API returned HTTP {status}: {message}")]
    ApiError { status: u16, message: String },
}

/// API-key-authenticated client for a local UniFi Network console.
#[derive(Clone)]
pub struct UniFiNetworkClient {
    base_url: Arc<str>,
    api_key: Arc<str>,
    http: reqwest::Client,
    call_limit: Arc<Semaphore>,
}

impl fmt::Debug for UniFiNetworkClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("UniFiNetworkClient")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl UniFiNetworkClient {
    /// Builds a client. Certificate validation stays enabled unless the one
    /// connector instance explicitly opts out; HTTPS itself is never disabled.
    pub fn connect(
        base_url: &str,
        api_key: &str,
        allow_insecure_cert: bool,
    ) -> Result<Self, UniFiNetworkError> {
        let http = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .danger_accept_invalid_certs(allow_insecure_cert)
            .build()
            .map_err(|error| UniFiNetworkError::ConnectionFailed(error.to_string()))?;
        Ok(Self {
            base_url: Arc::from(base_url.trim_end_matches('/')),
            api_key: Arc::from(api_key),
            http,
            call_limit: Arc::new(Semaphore::new(MAX_CONCURRENT_CALLS)),
        })
    }

    pub(crate) async fn get<T: DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, UniFiNetworkError> {
        let _permit = self.call_permit().await?;
        let endpoint = self.endpoint(path);
        let response = self
            .http
            .get(&endpoint)
            .header(API_KEY_HEADER, self.api_key.as_ref())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(connection_error)?;
        let status = response.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(UniFiNetworkError::AuthFailed(
                response_message(response).await,
            ));
        }
        if !status.is_success() {
            return Err(UniFiNetworkError::ApiError {
                status: status.as_u16(),
                message: response_message(response).await,
            });
        }
        response
            .json()
            .await
            .map_err(|error| UniFiNetworkError::ApiError {
                status: status.as_u16(),
                message: format!(
                    "GET {} returned JSON incompatible with the documented response: {}",
                    api_path(&endpoint),
                    error_chain(&error)
                ),
            })
    }

    pub(crate) async fn post_json(&self, path: &str, body: Value) -> Result<(), UniFiNetworkError> {
        let _permit = self.call_permit().await?;
        let endpoint = self.endpoint(path);
        let response = self
            .http
            .post(&endpoint)
            .header(API_KEY_HEADER, self.api_key.as_ref())
            .header(reqwest::header::ACCEPT, "application/json")
            .json(&body)
            .send()
            .await
            .map_err(connection_error)?;
        let status = response.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(UniFiNetworkError::AuthFailed(
                response_message(response).await,
            ));
        }
        if !status.is_success() {
            return Err(UniFiNetworkError::ApiError {
                status: status.as_u16(),
                message: response_message(response).await,
            });
        }
        Ok(())
    }

    pub(crate) async fn delete(&self, path: &str) -> Result<(), UniFiNetworkError> {
        let _permit = self.call_permit().await?;
        let endpoint = self.endpoint(path);
        let response = self
            .http
            .delete(&endpoint)
            .header(API_KEY_HEADER, self.api_key.as_ref())
            .header(reqwest::header::ACCEPT, "application/json")
            .send()
            .await
            .map_err(connection_error)?;
        let status = response.status();
        if matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN) {
            return Err(UniFiNetworkError::AuthFailed(
                response_message(response).await,
            ));
        }
        if !status.is_success() {
            return Err(UniFiNetworkError::ApiError {
                status: status.as_u16(),
                message: response_message(response).await,
            });
        }
        Ok(())
    }

    async fn call_permit(&self) -> Result<tokio::sync::SemaphorePermit<'_>, UniFiNetworkError> {
        // One connector clone is shared by the host poll and every target
        // consumer. The official API documents no endpoint-specific rate
        // budget, but its per-device statistics shape naturally creates a
        // request fan-out, so keep a hard upper bound across every clone.
        self.call_limit.acquire().await.map_err(|_| {
            UniFiNetworkError::ConnectionFailed(
                "the UniFi Network request limiter was closed".to_owned(),
            )
        })
    }

    fn endpoint(&self, path: &str) -> String {
        format!(
            "{}/{INTEGRATION_PATH}/{}",
            self.base_url,
            path.trim_start_matches('/')
        )
    }
}

fn api_path(endpoint: &str) -> &str {
    endpoint
        .find("/proxy/network/integration/")
        .map_or(endpoint, |start| &endpoint[start..])
}

fn error_chain(error: &reqwest::Error) -> String {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let rendered = cause.to_string();
        if !message.contains(&rendered) {
            message.push_str(": ");
            message.push_str(&rendered);
        }
        source = cause.source();
    }
    message
}

fn connection_error(error: reqwest::Error) -> UniFiNetworkError {
    let mut message = error.to_string();
    let mut source = error.source();
    while let Some(cause) = source {
        let rendered = cause.to_string();
        if !message.contains(&rendered) {
            message.push_str(": ");
            message.push_str(&rendered);
        }
        source = cause.source();
    }
    UniFiNetworkError::ConnectionFailed(message)
}

async fn response_message(response: reqwest::Response) -> String {
    let status = response.status();
    match response.text().await {
        Ok(body) if !body.trim().is_empty() => body,
        Ok(_) => status
            .canonical_reason()
            .unwrap_or("request failed")
            .to_owned(),
        Err(error) => format!("could not read error response: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reqwest::Url;

    #[test]
    fn endpoint_uses_the_official_local_versioned_base_path() {
        let client =
            UniFiNetworkClient::connect("https://console.example.com/", "not-a-real-key", false)
                .expect("client");
        assert_eq!(
            client.endpoint("sites"),
            "https://console.example.com/proxy/network/integration/v1/sites"
        );
    }

    #[test]
    fn configured_origin_is_a_valid_url() {
        let client =
            UniFiNetworkClient::connect("https://console.example.com:8443", "not-a-real-key", true)
                .expect("client");
        assert!(Url::parse(&client.endpoint("sites")).is_ok());
    }

    #[test]
    fn diagnostics_name_only_the_api_path_not_the_private_console_origin() {
        assert_eq!(
            api_path("https://console.example.com/proxy/network/integration/v1/sites/x/devices"),
            "/proxy/network/integration/v1/sites/x/devices"
        );
    }

    #[test]
    fn client_caps_concurrent_requests_for_device_fan_out() {
        let client =
            UniFiNetworkClient::connect("https://console.example.com", "not-a-real-key", false)
                .expect("client");
        assert_eq!(client.call_limit.available_permits(), MAX_CONCURRENT_CALLS);
    }
}
