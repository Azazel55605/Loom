//! Music Assistant connector and reconnecting WebSocket transport.

mod config;
mod connector;

use std::{collections::HashMap, fmt, sync::Arc, time::Duration};

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use thiserror::Error;
use tokio::{
    net::TcpStream,
    sync::{mpsc, oneshot, watch, Mutex},
    time,
};
use tokio_tungstenite::{connect_async, tungstenite::Message, MaybeTlsStream, WebSocketStream};
use uuid::Uuid;

pub use config::{config_schema, MusicAssistantConnectorConfig};
pub use connector::{
    MusicAssistantConnector, DATA_POINT_IS_AVAILABLE, DATA_POINT_MUSIC_ASSISTANT_VERSION,
    DATA_POINT_PLAYER_COUNT, DATA_POINT_PLAYER_NAME, DATA_POINT_PLAYER_TYPE, DISPLAY_NAME, ICON,
    RESOURCE_KIND_PLAYERS, TYPE_ID,
};

/// Default port published by Music Assistant's local web server.
pub const DEFAULT_PORT: u16 = 8095;
/// WebSocket API route published by Music Assistant.
pub const API_PATH: &str = "/ws";

const AUTH_REQUIRED_SCHEMA: u32 = 28;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
const RECONNECT_INITIAL_DELAY: Duration = Duration::from_secs(1);
const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);
const OUTGOING_CAPACITY: usize = 128;

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
type CallResult = Result<Value, MusicAssistantError>;
type PendingRequests = Arc<Mutex<HashMap<String, PendingRequest>>>;

struct PendingRequest {
    sender: oneshot::Sender<CallResult>,
    partial: Vec<Value>,
}

struct OutgoingCall {
    id: String,
    payload: String,
}

/// The server-identification frame sent immediately after WebSocket setup.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct ServerInfo {
    pub server_id: String,
    pub server_version: String,
    pub schema_version: u32,
    pub min_supported_schema_version: u32,
    #[serde(default)]
    pub name: Option<String>,
}

/// Failures kept at the Music Assistant transport boundary.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum MusicAssistantError {
    #[error("could not connect to Music Assistant: {0}")]
    ConnectionFailed(String),
    #[error("connected to Music Assistant, but authentication failed: {0}")]
    AuthFailed(String),
    #[error("Music Assistant command failed: {message}")]
    CommandError {
        /// MA's structured error code, when this came from the server.
        code: Option<i64>,
        message: String,
    },
    #[error("Music Assistant command timed out")]
    Timeout,
    #[error("the Music Assistant connection was lost")]
    Disconnected,
}

/// Observable lifecycle of the background connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    Connected,
    Reconnecting,
    Disconnected,
}

/// Authenticated, correlated Music Assistant command transport.
#[derive(Clone)]
pub struct MusicAssistantClient {
    endpoint: Arc<str>,
    server_info: Arc<ServerInfo>,
    outgoing: mpsc::Sender<OutgoingCall>,
    pending: PendingRequests,
    state: watch::Receiver<ConnectionState>,
}

impl fmt::Debug for MusicAssistantClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MusicAssistantClient")
            .field("endpoint", &self.endpoint)
            .field("server_info", &self.server_info)
            .field("state", &self.connection_state())
            .finish_non_exhaustive()
    }
}

impl MusicAssistantClient {
    /// Opens Music Assistant's `/ws` endpoint and authenticates when required.
    ///
    /// Bare hosts use the local server's normal plaintext WebSocket transport.
    /// Prefixing `host` with `https://` or `wss://` selects TLS for deployments
    /// exposed through a reverse proxy. Current API schema 28+ requires a token;
    /// `None` remains supported only for older, pre-authentication schemas.
    pub async fn connect(
        host: &str,
        port: u16,
        token: Option<&str>,
    ) -> Result<Self, MusicAssistantError> {
        let endpoint: Arc<str> = Arc::from(endpoint_for_host(host, port)?);
        let token: Option<Arc<str>> = token.map(Arc::from);
        let (socket, server_info) = open_authenticated(&endpoint, token.as_deref()).await?;
        let server_info = Arc::new(server_info);
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (outgoing, outgoing_rx) = mpsc::channel(OUTGOING_CAPACITY);
        let (state_tx, state) = watch::channel(ConnectionState::Connected);

        tokio::spawn(connection_task(
            socket,
            endpoint.clone(),
            token,
            outgoing_rx,
            pending.clone(),
            state_tx,
        ));

        Ok(Self {
            endpoint,
            server_info,
            outgoing,
            pending,
            state,
        })
    }

    /// Sends one MA command and awaits the response with the same message id.
    pub async fn call(&self, command: &str, params: Value) -> Result<Value, MusicAssistantError> {
        if self.connection_state() != ConnectionState::Connected {
            return Err(MusicAssistantError::Disconnected);
        }
        let id = Uuid::new_v4().simple().to_string();
        let payload = command_request(&id, command, params)?;
        let (response_tx, response_rx) = oneshot::channel();
        self.pending.lock().await.insert(
            id.clone(),
            PendingRequest {
                sender: response_tx,
                partial: Vec::new(),
            },
        );

        if self
            .outgoing
            .send(OutgoingCall {
                id: id.clone(),
                payload,
            })
            .await
            .is_err()
        {
            self.pending.lock().await.remove(&id);
            return Err(MusicAssistantError::Disconnected);
        }

        match time::timeout(CALL_TIMEOUT, response_rx).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err(MusicAssistantError::Disconnected),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(MusicAssistantError::Timeout)
            }
        }
    }

    pub fn server_info(&self) -> &ServerInfo {
        &self.server_info
    }

    pub fn connection_state(&self) -> ConnectionState {
        *self.state.borrow()
    }

    pub fn subscribe_state(&self) -> watch::Receiver<ConnectionState> {
        self.state.clone()
    }
}

fn endpoint_for_host(host: &str, port: u16) -> Result<String, MusicAssistantError> {
    let host = host.trim().trim_end_matches('/');
    if host.is_empty() {
        return Err(MusicAssistantError::ConnectionFailed(
            "the host must not be empty".to_owned(),
        ));
    }
    let (scheme, authority) = if let Some(authority) = host.strip_prefix("https://") {
        ("wss", authority)
    } else if let Some(authority) = host.strip_prefix("http://") {
        ("ws", authority)
    } else if let Some(authority) = host.strip_prefix("wss://") {
        ("wss", authority)
    } else if let Some(authority) = host.strip_prefix("ws://") {
        ("ws", authority)
    } else if host.contains("://") {
        return Err(MusicAssistantError::ConnectionFailed(
            "only http(s):// or ws(s):// host schemes are accepted".to_owned(),
        ));
    } else {
        ("ws", host)
    };
    if authority.is_empty()
        || authority.contains('/')
        || authority.contains('?')
        || authority.contains('#')
    {
        return Err(MusicAssistantError::ConnectionFailed(
            "the host must not include a path, query, or fragment".to_owned(),
        ));
    }
    Ok(format!("{scheme}://{authority}:{port}{API_PATH}"))
}

fn command_request(id: &str, command: &str, params: Value) -> Result<String, MusicAssistantError> {
    let args = match params {
        Value::Null => Value::Object(Default::default()),
        Value::Object(_) => params,
        _ => {
            return Err(MusicAssistantError::CommandError {
                code: None,
                message: "Music Assistant command arguments must be a JSON object".to_owned(),
            })
        }
    };
    serde_json::to_string(&json!({
        "message_id": id,
        "command": command,
        "args": args,
    }))
    .map_err(|error| MusicAssistantError::CommandError {
        code: None,
        message: format!("could not serialize command: {error}"),
    })
}

async fn open_authenticated(
    endpoint: &str,
    token: Option<&str>,
) -> Result<(Socket, ServerInfo), MusicAssistantError> {
    let (mut socket, _) = time::timeout(CONNECT_TIMEOUT, connect_async(endpoint))
        .await
        .map_err(|_| MusicAssistantError::ConnectionFailed("connection timed out".to_owned()))?
        .map_err(|error| MusicAssistantError::ConnectionFailed(error.to_string()))?;
    let server_info = receive_server_info(&mut socket).await?;

    if server_info.schema_version >= AUTH_REQUIRED_SCHEMA {
        let token = token.ok_or_else(|| {
            MusicAssistantError::AuthFailed(format!(
                "server schema {} requires an access token",
                server_info.schema_version
            ))
        })?;
        authenticate(&mut socket, token).await?;
    }
    Ok((socket, server_info))
}

async fn receive_server_info(socket: &mut Socket) -> Result<ServerInfo, MusicAssistantError> {
    let payload = receive_json(socket, "server-info frame").await?;
    serde_json::from_value(payload).map_err(|error| {
        MusicAssistantError::ConnectionFailed(format!(
            "Music Assistant sent an invalid server-info frame: {error}"
        ))
    })
}

async fn authenticate(socket: &mut Socket, token: &str) -> Result<(), MusicAssistantError> {
    let id = Uuid::new_v4().simple().to_string();
    let payload = command_request(&id, "auth", json!({ "token": token }))
        .map_err(|error| MusicAssistantError::AuthFailed(error.to_string()))?;
    socket
        .send(Message::Text(payload.into()))
        .await
        .map_err(|error| MusicAssistantError::AuthFailed(error.to_string()))?;

    loop {
        let response = receive_json(socket, "authentication response")
            .await
            .map_err(|error| MusicAssistantError::AuthFailed(error.to_string()))?;
        if response.get("message_id").and_then(Value::as_str) != Some(&id) {
            continue;
        }
        if let Some((_, message)) = command_error(&response) {
            return Err(MusicAssistantError::AuthFailed(message));
        }
        if auth_succeeded(response.get("result")) {
            return Ok(());
        }
        return Err(MusicAssistantError::AuthFailed(
            "the auth command did not return true".to_owned(),
        ));
    }
}

fn auth_succeeded(result: Option<&Value>) -> bool {
    result.and_then(Value::as_bool) == Some(true)
        || result
            .and_then(|value| value.get("authenticated"))
            .and_then(Value::as_bool)
            == Some(true)
}

async fn receive_json(socket: &mut Socket, expected: &str) -> Result<Value, MusicAssistantError> {
    loop {
        let frame = time::timeout(CONNECT_TIMEOUT, socket.next())
            .await
            .map_err(|_| {
                MusicAssistantError::ConnectionFailed(format!("timed out waiting for {expected}"))
            })?;
        match frame {
            Some(Ok(Message::Text(text))) => {
                return serde_json::from_str(&text).map_err(|error| {
                    MusicAssistantError::ConnectionFailed(format!(
                        "Music Assistant sent malformed JSON: {error}"
                    ))
                });
            }
            Some(Ok(Message::Binary(bytes))) => {
                return serde_json::from_slice(&bytes).map_err(|error| {
                    MusicAssistantError::ConnectionFailed(format!(
                        "Music Assistant sent malformed JSON: {error}"
                    ))
                });
            }
            Some(Ok(Message::Ping(payload))) => {
                socket
                    .send(Message::Pong(payload))
                    .await
                    .map_err(|error| MusicAssistantError::ConnectionFailed(error.to_string()))?;
            }
            Some(Ok(Message::Close(_))) | Some(Err(_)) | None => {
                return Err(MusicAssistantError::Disconnected)
            }
            Some(Ok(_)) => {}
        }
    }
}

enum ConnectionExit {
    Shutdown,
    Lost,
}

async fn connection_task(
    mut socket: Socket,
    endpoint: Arc<str>,
    token: Option<Arc<str>>,
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
            match open_authenticated(&endpoint, token.as_deref()).await {
                Ok((reconnected, _)) => {
                    socket = reconnected;
                    let _ = state.send(ConnectionState::Connected);
                    break;
                }
                Err(_) => delay = RECONNECT_MAX_DELAY.min(delay.saturating_mul(2)),
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

async fn dispatch_response(
    payload: &[u8],
    pending: &PendingRequests,
) -> Result<(), MusicAssistantError> {
    let message: Value = serde_json::from_slice(payload).map_err(|error| {
        MusicAssistantError::ConnectionFailed(format!(
            "Music Assistant sent malformed JSON: {error}"
        ))
    })?;
    let Some(id) = message.get("message_id").and_then(Value::as_str) else {
        return Ok(()); // server info and event frames are not call responses
    };

    if message.get("partial").and_then(Value::as_bool) == Some(true) {
        let chunk = message
            .get("result")
            .and_then(Value::as_array)
            .ok_or_else(|| MusicAssistantError::CommandError {
                code: None,
                message: "a partial response did not contain an array result".to_owned(),
            })?;
        if let Some(waiter) = pending.lock().await.get_mut(id) {
            waiter.partial.extend(chunk.iter().cloned());
        }
        return Ok(());
    }

    let Some(waiter) = pending.lock().await.remove(id) else {
        return Ok(());
    };
    let result = if let Some((code, message)) = command_error(&message) {
        Err(MusicAssistantError::CommandError {
            code: Some(code),
            message,
        })
    } else if let Some(result) = message.get("result") {
        if waiter.partial.is_empty() {
            Ok(result.clone())
        } else {
            let final_chunk = result
                .as_array()
                .ok_or_else(|| MusicAssistantError::CommandError {
                    code: None,
                    message: "the final chunk did not contain an array result".to_owned(),
                });
            final_chunk.map(|chunk| {
                Value::Array(
                    waiter
                        .partial
                        .into_iter()
                        .chain(chunk.iter().cloned())
                        .collect(),
                )
            })
        }
    } else {
        Err(MusicAssistantError::CommandError {
            code: None,
            message: "response contained neither result nor error details".to_owned(),
        })
    };
    let _ = waiter.sender.send(result);
    Ok(())
}

fn command_error(message: &Value) -> Option<(i64, String)> {
    let code = message.get("error_code")?.as_i64()?;
    Some((
        code,
        message
            .get("details")
            .and_then(Value::as_str)
            .unwrap_or("unknown Music Assistant command error")
            .to_owned(),
    ))
}

async fn fail_all_pending(pending: &PendingRequests) {
    let waiters = std::mem::take(&mut *pending.lock().await);
    for (_, waiter) in waiters {
        let _ = waiter.sender.send(Err(MusicAssistantError::Disconnected));
    }
}

async fn fail_one_pending(id: &str, pending: &PendingRequests) {
    if let Some(waiter) = pending.lock().await.remove(id) {
        let _ = waiter.sender.send(Err(MusicAssistantError::Disconnected));
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
                Some(outgoing_call) => fail_one_pending(&outgoing_call.id, pending).await,
                None => return false,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_support_local_plaintext_and_proxy_tls() {
        assert_eq!(
            endpoint_for_host("music.example.com", 8095).unwrap(),
            "ws://music.example.com:8095/ws"
        );
        assert_eq!(
            endpoint_for_host("https://music.example.com", 443).unwrap(),
            "wss://music.example.com:443/ws"
        );
    }

    #[test]
    fn endpoint_rejects_paths_and_unknown_schemes() {
        for host in ["music.example.com/path", "ftp://music.example.com"] {
            assert!(matches!(
                endpoint_for_host(host, DEFAULT_PORT),
                Err(MusicAssistantError::ConnectionFailed(_))
            ));
        }
    }

    #[test]
    fn command_envelope_uses_ma_fields_instead_of_json_rpc() {
        let request: Value =
            serde_json::from_str(&command_request("request-id", "players/all", json!({})).unwrap())
                .unwrap();
        assert_eq!(request["message_id"], "request-id");
        assert_eq!(request["command"], "players/all");
        assert_eq!(request["args"], json!({}));
        assert!(request.get("jsonrpc").is_none());
    }

    #[tokio::test]
    async fn response_reaches_only_its_correlated_waiter() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (wanted_tx, wanted_rx) = oneshot::channel();
        let (other_tx, mut other_rx) = oneshot::channel();
        pending.lock().await.insert(
            "wanted".to_owned(),
            PendingRequest {
                sender: wanted_tx,
                partial: Vec::new(),
            },
        );
        pending.lock().await.insert(
            "other".to_owned(),
            PendingRequest {
                sender: other_tx,
                partial: Vec::new(),
            },
        );

        dispatch_response(
            br#"{"message_id":"wanted","result":{"ok":true},"partial":false}"#,
            &pending,
        )
        .await
        .unwrap();

        assert_eq!(wanted_rx.await.unwrap().unwrap(), json!({"ok": true}));
        assert!(matches!(
            other_rx.try_recv(),
            Err(oneshot::error::TryRecvError::Empty)
        ));
    }

    #[tokio::test]
    async fn partial_array_responses_are_combined() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let (tx, rx) = oneshot::channel();
        pending.lock().await.insert(
            "stream".to_owned(),
            PendingRequest {
                sender: tx,
                partial: Vec::new(),
            },
        );
        dispatch_response(
            br#"{"message_id":"stream","result":[1,2],"partial":true}"#,
            &pending,
        )
        .await
        .unwrap();
        dispatch_response(
            br#"{"message_id":"stream","result":[3],"partial":false}"#,
            &pending,
        )
        .await
        .unwrap();
        assert_eq!(rx.await.unwrap().unwrap(), json!([1, 2, 3]));
    }

    #[tokio::test]
    async fn disconnect_resolves_every_waiter() {
        let pending = Arc::new(Mutex::new(HashMap::new()));
        let mut receivers = Vec::new();
        for index in 0..3 {
            let (tx, rx) = oneshot::channel();
            pending.lock().await.insert(
                index.to_string(),
                PendingRequest {
                    sender: tx,
                    partial: Vec::new(),
                },
            );
            receivers.push(rx);
        }
        fail_all_pending(&pending).await;
        for receiver in receivers {
            assert_eq!(
                receiver.await.unwrap(),
                Err(MusicAssistantError::Disconnected)
            );
        }
    }

    #[test]
    fn command_errors_keep_the_server_details() {
        assert_eq!(
            command_error(&json!({"error_code": 401, "details": "invalid token"})),
            Some((401, "invalid token".to_owned()))
        );
    }

    #[test]
    fn authentication_accepts_current_and_legacy_success_shapes() {
        assert!(auth_succeeded(Some(&json!({
            "authenticated": true,
            "user": {"username": "example"}
        }))));
        assert!(auth_succeeded(Some(&json!(true))));
        assert!(!auth_succeeded(Some(&json!({"authenticated": false}))));
    }
}
