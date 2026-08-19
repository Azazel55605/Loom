//! Connector listing and action execution.
//!
//! These moved out of the removed `dev_stub_auth` module unchanged: they were
//! never stub auth, only stub-*adjacent*, and the response shapes clients are
//! built against are the real ones.
//!
//! **They are not yet authorization-checked.** Anyone who can reach the port
//! can list connectors and execute actions on them. The `connectors.view` and
//! `connectors.control` permissions exist and are granted, but nothing consults
//! them until the authorization middleware lands — that is the follow-up this
//! change deliberately stops short of, and until it exists these routes are no
//! better protected than they were before.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use loom_core::connector::{ConnectorAction, ConnectorError, ConnectorMetadata, ConnectorStatus};
use serde::Serialize;
use serde_json::Value;

use crate::auth::extract::{
    AuthenticatedUser, ConnectorsControl, ConnectorsView, Permission, RequirePermission,
};
use crate::error::ErrorBody;
use crate::state::AppState;

/// The `resource_type` connectors are scoped by in `group_permissions`.
///
/// One constant rather than a literal at each call site: a typo in a scope
/// string does not fail to compile, it silently fails to match, which means a
/// grant that quietly authorizes nothing.
pub const CONNECTOR_RESOURCE_TYPE: &str = "connector";

/// One entry in `GET /connectors`.
///
/// Nested rather than flattened: `metadata`, `status`, and `actions` are Core
/// wire types clients deserialize elsewhere too, so nesting lets the TypeScript
/// types compose instead of being re-declared per response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorListEntry {
    metadata: ConnectorMetadata,
    /// `null` when the status check itself failed — see `statusError`.
    status: Option<ConnectorStatus>,
    /// Present only when `status` is `null`. One unreachable connector must not
    /// blank out the whole list, so the failure is reported per entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    status_error: Option<ConnectorError>,
    /// What this connector can be asked to do, right now. May be empty.
    ///
    /// Included in the list rather than behind a second request because the
    /// dashboard needs it for every connector it renders.
    actions: Vec<ConnectorAction>,
}

/// `GET /connectors`
///
/// Requires a **global** `connectors.view` grant.
///
/// Worth noting what that excludes: a user granted `connectors.view` scoped to
/// a single connector is refused here rather than shown a one-element list. The
/// alternative — accept any `connectors.view` grant and filter the response to
/// what the caller may see — is friendlier and is what this should become once
/// scoped view grants are actually issued. It is not built yet because nothing
/// issues them, and a filter with no way to create the case it filters is a
/// feature that cannot be tested against reality.
pub async fn list_connectors(
    _caller: RequirePermission<ConnectorsView>,
    State(state): State<AppState>,
) -> Json<Vec<ConnectorListEntry>> {
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
            actions: connector.actions().await,
        });
    }

    Json(entries)
}

/// `POST /connectors/{id}/actions/{action_id}`
///
/// Requires `connectors.control` over this specific connector. A global grant
/// covers every connector; a grant scoped to `connector:<id>` covers only that
/// one.
///
/// The check runs **before** the connector is looked up, so a caller without
/// permission gets 403 whether or not the id exists. Looking up first would
/// answer 404 for an unknown id and 403 for a known one, turning this endpoint
/// into a way to enumerate which connectors are configured.
pub async fn execute_action(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path((id, action_id)): Path<(String, String)>,
    body: Bytes,
) -> Response {
    // Resource-scoped, so the requirement is checked here rather than in the
    // signature: the resource id only exists once the path has been parsed.
    if let Some(denied) = caller.deny_unless(
        ConnectorsControl::KEY,
        Some(CONNECTOR_RESOURCE_TYPE),
        Some(&id),
    ) {
        return denied;
    }

    let Some(connector) = state.connector(&id) else {
        return ErrorBody::message(StatusCode::NOT_FOUND, format!("no such connector: {id}"));
    };

    // Read raw bytes rather than using the `Json` extractor, so an absent body
    // is legal and no `Content-Type` is demanded. An empty body becomes JSON
    // `null`, deliberately distinct from `{}`: "sent nothing" and "sent an
    // empty object" stay distinguishable.
    let params: Value = if body.is_empty() {
        Value::Null
    } else {
        match serde_json::from_slice(&body) {
            Ok(value) => value,
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

/// Maps a connector failure onto an HTTP status.
///
/// `AuthFailed` is **502, not 401**. It means the *upstream service* rejected
/// *Loom's* stored credentials: the caller is not the party that failed to
/// authenticate and holds nothing that would fix it, so a 401 would wrongly
/// tell a client to re-prompt its user. It is a gateway failure, like
/// `Unreachable`.
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
