//! Loom's minimal host-level connector and TLS-only JSON-RPC transport for TrueNAS.
//!
//! [`TrueNasClient`] establishes an authenticated, reconnecting WebSocket and
//! correlates concurrent RPC calls. [`TrueNasConnector`] builds the first
//! useful connector surface on it: host version and aggregate pool capacity.

mod config;
mod connector;

pub use config::{config_schema, TrueNasConnectorConfig};
pub use connector::{
    TrueNasConnector, DATA_POINT_FREE_CAPACITY_BYTES, DATA_POINT_POOL_COUNT,
    DATA_POINT_TOTAL_CAPACITY_BYTES, DATA_POINT_TRUENAS_VERSION, DATA_POINT_USED_CAPACITY_BYTES,
    DISPLAY_NAME, ICON, TYPE_ID,
};

use std::{collections::HashMap, fmt, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime},
    ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot, watch, Mutex},
    time,
};
use tokio_tungstenite::{
    connect_async_tls_with_config,
    tungstenite::{self, Message},
    Connector, MaybeTlsStream, WebSocketStream,
};
use uuid::Uuid;

/// The version-selected JSON-RPC endpoint published by current TrueNAS.
pub const API_PATH: &str = "/api/current";

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const RECONNECT_INITIAL_DELAY: Duration = Duration::from_secs(1);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);
const OUTGOING_CAPACITY: usize = 128;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type RpcResult = Result<Value, TrueNasError>;
type PendingRequests = Arc<Mutex<HashMap<Uuid, oneshot::Sender<RpcResult>>>>;

/// A transport failure kept distinct from TrueNAS's own RPC failures.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TrueNasError {
    /// DNS, TCP, TLS, WebSocket-handshake, or malformed-endpoint failure.
    #[error("could not connect to TrueNAS: {0}")]
    ConnectionFailed(String),
    /// The WebSocket connected, but TrueNAS did not establish a session.
    #[error("connected to TrueNAS, but authentication failed: {0}")]
    AuthFailed(String),
    /// A JSON-RPC error returned by TrueNAS itself.
    #[error("TrueNAS RPC error {code}: {message}")]
    RpcError { code: i64, message: String },
    /// No correlated response arrived within the call deadline.
    #[error("TrueNAS RPC call timed out")]
    Timeout,
    /// The ready connection disappeared while a call was in flight.
    #[error("the TrueNAS connection was lost")]
    Disconnected,
}

/// Observable lifecycle of the client's background connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Reconnecting,
    Disconnected,
}

#[derive(Clone)]
enum Authentication {
    /// Stable 25.10 still supports this key-only method, although it is
    /// deprecated in favour of `auth.login_ex` and scheduled for removal in 27.
    LegacyApiKey { api_key: Arc<str> },
    /// Current preferred plain API-key mechanism. The raw key remains protected
    /// by mandatory TLS; future SCRAM support can become another strategy.
    ApiKeyPlain {
        username: Arc<str>,
        api_key: Arc<str>,
    },
}

impl Authentication {
    fn request(&self) -> (&'static str, Value) {
        match self {
            Self::LegacyApiKey { api_key } => {
                ("auth.login_with_api_key", json!([api_key.as_ref()]))
            }
            Self::ApiKeyPlain { username, api_key } => (
                "auth.login_ex",
                json!([{
                    "mechanism": "API_KEY_PLAIN",
                    "username": username.as_ref(),
                    "api_key": api_key.as_ref(),
                    "login_options": { "user_info": false }
                }]),
            ),
        }
    }

    fn accepts(&self, result: &Value) -> bool {
        match self {
            Self::LegacyApiKey { .. } => result == &Value::Bool(true),
            Self::ApiKeyPlain { .. } => {
                result.get("response_type").and_then(Value::as_str) == Some("SUCCESS")
            }
        }
    }

    fn rejection(&self, result: &Value) -> String {
        match self {
            Self::LegacyApiKey { .. } => "auth.login_with_api_key did not return true".to_owned(),
            Self::ApiKeyPlain { .. } => result
                .get("response_type")
                .and_then(Value::as_str)
                .map(|kind| format!("auth.login_ex returned {kind}"))
                .unwrap_or_else(|| "auth.login_ex returned an unexpected response".to_owned()),
        }
    }
}

struct OutgoingCall {
    id: Uuid,
    payload: String,
}

/// An authenticated TrueNAS JSON-RPC client with automatic reconnection.
#[derive(Clone)]
pub struct TrueNasClient {
    endpoint: Arc<str>,
    outgoing: mpsc::Sender<OutgoingCall>,
    pending: PendingRequests,
    state: watch::Receiver<ConnectionState>,
}

impl fmt::Debug for TrueNasClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("TrueNasClient")
            .field("endpoint", &self.endpoint)
            .field("state", &self.connection_state())
            .finish_non_exhaustive()
    }
}

impl TrueNasClient {
    /// Connects over `wss://`, validates or explicitly accepts the server
    /// certificate, and authenticates with the stable key-only API method.
    ///
    /// `allow_insecure_cert` disables certificate validation only. It never
    /// disables encryption and cannot make this client construct `ws://`.
    ///
    /// TrueNAS 25.10 still supports the requested key-only
    /// `auth.login_with_api_key` method, but deprecates it for removal in 27.
    /// New integrations that know the API key's username should use
    /// [`Self::connect_with_username`], which uses the current `auth.login_ex`
    /// `API_KEY_PLAIN` mechanism.
    pub async fn connect(
        host: &str,
        api_key: &str,
        allow_insecure_cert: bool,
    ) -> Result<Self, TrueNasError> {
        Self::connect_with_authentication(
            host,
            Authentication::LegacyApiKey {
                api_key: Arc::from(api_key),
            },
            allow_insecure_cert,
        )
        .await
    }

    /// Connects using TrueNAS's preferred stable API-key authentication shape.
    ///
    /// This is otherwise identical to [`Self::connect`], including mandatory
    /// TLS and the meaning of `allow_insecure_cert`.
    pub async fn connect_with_username(
        host: &str,
        username: &str,
        api_key: &str,
        allow_insecure_cert: bool,
    ) -> Result<Self, TrueNasError> {
        Self::connect_with_authentication(
            host,
            Authentication::ApiKeyPlain {
                username: Arc::from(username),
                api_key: Arc::from(api_key),
            },
            allow_insecure_cert,
        )
        .await
    }

    async fn connect_with_authentication(
        host: &str,
        authentication: Authentication,
        allow_insecure_cert: bool,
    ) -> Result<Self, TrueNasError> {
        let endpoint: Arc<str> = Arc::from(endpoint_for_host(host)?);
        let socket = open_authenticated(&endpoint, &authentication, allow_insecure_cert).await?;
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (outgoing, outgoing_rx) = mpsc::channel(OUTGOING_CAPACITY);
        let (state_tx, state) = watch::channel(ConnectionState::Connected);

        tokio::spawn(connection_task(
            socket,
            endpoint.clone(),
            authentication,
            allow_insecure_cert,
            outgoing_rx,
            pending.clone(),
            state_tx,
        ));

        Ok(Self {
            endpoint,
            outgoing,
            pending,
            state,
        })
    }

    /// Sends one JSON-RPC request and awaits the response bearing the same id.
    pub async fn call(&self, method: &str, params: Value) -> RpcResult {
        if self.connection_state() != ConnectionState::Connected {
            return Err(TrueNasError::Disconnected);
        }

        let id = Uuid::new_v4();
        let payload = rpc_request(id, method, params);
        let (response_tx, response_rx) = oneshot::channel();
        self.pending.lock().await.insert(id, response_tx);

        if self
            .outgoing
            .send(OutgoingCall { id, payload })
            .await
            .is_err()
        {
            self.pending.lock().await.remove(&id);
            return Err(TrueNasError::Disconnected);
        }

        match time::timeout(CALL_TIMEOUT, response_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(TrueNasError::Disconnected),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(TrueNasError::Timeout)
            }
        }
    }

    /// Returns the latest connection state without waiting for a change.
    pub fn connection_state(&self) -> ConnectionState {
        *self.state.borrow()
    }

    /// Subscribes to connection-state changes for status surfaces or tests.
    pub fn subscribe_state(&self) -> watch::Receiver<ConnectionState> {
        self.state.clone()
    }
}

fn endpoint_for_host(host: &str) -> Result<String, TrueNasError> {
    let host = host.trim();
    if host.is_empty() {
        return Err(TrueNasError::ConnectionFailed(
            "the host must not be empty".to_owned(),
        ));
    }

    let authority = if let Some(authority) = host.strip_prefix("wss://") {
        authority
    } else if host.contains("://") {
        return Err(TrueNasError::ConnectionFailed(
            "only the wss:// scheme is accepted".to_owned(),
        ));
    } else {
        host
    }
    .trim_end_matches('/');

    if authority.is_empty()
        || authority.contains('/')
        || authority.contains('?')
        || authority.contains('#')
    {
        return Err(TrueNasError::ConnectionFailed(
            "the host must contain only a hostname and optional port".to_owned(),
        ));
    }

    Ok(format!("wss://{authority}{API_PATH}"))
}

fn rpc_request(id: Uuid, method: &str, params: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": id.to_string(),
        "method": method,
        "params": params,
    })
    .to_string()
}

async fn open_authenticated(
    endpoint: &str,
    authentication: &Authentication,
    allow_insecure_cert: bool,
) -> Result<Socket, TrueNasError> {
    let connector = tls_connector(allow_insecure_cert)?;
    let connecting = connect_async_tls_with_config(endpoint, None, false, Some(connector));
    let (mut socket, _) = time::timeout(CONNECT_TIMEOUT, connecting)
        .await
        .map_err(|_| {
            TrueNasError::ConnectionFailed("the TLS/WebSocket handshake timed out".to_owned())
        })?
        .map_err(connection_error)?;

    authenticate(&mut socket, authentication).await?;
    Ok(socket)
}

fn tls_connector(allow_insecure_cert: bool) -> Result<Connector, TrueNasError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|error| TrueNasError::ConnectionFailed(error.to_string()))?;

    let config = if allow_insecure_cert {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(SkipServerVerification(provider)))
            .with_no_client_auth()
    } else {
        let roots = RootCertStore::from_iter(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        builder.with_root_certificates(roots).with_no_client_auth()
    };

    Ok(Connector::Rustls(Arc::new(config)))
}

/// Accepts an untrusted chain/hostname while retaining TLS encryption and
/// cryptographic handshake-signature verification. This is used only when the
/// caller explicitly opts into self-signed certificates.
#[derive(Debug)]
struct SkipServerVerification(Arc<CryptoProvider>);

impl ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

async fn authenticate(
    socket: &mut Socket,
    authentication: &Authentication,
) -> Result<(), TrueNasError> {
    let (method, params) = authentication.request();
    let result = direct_call(socket, method, params)
        .await
        .map_err(|error| match error {
            TrueNasError::RpcError { code, message } => {
                TrueNasError::AuthFailed(format!("TrueNAS returned RPC error {code}: {message}"))
            }
            other => other,
        })?;

    if authentication.accepts(&result) {
        Ok(())
    } else {
        Err(TrueNasError::AuthFailed(authentication.rejection(&result)))
    }
}

async fn direct_call(socket: &mut Socket, method: &str, params: Value) -> RpcResult {
    let id = Uuid::new_v4();
    socket
        .send(Message::Text(rpc_request(id, method, params).into()))
        .await
        .map_err(connection_error)?;

    time::timeout(CALL_TIMEOUT, async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Text(text))) => {
                    if let Some(result) = correlated_result(text.as_ref(), id)? {
                        return result;
                    }
                }
                Some(Ok(Message::Binary(bytes))) => {
                    if let Some(result) = correlated_result(&bytes, id)? {
                        return result;
                    }
                }
                Some(Ok(Message::Ping(payload))) => socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(connection_error)?,
                Some(Ok(Message::Close(frame))) => {
                    let detail = frame.map_or_else(
                        || "without a close frame".to_owned(),
                        |frame| {
                            let reason = frame.reason.trim();
                            if reason.is_empty() {
                                format!("with close code {} and no reason", frame.code)
                            } else {
                                format!("with close code {}: {reason}", frame.code)
                            }
                        },
                    );
                    return Err(TrueNasError::ConnectionFailed(format!(
                        "TrueNAS closed the WebSocket while waiting for `{method}` {detail}"
                    )));
                }
                None => {
                    return Err(TrueNasError::ConnectionFailed(format!(
                        "the TrueNAS WebSocket ended while waiting for `{method}` without a close frame"
                    )));
                }
                Some(Err(error)) => return Err(connection_error(error)),
                Some(Ok(_)) => {}
            }
        }
    })
    .await
    .map_err(|_| TrueNasError::Timeout)?
}

fn correlated_result(payload: &[u8], expected_id: Uuid) -> Result<Option<RpcResult>, TrueNasError> {
    let message: Value = serde_json::from_slice(payload).map_err(|error| {
        TrueNasError::ConnectionFailed(format!("TrueNAS sent malformed JSON: {error}"))
    })?;
    let Some(id) = response_id(&message) else {
        return Ok(None);
    };
    if id != expected_id {
        return Ok(None);
    }
    Ok(Some(result_from_message(&message)))
}

fn response_id(message: &Value) -> Option<Uuid> {
    message
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())
}

fn result_from_message(message: &Value) -> RpcResult {
    if let Some(error) = message.get("error") {
        let code = error.get("code").and_then(Value::as_i64).unwrap_or(-32000);
        let message = error
            .get("message")
            .and_then(Value::as_str)
            .or_else(|| {
                error
                    .get("data")
                    .and_then(|data| data.get("reason"))
                    .and_then(Value::as_str)
            })
            .unwrap_or("unknown RPC error")
            .to_owned();
        return Err(TrueNasError::RpcError { code, message });
    }

    message.get("result").cloned().ok_or_else(|| {
        TrueNasError::ConnectionFailed("TrueNAS response had neither result nor error".to_owned())
    })
}

enum ConnectionExit {
    Shutdown,
    Lost,
}

async fn connection_task(
    mut socket: Socket,
    endpoint: Arc<str>,
    authentication: Authentication,
    allow_insecure_cert: bool,
    mut outgoing: mpsc::Receiver<OutgoingCall>,
    pending: PendingRequests,
    state: watch::Sender<ConnectionState>,
) {
    loop {
        let exit = run_connected(&mut socket, &mut outgoing, &pending).await;
        fail_all_pending(&pending).await;
        let _ = state.send(ConnectionState::Disconnected);

        if matches!(exit, ConnectionExit::Shutdown) {
            return;
        }

        let _ = state.send(ConnectionState::Reconnecting);
        let mut delay = RECONNECT_INITIAL_DELAY;
        loop {
            if !wait_to_reconnect(delay, &mut outgoing, &pending).await {
                let _ = state.send(ConnectionState::Disconnected);
                return;
            }

            match open_authenticated(&endpoint, &authentication, allow_insecure_cert).await {
                Ok(reconnected) => {
                    socket = reconnected;
                    let _ = state.send(ConnectionState::Connected);
                    break;
                }
                Err(_) => {
                    delay = RECONNECT_MAX_DELAY.min(delay.saturating_mul(2));
                }
            }
        }
    }
}

async fn run_connected(
    socket: &mut Socket,
    outgoing: &mut mpsc::Receiver<OutgoingCall>,
    pending: &PendingRequests,
) -> ConnectionExit {
    loop {
        tokio::select! {
            outgoing_call = outgoing.recv() => {
                let Some(outgoing_call) = outgoing_call else {
                    let _ = socket.close(None).await;
                    return ConnectionExit::Shutdown;
                };
                if !pending.lock().await.contains_key(&outgoing_call.id) {
                    continue;
                }
                if socket.send(Message::Text(outgoing_call.payload.into())).await.is_err() {
                    return ConnectionExit::Lost;
                }
            }
            incoming = socket.next() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if dispatch_response(text.as_bytes(), pending).await.is_err() {
                            return ConnectionExit::Lost;
                        }
                    }
                    Some(Ok(Message::Binary(bytes))) => {
                        if dispatch_response(&bytes, pending).await.is_err() {
                            return ConnectionExit::Lost;
                        }
                    }
                    Some(Ok(Message::Ping(payload))) => {
                        if socket.send(Message::Pong(payload)).await.is_err() {
                            return ConnectionExit::Lost;
                        }
                    }
                    Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                        return ConnectionExit::Lost;
                    }
                    Some(Ok(_)) => {}
                }
            }
        }
    }
}

async fn dispatch_response(payload: &[u8], pending: &PendingRequests) -> Result<(), TrueNasError> {
    let message: Value = serde_json::from_slice(payload).map_err(|error| {
        TrueNasError::ConnectionFailed(format!("TrueNAS sent malformed JSON: {error}"))
    })?;
    let Some(id) = response_id(&message) else {
        return Ok(());
    };
    let Some(waiter) = pending.lock().await.remove(&id) else {
        return Ok(());
    };
    let _ = waiter.send(result_from_message(&message));
    Ok(())
}

async fn fail_all_pending(pending: &PendingRequests) {
    let waiters = std::mem::take(&mut *pending.lock().await);
    for (_, waiter) in waiters {
        let _ = waiter.send(Err(TrueNasError::Disconnected));
    }
}

async fn fail_one_pending(id: Uuid, pending: &PendingRequests) {
    if let Some(waiter) = pending.lock().await.remove(&id) {
        let _ = waiter.send(Err(TrueNasError::Disconnected));
    }
}

async fn wait_to_reconnect(
    delay: Duration,
    outgoing: &mut mpsc::Receiver<OutgoingCall>,
    pending: &PendingRequests,
) -> bool {
    let sleep = time::sleep(delay);
    tokio::pin!(sleep);
    loop {
        tokio::select! {
            () = &mut sleep => return true,
            outgoing_call = outgoing.recv() => match outgoing_call {
                Some(outgoing_call) => fail_one_pending(outgoing_call.id, pending).await,
                None => return false,
            }
        }
    }
}

fn connection_error(error: tungstenite::Error) -> TrueNasError {
    TrueNasError::ConnectionFailed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_is_always_wss_and_current() {
        assert_eq!(
            endpoint_for_host("nas.example.com:8443").unwrap(),
            "wss://nas.example.com:8443/api/current"
        );
        assert_eq!(
            endpoint_for_host("wss://nas.example.com/").unwrap(),
            "wss://nas.example.com/api/current"
        );
    }

    #[test]
    fn plaintext_and_host_paths_are_structurally_rejected() {
        for invalid in [
            "ws://nas.example.com",
            "http://nas.example.com",
            "https://nas.example.com",
            "nas.example.com/websocket",
        ] {
            assert!(matches!(
                endpoint_for_host(invalid),
                Err(TrueNasError::ConnectionFailed(_))
            ));
        }
    }

    #[test]
    fn rpc_envelope_uses_a_string_id_and_supplied_params() {
        let id = Uuid::new_v4();
        let request: Value =
            serde_json::from_str(&rpc_request(id, "core.ping", Value::Array(Vec::new()))).unwrap();
        assert_eq!(request["jsonrpc"], "2.0");
        assert_eq!(request["id"], id.to_string());
        assert_eq!(request["method"], "core.ping");
        assert_eq!(request["params"], json!([]));
    }

    #[test]
    fn preferred_authentication_shape_matches_the_stable_schema() {
        let authentication = Authentication::ApiKeyPlain {
            username: Arc::from("service-account"),
            api_key: Arc::from("not-a-real-key"),
        };
        let (method, params) = authentication.request();
        assert_eq!(method, "auth.login_ex");
        assert_eq!(params[0]["mechanism"], "API_KEY_PLAIN");
        assert_eq!(params[0]["username"], "service-account");
        assert_eq!(params[0]["api_key"], "not-a-real-key");
        assert_eq!(params[0]["login_options"]["user_info"], false);
    }

    #[tokio::test]
    async fn a_response_reaches_only_its_correlated_waiter() {
        let wanted = Uuid::new_v4();
        let other = Uuid::new_v4();
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (wanted_tx, wanted_rx) = oneshot::channel();
        let (other_tx, mut other_rx) = oneshot::channel();
        pending.lock().await.insert(wanted, wanted_tx);
        pending.lock().await.insert(other, other_tx);

        dispatch_response(
            json!({"jsonrpc":"2.0", "id":wanted, "result":"pong"})
                .to_string()
                .as_bytes(),
            &pending,
        )
        .await
        .unwrap();

        assert_eq!(wanted_rx.await.unwrap().unwrap(), "pong");
        assert!(matches!(
            other_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn truenas_rpc_errors_keep_their_code_and_best_message() {
        let result = result_from_message(&json!({
            "jsonrpc": "2.0",
            "id": Uuid::new_v4(),
            "error": {
                "code": -32001,
                "message": null,
                "data": {"reason": "permission denied"}
            }
        }));
        assert_eq!(
            result,
            Err(TrueNasError::RpcError {
                code: -32001,
                message: "permission denied".to_owned(),
            })
        );
    }

    #[tokio::test]
    async fn disconnect_resolves_every_waiter() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let mut receivers = Vec::new();
        for _ in 0..3 {
            let (tx, rx) = oneshot::channel();
            pending.lock().await.insert(Uuid::new_v4(), tx);
            receivers.push(rx);
        }

        fail_all_pending(&pending).await;
        assert!(pending.lock().await.is_empty());
        for receiver in receivers {
            assert_eq!(receiver.await.unwrap(), Err(TrueNasError::Disconnected));
        }
    }
}
