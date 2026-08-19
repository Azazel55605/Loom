//! **Temporary stub. Not auth.**
//!
//! This module exists so the clients can be built against a *shaped* API before
//! the real auth layer exists. It is a placeholder for
//! `docs/adr/0003-auth-model-vpn-vs-external.md` (VPN-trusted owner, Authentik
//! for everyone else) and it implements none of it:
//!
//! - `POST /api/auth/login` accepts **any** username and password and hands
//!   back a fixed, guessable token.
//! - `GET /api/auth/session` compares against that one hard-coded token.
//! - The connector routes are **unauthenticated** — anyone who can reach the
//!   port can execute connector actions.
//!
//! There is deliberately **no credential storage of any kind**. This codebase
//! has no database yet, and per
//! `docs/adr/0004-zero-config-startup.md` the persistence and secret-generation
//! design is deferred until the real auth work starts. Adding a half-persisted
//! user table here would mean designing that system against a stub, so nothing
//! is stored: no users, no password hashes, no sessions, no revocation.
//!
//! The whole module is compiled only under the `dev-stub-auth` feature, which is
//! not in `default` and must never be enabled in a shipped build. See
//! `docs/API_CONTRACT.md` for the request/response contract and
//! `docs/AGENT_INSTRUCTIONS.md` for the rule.
//!
//! ## Error shape
//!
//! Every error response from this module is `{"error": "<message>"}`, optionally
//! with a `connectorError` field carrying the serialized
//! [`ConnectorError`] when the failure came from a connector.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Duration, Utc};
use loom_core::connector::{
    mock::MockConnector, Connector, ConnectorError, ConnectorMetadata, ConnectorStatus,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The one token the stub accepts. Fixed and public on purpose: it is a
/// placeholder for a real signed session token, not a secret.
pub const DEV_STUB_TOKEN: &str = "dev-stub-token";

/// The user every stub login resolves to. There is no user store.
pub const DEV_STUB_USER: &str = "dev-stub-user";

/// How long a stub token claims to be valid. Never enforced — nothing checks
/// `expiresAt`, because nothing records when a token was issued.
const TOKEN_LIFETIME_HOURS: i64 = 1;

/// The connector registry, shared by every handler.
///
/// Heterogeneous and plural from the start — `Vec<Arc<dyn Connector>>` rather
/// than a single `MockConnector` — so that registering real connectors is an
/// insertion rather than a reshape of the state type and every handler that
/// reads it. Lookup is by [`ConnectorMetadata::id`]; a linear scan is correct
/// at this size and a map can replace it without touching the routes.
#[derive(Clone)]
pub struct StubState {
    connectors: Arc<Vec<Arc<dyn Connector>>>,
}

impl StubState {
    /// The registry the stub boots with: one happy-path [`MockConnector`].
    pub fn with_mock_connector() -> Self {
        Self {
            connectors: Arc::new(vec![Arc::new(MockConnector::default())]),
        }
    }

    /// Finds a registered connector by its metadata id.
    fn find(&self, id: &str) -> Option<&Arc<dyn Connector>> {
        self.connectors
            .iter()
            .find(|connector| connector.metadata().id == id)
    }
}

/// The stub's routes, ready to be merged into the main router.
pub fn routes() -> Router {
    Router::new()
        .route("/api/auth/login", post(login))
        .route("/api/auth/session", get(session))
        .route("/api/connectors", get(list_connectors))
        .route(
            "/api/connectors/{id}/actions/{action_id}",
            post(execute_action),
        )
        .with_state(StubState::with_mock_connector())
}

/// The single error shape for every failing response in this module.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    /// Human-readable summary, safe to show to a user.
    error: String,
    /// The originating [`ConnectorError`], when there was one, so a client can
    /// branch on the variant instead of parsing prose.
    #[serde(skip_serializing_if = "Option::is_none")]
    connector_error: Option<ConnectorError>,
}

impl ErrorBody {
    fn message(status: StatusCode, error: impl Into<String>) -> Response {
        (
            status,
            Json(Self {
                error: error.into(),
                connector_error: None,
            }),
        )
            .into_response()
    }

    fn connector(status: StatusCode, error: ConnectorError) -> Response {
        (
            status,
            Json(Self {
                error: error.to_string(),
                connector_error: Some(error),
            }),
        )
            .into_response()
    }
}

/// `POST /api/auth/login` request body. Both fields are read and discarded.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LoginRequest {
    #[allow(dead_code, reason = "the stub accepts any credentials and stores none")]
    username: String,
    #[allow(dead_code, reason = "the stub accepts any credentials and stores none")]
    password: String,
}

/// `POST /api/auth/login` response body.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LoginResponse {
    token: &'static str,
    expires_at: DateTime<Utc>,
}

/// `GET /api/auth/session` response body for an accepted token.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionResponse {
    authenticated: bool,
    user: &'static str,
}

/// One entry in `GET /api/connectors`.
///
/// Nested rather than flattened: `metadata` and `status` are Core wire types
/// that clients already deserialize elsewhere, so keeping them intact means the
/// TypeScript types compose instead of being re-declared per response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ConnectorListEntry {
    metadata: ConnectorMetadata,
    /// `null` when the status check itself failed — see `statusError`.
    status: Option<ConnectorStatus>,
    /// Present only when `status` is `null`. One unreachable connector must not
    /// blank out the whole list, so the failure is reported per entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    status_error: Option<ConnectorError>,
}

/// Accepts anything and issues the fixed token.
async fn login(Json(_credentials): Json<LoginRequest>) -> Json<LoginResponse> {
    Json(LoginResponse {
        token: DEV_STUB_TOKEN,
        expires_at: Utc::now() + Duration::hours(TOKEN_LIFETIME_HOURS),
    })
}

/// Reports whether the `Authorization: Bearer …` header carries the stub token.
async fn session(headers: HeaderMap) -> Response {
    match bearer_token(&headers) {
        Some(DEV_STUB_TOKEN) => Json(SessionResponse {
            authenticated: true,
            user: DEV_STUB_USER,
        })
        .into_response(),
        _ => ErrorBody::message(
            StatusCode::UNAUTHORIZED,
            "missing or invalid bearer token".to_owned(),
        ),
    }
}

/// Extracts the token from an `Authorization: Bearer <token>` header.
fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
}

/// Lists every registered connector with its current status.
async fn list_connectors(State(state): State<StubState>) -> Json<Vec<ConnectorListEntry>> {
    let mut entries = Vec::with_capacity(state.connectors.len());

    for connector in state.connectors.iter() {
        let (status, status_error) = match connector.status().await {
            Ok(status) => (Some(status), None),
            Err(error) => (None, Some(error)),
        };

        entries.push(ConnectorListEntry {
            metadata: connector.metadata(),
            status,
            status_error,
        });
    }

    Json(entries)
}

/// Executes one action on one connector.
///
/// The request body is optional: an absent or empty body becomes
/// [`Value::Null`] rather than `{}`, because that is what
/// [`Connector::execute_action`] already treats as "no parameters" and it keeps
/// "sent nothing" distinguishable from "sent an empty object".
async fn execute_action(
    State(state): State<StubState>,
    Path((id, action_id)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    let Some(connector) = state.find(&id) else {
        return ErrorBody::message(StatusCode::NOT_FOUND, format!("no such connector: {id}"));
    };

    let params = if body.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&body) {
            Ok(params) => params,
            Err(error) => {
                return ErrorBody::message(
                    StatusCode::BAD_REQUEST,
                    format!("request body is not valid JSON: {error}"),
                );
            }
        }
    };

    match connector.execute_action(&action_id, params).await {
        Ok(result) => Json(result).into_response(),
        Err(error) => ErrorBody::connector(status_for(&error), error),
    }
}

/// Maps a [`ConnectorError`] onto the HTTP status the *caller* should see.
///
/// The distinction that drives this: `InvalidAction` and `InvalidParams` are the
/// caller's mistake, everything else is the upstream service failing Loom.
///
/// - `InvalidAction` → **404**, consistent with an unknown connector id: the
///   `/api/connectors/{id}/actions/{action_id}` path names a thing that is not
///   there.
/// - `InvalidParams` → **400**, the request reached a real action and was
///   malformed.
/// - `AuthFailed` → **502**, deliberately *not* 401. It means the *service*
///   rejected *Loom's* stored credentials; the caller is not the party who
///   failed to authenticate and has no credentials to correct. Answering 401
///   would tell a client to re-prompt its user, which cannot fix a bad token in
///   Loom's connector configuration. It is a bad gateway response, like
///   `Unreachable`.
/// - `Unreachable` → **502**, Loom could not reach the upstream at all.
/// - `Internal` → **500**, the failure is inside Loom.
fn status_for(error: &ConnectorError) -> StatusCode {
    match error {
        ConnectorError::InvalidAction { .. } => StatusCode::NOT_FOUND,
        ConnectorError::InvalidParams { .. } => StatusCode::BAD_REQUEST,
        ConnectorError::AuthFailed { .. } | ConnectorError::Unreachable { .. } => {
            StatusCode::BAD_GATEWAY
        }
        ConnectorError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
