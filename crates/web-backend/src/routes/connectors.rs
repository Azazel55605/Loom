//! Connector types and connector instances.
//!
//! Two related route groups:
//!
//! - `/connector-types` — the catalog of what this build can create. Read-only,
//!   code-defined, identical on every deployment of the same version.
//! - `/connector-instances` — what this deployment actually has. Full CRUD,
//!   stored in `connector_instances`, mirrored into the in-memory runtime.
//!
//! The catalog carries each type's JSON Schema, which is what lets the "add
//! connector" form be generated from data rather than written per connector in
//! three clients. See `docs/adr/0011-connector-instance-registry.md`.
//!
//! **Status comes from the runtime's poll cache.** HTTP reads never wait on an
//! upstream service. The same cache changes are broadcast over `/ws`, so the
//! list response and live updates cannot disagree about the latest snapshot.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use loom_core::connector::{
    Connector, ConnectorAction, ConnectorError, ConnectorMetadata, ConnectorStatus,
    DataPointDescriptor, DisplayField, SetupGuide, WidgetLayout,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use uuid::Uuid;

use crate::auth::extract::{
    AuthenticatedUser, ConnectorsControl, ConnectorsManage, ConnectorsView, Permission,
    RequirePermission,
};
use crate::connectors::runtime::{BuildError, ConnectorStatusSnapshot, PendingOperation};
use crate::error::{internal_error, ErrorBody};
use crate::state::AppState;

/// The `resource_type` connectors are scoped by in `group_permissions`.
///
/// One constant rather than a literal at each call site: a typo in a scope
/// string does not fail to compile, it silently fails to match, which means a
/// grant that quietly authorizes nothing.
pub const CONNECTOR_RESOURCE_TYPE: &str = "connector";

/// Internal route helpers carry ready-made HTTP failures, but an Axum
/// `Response` is large enough to trip Clippy's `result_large_err` on newer Rust
/// releases. Keep the success path compact by storing that uncommon value out
/// of line.
type RouteResult<T> = Result<T, Box<Response>>;

/* ------------------------------------------------------------------ */
/* Connector types                                                     */
/* ------------------------------------------------------------------ */

/// One entry in `GET /connector-types`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorTypeResponse {
    type_id: String,
    display_name: String,
    /// The type's icon reference, so the type picker can draw one before any
    /// instance of it exists. Same convention as `ConnectorMetadata::icon`.
    icon: Option<String>,
    /// JSON Schema for this type's configuration, as published by the
    /// connector itself. The add-connector form is generated from it, so
    /// registering a new connector type needs no frontend change.
    config_schema: Value,
    setup_guide: Option<SetupGuide>,
    discoverable_type: Option<String>,
}

/// `GET /connector-types`
///
/// Requires a global `connectors.manage` grant. This is the catalog behind the
/// "add a connector" form, and someone who cannot add one has nothing to do
/// with it — the instances they may see are on `/connector-instances`, which
/// asks only for `connectors.view`.
pub async fn list_connector_types(
    _caller: RequirePermission<ConnectorsManage>,
    State(state): State<AppState>,
) -> Json<Vec<ConnectorTypeResponse>> {
    let mut types: Vec<ConnectorTypeResponse> = state
        .connectors
        .types()
        .values()
        .map(|registration| ConnectorTypeResponse {
            type_id: registration.type_id.to_owned(),
            display_name: registration.display_name.to_owned(),
            icon: registration.icon.clone(),
            config_schema: registration.schema.clone(),
            setup_guide: registration.setup_guide.clone(),
            discoverable_type: registration.discoverable_type.clone(),
        })
        .collect();

    // The registry is a `HashMap`, whose iteration order varies per process.
    // Sorted so a client's type picker does not reshuffle between restarts.
    types.sort_by(|a, b| a.display_name.cmp(&b.display_name));

    Json(types)
}

/* ------------------------------------------------------------------ */
/* Connector instances                                                 */
/* ------------------------------------------------------------------ */

/// One entry in `GET /connector-instances`.
///
/// Nested rather than flattened: `metadata` and `status` are Core wire types
/// clients deserialize elsewhere too, so nesting lets the TypeScript types
/// compose instead of being re-declared per response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInstanceResponse {
    id: String,
    name: String,
    connector_type: String,
    created_at: String,
    metadata: ConnectorMetadata,
    /// The user's per-instance icon choice, overriding `metadata.icon`.
    ///
    /// `null` means "no override": the client falls back to `metadata.icon`,
    /// and then to its own generic default. Same reference convention as
    /// `metadata.icon`, and equally unvalidated here — resolution is entirely
    /// client-side, so the backend stores the string and nothing more.
    icon_override: Option<String>,
    /// `null` when the status check itself failed — see `statusError`.
    status: Option<ConnectorStatus>,
    /// Present only when `status` is `null`. One unreachable connector must not
    /// blank out the whole list, so the failure is reported per entry.
    #[serde(skip_serializing_if = "Option::is_none")]
    status_error: Option<ConnectorError>,
    /// A disruptive action running against this instance right now.
    ///
    /// A **sibling** of `status` rather than a field inside it, and the two
    /// halves say different things: `status` is what the connector reported,
    /// this is what the platform is doing to it. A service mid-restart really
    /// is Down, and folding the overlay into `ConnectorStatus` would both
    /// destroy that fact and change a Core type that three clients and every
    /// connector already depend on. Same shape as the WebSocket frame.
    pending_operation: Option<PendingOperation>,
    /// Why this instance is Down, established by probing the network beneath
    /// it. `null` unless it is Down and its connector names a target.
    diagnosis: Option<String>,
    /// The values this connector has agreed may be shown on the shell.
    display_fields: Vec<DisplayField>,
}

/// The full detail of one instance, as `GET /connector-instances/{id}` returns.
///
/// Everything the list carries, plus what a dashboard placement UI needs: the
/// data points that can be bound, the layout the connector ships with, and the
/// stored configuration so an edit form can be pre-filled.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorInstanceDetail {
    #[serde(flatten)]
    instance: ConnectorInstanceResponse,
    /// The stored configuration, as written.
    ///
    /// Returned so the edit form can be pre-filled. **Revisit when a connector
    /// type stores a credential**: this is `connectors.view`-gated today
    /// because the only registered type has nothing secret in it, and a real
    /// integration will need either a redaction pass here or a stricter
    /// permission on this field.
    config: Value,
    /// What this connector can be asked to do, right now. May be empty.
    actions: Vec<ConnectorAction>,
    /// What this connector can bind to a widget.
    data_points: Vec<DataPointDescriptor>,
    /// The widget arrangement the connector ships with.
    default_layout: WidgetLayout,
    /// Type id this live instance can discover, or null when unsupported.
    discoverable_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateInstanceRequest {
    connector_type: String,
    name: String,
    /// Absent means "no configuration", which is what an unfilled form sends.
    #[serde(default)]
    config: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInstanceRequest {
    /// Absent leaves it alone.
    name: Option<String>,
    /// Absent leaves it alone. Present **replaces** the whole configuration —
    /// a connector is rebuilt from its config wholesale, so there is no
    /// coherent meaning for a partial one.
    config: Option<Value>,
    /// Three states, which is why it is a nested `Option`: **absent** leaves the
    /// override alone, **`null`** clears it back to the connector type's own
    /// icon, and a **string** sets it. A flat `Option<String>` would make
    /// "clear it" and "do not touch it" the same request, leaving no way to
    /// undo a choice.
    #[serde(default, deserialize_with = "present_or_absent")]
    icon_override: Option<Option<String>>,
}

/// Distinguishes an absent JSON field from one explicitly set to `null`.
///
/// `#[serde(default)]` alone cannot: `Option<Option<T>>` collapses `null` to
/// the outer `None`, which is the same value an absent field produces. Running
/// the inner `Option` through its own deserializer and wrapping the result in
/// `Some` keeps the two apart, because this function is only *called* when the
/// field is present.
fn present_or_absent<'de, D>(deserializer: D) -> Result<Option<Option<String>>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<String>::deserialize(deserializer).map(Some)
}

/// The stored columns of one instance.
#[derive(sqlx::FromRow)]
struct InstanceRow {
    id: String,
    connector_type: String,
    name: String,
    config: String,
    created_at: String,
    icon_override: Option<String>,
}

/// `GET /connector-instances`
///
/// Requires a **global** `connectors.view` grant.
///
/// Worth noting what that excludes: a user granted `connectors.view` scoped to
/// a single connector is refused here rather than shown a one-element list. The
/// alternative — accept any `connectors.view` grant and filter the response to
/// what the caller may see — is friendlier and is what this should become once
/// scoped view grants are actually issued by something. It is not built yet
/// because a filter with no way to create the case it filters is a feature that
/// cannot be tested against reality.
pub async fn list_instances(
    _caller: RequirePermission<ConnectorsView>,
    State(state): State<AppState>,
) -> Response {
    let rows = sqlx::query_as::<_, InstanceRow>(
        "SELECT id, connector_type, name, config, created_at, icon_override \
         FROM connector_instances ORDER BY name",
    )
    .fetch_all(&state.pool)
    .await;

    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => return internal_error("listing connector instances", error),
    };

    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        // A row with no live connector is a row that failed to load at startup
        // — an unregistered type, or configuration the factory refused. It is
        // still listed, because hiding it would leave a user with a connector
        // they cannot see and therefore cannot delete.
        let (live, snapshot) = match Uuid::parse_str(&row.id) {
            Ok(id) => (
                state.connectors.get(&id).await,
                state.connectors.cached_status(&id).await,
            ),
            Err(_) => (None, None),
        };

        entries.push(entry_for(&row, live.as_deref(), snapshot.as_ref()));
    }

    Json(entries).into_response()
}

/// `GET /connector-instances/{id}`
///
/// Requires a **global** `connectors.view` grant, for the same reason the list
/// does.
pub async fn get_instance(
    _caller: RequirePermission<ConnectorsView>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let row = match load_row(&state, &id).await {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(&id),
        Err(response) => return *response,
    };

    let (live, snapshot) = match Uuid::parse_str(&row.id) {
        Ok(uuid) => (
            state.connectors.get(&uuid).await,
            state.connectors.cached_status(&uuid).await,
        ),
        Err(_) => (None, None),
    };

    detail_for(&row, live.as_deref(), snapshot.as_ref()).await
}

/// `POST /connector-instances/{id}/discover`
///
/// Discovery is a management operation: its suggestions are intended to lead
/// to creating connector instances, so it uses the same permission tier as
/// instance creation rather than the read-only instance detail permission.
pub async fn discover_instance(
    _caller: RequirePermission<ConnectorsManage>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let row = match load_row(&state, &id).await {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(&id),
        Err(response) => return *response,
    };

    let Ok(uuid) = Uuid::parse_str(&row.id) else {
        return not_found(&id);
    };
    let Some(connector) = state.connectors.get(&uuid).await else {
        return ErrorBody::message(
            StatusCode::BAD_REQUEST,
            "discovery is unavailable because this connector instance is not loaded",
        );
    };
    if connector.discoverable_type().is_none() {
        return ErrorBody::message(
            StatusCode::BAD_REQUEST,
            "discovery is not supported for this connector instance",
        );
    }

    match connector.discover().await {
        Ok(resources) => Json(resources).into_response(),
        Err(error) => ErrorBody::connector(status_for(&error), error),
    }
}

/// Load the same cached connector summary used by the public list endpoint.
///
/// Dashboard placements embed this value. Keeping construction here prevents
/// their API from growing a subtly different interpretation of unloaded
/// connectors or cached poll failures.
pub(crate) async fn instance_summary(
    state: &AppState,
    id: &str,
) -> RouteResult<Option<ConnectorInstanceResponse>> {
    let Some(row) = load_row(state, id).await? else {
        return Ok(None);
    };

    let (live, snapshot) = match Uuid::parse_str(&row.id) {
        Ok(uuid) => (
            state.connectors.get(&uuid).await,
            state.connectors.cached_status(&uuid).await,
        ),
        Err(_) => (None, None),
    };

    Ok(Some(entry_for(&row, live.as_deref(), snapshot.as_ref())))
}

/// `POST /connector-instances`
///
/// Requires a global `connectors.manage` grant.
///
/// Construction happens **before** the insert, and the connector's own
/// objection is what a rejected configuration reports. This is real validation
/// rather than a shape check against the published schema: only the connector
/// knows that `baseLoad` has to be a percentage, and a row that the factory
/// would refuse must never reach the database, or it would be silently skipped
/// at the next startup.
pub async fn create_instance(
    _caller: RequirePermission<ConnectorsManage>,
    State(state): State<AppState>,
    Json(request): Json<CreateInstanceRequest>,
) -> Response {
    let name = request.name.trim();
    if name.is_empty() {
        return ErrorBody::message(StatusCode::BAD_REQUEST, "name must not be empty".to_owned());
    }

    let connector = match state
        .connectors
        .build(&request.connector_type, request.config.clone())
        .await
    {
        Ok(connector) => connector,
        Err(error) => return build_failure(error),
    };

    let id = Uuid::new_v4();
    let created_at = Utc::now().to_rfc3339();
    let config = request.config.to_string();

    let inserted = sqlx::query(
        "INSERT INTO connector_instances (id, connector_type, name, config, created_at) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(&request.connector_type)
    .bind(name)
    .bind(&config)
    .bind(&created_at)
    .execute(&state.pool)
    .await;

    if let Err(error) = inserted {
        return internal_error("creating a connector instance", error);
    }

    // Only after the row is durable: a live connector with no row behind it
    // would vanish on restart, and would not be deletable through the API.
    state.connectors.insert(id, connector.clone()).await;
    let snapshot = state.connectors.cached_status(&id).await;

    let row = InstanceRow {
        id: id.to_string(),
        connector_type: request.connector_type,
        name: name.to_owned(),
        config,
        created_at,
        // A new instance has no override; it inherits its type's icon until
        // someone chooses otherwise. Create deliberately takes no `iconOverride`
        // — an instance has to exist before there is anything to distinguish it
        // from, and one field with one place to set it is one fewer way to
        // disagree with itself.
        icon_override: None,
    };

    let body = detail_for(&row, Some(connector.as_ref()), snapshot.as_ref()).await;
    (StatusCode::CREATED, body).into_response()
}

/// `PATCH /connector-instances/{id}`
///
/// Requires a global `connectors.manage` grant. A new configuration is
/// validated by rebuilding the connector, exactly as create does; the live
/// entry is only replaced once the new row is written.
pub async fn update_instance(
    _caller: RequirePermission<ConnectorsManage>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateInstanceRequest>,
) -> Response {
    let row = match load_row(&state, &id).await {
        Ok(Some(row)) => row,
        Ok(None) => return not_found(&id),
        Err(response) => return *response,
    };

    let name = match request.name.as_deref().map(str::trim) {
        Some("") => {
            return ErrorBody::message(StatusCode::BAD_REQUEST, "name must not be empty".to_owned())
        }
        Some(name) => name.to_owned(),
        None => row.name.clone(),
    };

    // Rebuilt whether or not the config changed: the new connector replaces the
    // live one, and building from the stored value is how an instance that
    // failed to load at startup gets a second chance once its config is fixed.
    let config = request.config.clone().unwrap_or_else(|| {
        serde_json::from_str(&row.config).unwrap_or(Value::Object(Default::default()))
    });

    let connector = match state
        .connectors
        .build(&row.connector_type, config.clone())
        .await
    {
        Ok(connector) => connector,
        Err(error) => return build_failure(error),
    };

    // Absent leaves the stored value; `null` clears it; a string sets it. See
    // `UpdateInstanceRequest::icon_override` for why that needs three states.
    let icon_override = match request.icon_override {
        Some(next) => next
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty()),
        None => row.icon_override.clone(),
    };

    let serialized = config.to_string();
    let updated = sqlx::query(
        "UPDATE connector_instances SET name = ?, config = ?, icon_override = ? WHERE id = ?",
    )
    .bind(&name)
    .bind(&serialized)
    .bind(&icon_override)
    .bind(&row.id)
    .execute(&state.pool)
    .await;

    if let Err(error) = updated {
        return internal_error("updating a connector instance", error);
    }

    if let Ok(uuid) = Uuid::parse_str(&row.id) {
        state.connectors.insert(uuid, connector.clone()).await;
    }

    let row = InstanceRow {
        name,
        config: serialized,
        icon_override,
        ..row
    };

    let snapshot = match Uuid::parse_str(&row.id) {
        Ok(uuid) => state.connectors.cached_status(&uuid).await,
        Err(_) => None,
    };

    detail_for(&row, Some(connector.as_ref()), snapshot.as_ref()).await
}

/// `DELETE /connector-instances/{id}`
///
/// Requires a global `connectors.manage` grant.
///
/// Dashboard placements reference the instance with `ON DELETE CASCADE`, so
/// they are removed with it. The dashboards themselves remain intact.
pub async fn delete_instance(
    _caller: RequirePermission<ConnectorsManage>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let deleted = sqlx::query("DELETE FROM connector_instances WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await;

    let deleted = match deleted {
        Ok(result) => result,
        Err(error) => return internal_error("deleting a connector instance", error),
    };

    if deleted.rows_affected() == 0 {
        return not_found(&id);
    }

    if let Ok(uuid) = Uuid::parse_str(&id) {
        state.connectors.remove(&uuid).await;
    }

    // Placements cascade away with the instance, and a cascade cannot know that
    // a dashboard tile group is only a group while it has two members. Without
    // this, deleting a connector could leave a one-member group standing on
    // someone else's dashboard — a tile that cannot be dragged and has no
    // remaining reason to exist. See `dashboards::dissolve_undersized_groups`.
    if let Err(error) = super::dashboards::dissolve_undersized_groups(&state.pool).await {
        return internal_error("dissolving undersized placement groups", error);
    }

    StatusCode::NO_CONTENT.into_response()
}

/// `POST /connector-instances/{id}/actions/{actionId}`
///
/// Requires `connectors.control` over this specific instance. A global grant
/// covers every connector; a grant scoped to `connector:<id>` covers only that
/// one.
///
/// The check runs **before** the instance is looked up, so a caller without
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

    let connector = match Uuid::parse_str(&id) {
        Ok(uuid) => state.connectors.get(&uuid).await,
        Err(_) => None,
    };
    let Some(connector) = connector else {
        return not_found(&id);
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

    // A disruptive action makes the service stop answering for a while, and a
    // poll landing in that window would report a perfectly accurate outage. The
    // marker goes up *before* the request is dispatched, because the gap
    // between sending it and recording it is exactly where that spurious Down
    // would be observed.
    //
    // Looked up from the connector's own action list rather than from a name
    // this route recognises: which actions are disruptive is the connector
    // author's judgement, and hardcoding "restart" here would be right for
    // Docker and wrong for the next connector that calls it "recreate".
    let disruptive = connector
        .actions()
        .await
        .into_iter()
        .find(|action| action.id == action_id)
        .filter(|action| action.is_disruptive)
        .map(|action| action.label);

    let instance_id = Uuid::parse_str(&id).ok();
    if let (Some(uuid), Some(label)) = (instance_id, disruptive.as_ref()) {
        state.connectors.begin_operation(uuid, label).await;
    }

    let outcome = connector.execute_action(&action_id, params).await;

    if let Some(uuid) = instance_id {
        if disruptive.is_some() {
            // Cleared whatever happened: a restart that failed is not still
            // being performed. The safety net in the runtime covers the case
            // where `execute_action` never returns at all.
            state.connectors.end_operation(uuid).await;
        }
        // Every action, not only disruptive ones — pressing a button is the
        // strongest signal available that the state is about to change and
        // that somebody is watching. This also undoes any poll backoff, so a
        // connector that had drifted out to a two-minute interval reports its
        // new state immediately rather than when its turn next comes round.
        state.connectors.refresh_now(uuid).await;
    }

    match outcome {
        Ok(result) => Json(result).into_response(),
        Err(error) => ErrorBody::connector(status_for(&error), error),
    }
}

/* ------------------------------------------------------------------ */
/* Helpers                                                             */
/* ------------------------------------------------------------------ */

/// Reads one row, mapping "no such row" to `Ok(None)` and a database failure to
/// a ready-made 500.
async fn load_row(state: &AppState, id: &str) -> RouteResult<Option<InstanceRow>> {
    sqlx::query_as::<_, InstanceRow>(
        "SELECT id, connector_type, name, config, created_at, icon_override \
         FROM connector_instances WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|error| Box::new(internal_error("loading a connector instance", error)))
}

/// The 404 body, phrased identically wherever it comes from.
fn not_found(id: &str) -> Response {
    ErrorBody::message(
        StatusCode::NOT_FOUND,
        format!("no such connector instance: {id}"),
    )
}

/// Turns a construction failure into the right refusal.
///
/// Both are 400 rather than 404: the request as a whole is malformed, and a 404
/// on an unknown `connectorType` would suggest the *instance* was not found.
fn build_failure(error: BuildError) -> Response {
    match error {
        BuildError::UnknownType(type_id) => ErrorBody::message(
            StatusCode::BAD_REQUEST,
            format!("no such connector type: {type_id}"),
        ),
        BuildError::Rejected(error) => ErrorBody::connector(StatusCode::BAD_REQUEST, error),
    }
}

/// Builds a list entry from the last completed status poll.
///
/// `live` is `None` for a row that failed to load at startup. Such a row still
/// appears, with a synthetic metadata block and a `statusError` explaining why
/// there is nothing behind it — otherwise a broken connector would be invisible
/// and therefore undeletable.
fn entry_for(
    row: &InstanceRow,
    live: Option<&dyn Connector>,
    snapshot: Option<&ConnectorStatusSnapshot>,
) -> ConnectorInstanceResponse {
    let (pending_operation, diagnosis) = snapshot.map_or((None, None), |value| {
        (value.pending_operation.clone(), value.diagnosis.clone())
    });

    let (metadata, status, status_error, display_fields) = match live {
        Some(connector) => (
            connector.metadata(),
            snapshot.and_then(|value| value.status.clone()),
            snapshot
                .and_then(|value| value.status_error.clone())
                .or_else(|| {
                    snapshot.is_none().then(|| {
                        ConnectorError::Internal(
                            "this instance has not completed its first status poll".to_owned(),
                        )
                    })
                }),
            connector.display_fields(),
        ),
        None => (
            unloaded_metadata(row),
            None,
            Some(ConnectorError::Internal(format!(
                "this instance was not loaded: no connector of type {} could be built from its \
                 stored configuration",
                row.connector_type
            ))),
            Vec::new(),
        ),
    };

    ConnectorInstanceResponse {
        id: row.id.clone(),
        name: row.name.clone(),
        connector_type: row.connector_type.clone(),
        created_at: row.created_at.clone(),
        metadata,
        icon_override: row.icon_override.clone(),
        status,
        status_error,
        pending_operation,
        diagnosis,
        display_fields,
    }
}

/// Builds the full detail response for one row.
async fn detail_for(
    row: &InstanceRow,
    live: Option<&dyn Connector>,
    snapshot: Option<&ConnectorStatusSnapshot>,
) -> Response {
    let instance = entry_for(row, live, snapshot);

    let (actions, data_points, default_layout, discoverable_type) = match live {
        Some(connector) => (
            connector.actions().await,
            connector.data_points(),
            connector.default_layout(),
            connector.discoverable_type(),
        ),
        None => (Vec::new(), Vec::new(), WidgetLayout::default(), None),
    };

    Json(ConnectorInstanceDetail {
        instance,
        config: serde_json::from_str(&row.config).unwrap_or(Value::Null),
        actions,
        data_points,
        default_layout,
        discoverable_type,
    })
    .into_response()
}

/// Stand-in metadata for an instance with no live connector behind it.
///
/// Enough for a client to render a row and offer a delete button, and honest
/// about there being nothing there.
fn unloaded_metadata(row: &InstanceRow) -> ConnectorMetadata {
    ConnectorMetadata {
        id: row.connector_type.clone(),
        name: row.name.clone(),
        icon: None,
        version: "unknown".to_owned(),
        min_size: (1, 1),
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
        ConnectorError::InvalidParams { .. } | ConnectorError::InvalidConfig { .. } => {
            StatusCode::BAD_REQUEST
        }
        ConnectorError::AuthFailed { .. } | ConnectorError::Unreachable { .. } => {
            StatusCode::BAD_GATEWAY
        }
        ConnectorError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
    }
}
