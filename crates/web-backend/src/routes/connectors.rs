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
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use loom_core::connector::{
    ActionResult, ColumnDescriptor, ColumnValueType, Connector, ConnectorAction, ConnectorError,
    ConnectorMetadata, ConnectorStatus, DataPointDescriptor, DisplayField, ResourceItem,
    ResourceKindDescriptor, SetupGuide, WidgetLayout,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
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
    discovery_target_field: Option<String>,
}

/// Discovery proposals plus the candidate field they may fill directly.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryResponse {
    discovery_target_field: Option<String>,
    resources: Vec<loom_core::connector::DiscoveredResource>,
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
            discovery_target_field: registration.discovery_target_field.clone(),
        })
        .collect();

    // The registry is a `HashMap`, whose iteration order varies per process.
    // Sorted so a client's type picker does not reshuffle between restarts.
    types.sort_by(|a, b| a.display_name.cmp(&b.display_name));

    Json(types)
}

/// `POST /connector-types/{type_id}/discover`
///
/// Builds a connector from a candidate configuration, uses it for one
/// discovery pass, and discards it. Nothing is added to the runtime or the
/// database, which makes this suitable for filling a generated setup form
/// before an instance exists.
pub async fn discover_type(
    _caller: RequirePermission<ConnectorsManage>,
    State(state): State<AppState>,
    Path(type_id): Path<String>,
    Json(config): Json<Value>,
) -> Response {
    let connector = match state.connectors.build(&type_id, config).await {
        Ok(connector) => connector,
        Err(error) => return build_failure(error),
    };

    discover_with(connector.as_ref(), "this configuration").await
}

/// `POST /connector-types/{type_id}/test-connection`
///
/// Builds a connector from a candidate configuration, asks it for a one-shot
/// reachability and capability report, and discards it. As with type-scoped
/// discovery, neither the database nor the live runtime map is touched.
pub async fn test_type_connection(
    _caller: RequirePermission<ConnectorsManage>,
    State(state): State<AppState>,
    Path(type_id): Path<String>,
    Json(config): Json<Value>,
) -> Response {
    let connector = match state
        .connectors
        .build_for_connection_test(&type_id, config)
        .await
    {
        Ok(connector) => connector,
        Err(error) => return build_failure(error),
    };

    Json(connector.test_connection().await).into_response()
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
    /// Free-form administrator labels, sorted alphabetically.
    tags: Vec<String>,
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
    /// Whether this instance exposes addressable views below itself.
    supports_sub_targets: bool,
    /// Type id this live instance can discover, or null when unsupported.
    discoverable_type: Option<String>,
    /// Whether this connector can be asked if what it manages is out of date.
    supports_update_checking: bool,
    /// What the update scheduler last found, keyed by target with `""` for the
    /// instance itself — the same convention `status.details` uses.
    ///
    /// Empty until a check has run, and each entry carries its own
    /// `lastChecked` so a client shows the age of the answer rather than
    /// implying it is current. Beside `status` rather than inside it: a
    /// registry reading is hours old by design and a status reading is
    /// seconds old, and one object carrying both would invite a client to
    /// treat them as equally fresh.
    update_status: HashMap<String, crate::connectors::updates::UpdateStatus>,
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
    /// Absent leaves tags alone; present replaces the complete set.
    tags: Option<Vec<String>>,
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

    let mut tags_by_instance = match load_all_tags(&state).await {
        Ok(tags) => tags,
        Err(response) => return *response,
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

        let tags = tags_by_instance.remove(&row.id).unwrap_or_default();
        entries.push(entry_for(&row, tags, live.as_deref(), snapshot.as_ref()));
    }

    Json(entries).into_response()
}

/// `GET /connector-instances/tags`
///
/// The vocabulary is derived from tags currently in use, so removing the last
/// assignment removes that suggestion without maintaining a second table.
pub async fn list_tags(
    _caller: RequirePermission<ConnectorsView>,
    State(state): State<AppState>,
) -> Response {
    let tags = sqlx::query_scalar::<_, String>(
        "SELECT DISTINCT tag FROM connector_instance_tags ORDER BY tag COLLATE NOCASE, tag",
    )
    .fetch_all(&state.pool)
    .await;

    match tags {
        Ok(tags) => Json(tags).into_response(),
        Err(error) => internal_error("listing connector tags", error),
    }
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

    let tags = match load_instance_tags(&state, &row.id).await {
        Ok(tags) => tags,
        Err(response) => return *response,
    };

    let update_status = match Uuid::parse_str(&row.id) {
        Ok(uuid) => state.updates.statuses_for(&uuid).await.unwrap_or_default(),
        Err(_) => HashMap::new(),
    };

    detail_for(
        &row,
        tags,
        live.as_deref(),
        snapshot.as_ref(),
        update_status,
    )
    .await
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
    discover_with(connector.as_ref(), "this connector instance").await
}

/// `GET /connector-instances/{id}/sub-targets`
///
/// A cheap live enumeration of addressable views inside one instance. This is
/// read-only metadata and therefore uses `connectors.view`, unlike discovery,
/// which proposes creating new instances and requires management authority.
pub async fn list_sub_targets(
    _caller: RequirePermission<ConnectorsView>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let connector = match live_connector(&state, &id, "sub-targets").await {
        Ok(connector) => connector,
        Err(response) => return *response,
    };
    if !connector.supports_sub_targets() {
        return ErrorBody::message(
            StatusCode::BAD_REQUEST,
            "this connector instance does not support sub-targets",
        );
    }

    match connector.list_sub_targets().await {
        Ok(targets) => Json(targets).into_response(),
        Err(error) => ErrorBody::connector(status_for(&error), error),
    }
}

/// The kind id of the platform-provided "recently updated" table.
///
/// **Provided by the backend, not by the connector**, and that is a deliberate
/// exception to the resource-browser's usual rule that kinds are connector-
/// declared. The rows are the *action log's* — who applied which update, when,
/// and what it replaced — and the action log is platform state that no
/// connector can see or should be given. A connector reaching into Loom's
/// database to fill a table would invert the dependency the whole architecture
/// rests on.
///
/// It is offered for any instance whose connector reports
/// [`Connector::supports_update_checking`], because that is exactly the set of
/// connectors for which "what did we update, and what was it before?" is a
/// question with answers.
pub const RESOURCE_KIND_RECENTLY_UPDATED: &str = "recentlyUpdated";

/// The action whose log entries the table is built from.
const APPLY_UPDATE_ACTION: &str = crate::connectors::updates::APPLY_UPDATE_ACTION;

/// How many past updates the table shows.
const RECENTLY_UPDATED_LIMIT: i64 = 25;

/// The platform's own resource kinds for one instance.
///
/// Appended to whatever the connector declares. A connector that browses
/// nothing still gets this one if it supports update checking, and a connector
/// that declares a kind by the same name would shadow nothing — the ids are
/// distinct by construction because this one is not a Docker word.
fn platform_resource_kinds(connector: &dyn Connector) -> Vec<ResourceKindDescriptor> {
    if !connector.supports_update_checking() {
        return Vec::new();
    }

    vec![ResourceKindDescriptor::new(
        RESOURCE_KIND_RECENTLY_UPDATED,
        "Recently updated",
        vec![
            // `targetId`, the platform's name for "which sub-target this row
            // is about", so a row action knows where to go. These rows are keyed
            // by log entry, so the row id cannot stand in for it.
            ColumnDescriptor::new("targetId", "Target", ColumnValueType::Text),
            // Named after `applyUpdate`'s own parameter: a client that can
            // answer a parameter from a same-named column turns this row into a
            // one-click rollback, with no rollback-specific mechanism anywhere.
            ColumnDescriptor::new("targetImageRef", "Was running", ColumnValueType::Text),
            ColumnDescriptor::new("newRef", "Updated to", ColumnValueType::Text),
            ColumnDescriptor::new("appliedAt", "When", ColumnValueType::Timestamp),
            ColumnDescriptor::new("appliedBy", "By", ColumnValueType::Text),
        ],
    )
    .with_row_actions(
        // The rollback, and there is no other rollback. Each row already holds
        // the reference the target was running before that update — recorded by
        // the action log's snapshot mechanism — so going back is this same
        // action invoked with that value. A dedicated `rollback` action would
        // need its own store of previous versions, which is the store this
        // table is reading from.
        // Borrowed from the connector's own descriptors rather than
        // constructed here, so the schema, label and snapshot declaration are
        // the connector author's and cannot drift from the action that will
        // actually run. A connector that offers no such action gets a
        // history table with no buttons, which is still worth reading.
        connector
            .resource_kinds()
            .into_iter()
            .flat_map(|kind| kind.row_actions)
            .find(|action| action.id == APPLY_UPDATE_ACTION)
            .into_iter()
            .collect(),
    )]
}

/// One `applyUpdate` entry, shaped as a browsable row.
///
/// `previousRef` comes from the log entry's **snapshot** — the value the target
/// reported immediately before the action ran — and `newRef` from its params.
/// Neither is stored by this feature: both are there because the action
/// declared `snapshotDataPointIds` and because the log records params. That is
/// the whole rollback mechanism.
async fn recently_updated_rows(
    state: &AppState,
    instance_id: &str,
) -> Result<Vec<ResourceItem>, sqlx::Error> {
    let rows = sqlx::query_as::<_, ActionLogRow>(
        "SELECT log.id, log.action_id, log.target_id, log.params, log.invoked_by_user_id, \
                users.username AS invoked_by_username, log.invoked_by_system, \
                log.invoked_at, log.completed_at, log.success, log.result_message, log.snapshot \
         FROM connector_action_log AS log \
         LEFT JOIN users ON users.id = log.invoked_by_user_id \
         WHERE log.instance_id = ? AND log.action_id = ? AND log.success = 1 \
         ORDER BY log.invoked_at DESC, log.rowid DESC \
         LIMIT ?",
    )
    .bind(instance_id)
    .bind(APPLY_UPDATE_ACTION)
    .bind(RECENTLY_UPDATED_LIMIT)
    .fetch_all(&state.pool)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| {
            let params = parse_stored_json(&row.params);
            let snapshot = row.snapshot.as_deref().map(parse_stored_json);
            let previous = snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.as_object())
                .and_then(|snapshot| snapshot.values().next())
                .and_then(Value::as_str)
                .unwrap_or("unknown")
                .to_owned();

            ResourceItem::new(row.id)
                .with_field("targetId", row.target_id.unwrap_or_default())
                .with_field("targetImageRef", previous)
                .with_field(
                    "newRef",
                    params
                        .get(crate::connectors::updates::TARGET_IMAGE_REF_PARAM)
                        .and_then(Value::as_str)
                        .unwrap_or("unknown"),
                )
                .with_field("appliedAt", row.completed_at.unwrap_or(row.invoked_at))
                .with_field(
                    "appliedBy",
                    if row.invoked_by_system {
                        "Loom (scheduled)".to_owned()
                    } else {
                        row.invoked_by_username
                            .unwrap_or_else(|| "unknown".to_owned())
                    },
                )
        })
        .collect())
}

/// `GET /connector-instances/{id}/resource-kinds`
///
/// The browsable tables this instance publishes, live from the loaded
/// connector. Read-only metadata, so `connectors.view` — the same tier as
/// sub-targets, and deliberately not the management tier: browsing what a
/// service holds is looking at it, not administering Loom.
pub async fn list_resource_kinds(
    _caller: RequirePermission<ConnectorsView>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let connector = match live_connector(&state, &id, "resource kinds").await {
        Ok(connector) => connector,
        Err(response) => return *response,
    };
    let mut kinds = connector.resource_kinds();
    kinds.extend(platform_resource_kinds(connector.as_ref()));
    Json(kinds).into_response()
}

/// Query parameters for a resource listing.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceListQuery {
    /// Optional sub-target to scope the listing to, passed straight through to
    /// the connector. Absent means the instance as a whole.
    #[serde(default)]
    target_id: Option<String>,
}

/// `GET /connector-instances/{id}/resources/{kind}`
///
/// The rows of one browsable table, optionally scoped with `?targetId=`.
///
/// The kind is checked against the connector's *live* descriptors before the
/// listing runs. That matters because an unknown kind and an empty kind are the
/// same answer at the trait level — `Ok(vec![])` — and a user staring at an
/// empty table deserves to know whether they are looking at a service with
/// nothing in it or at a typo. Only the descriptor list can tell them apart, so
/// the check happens here rather than being pushed onto every connector author.
pub async fn list_resources(
    _caller: RequirePermission<ConnectorsView>,
    State(state): State<AppState>,
    Path((id, kind)): Path<(String, String)>,
    Query(query): Query<ResourceListQuery>,
) -> Response {
    let connector = match live_connector(&state, &id, "resources").await {
        Ok(connector) => connector,
        Err(response) => return *response,
    };

    // The platform's own kinds are served from Loom's tables, not from the
    // connector: their rows are the action log's, which no connector can see.
    if kind == RESOURCE_KIND_RECENTLY_UPDATED && connector.supports_update_checking() {
        return match recently_updated_rows(&state, &id).await {
            Ok(rows) => Json(rows).into_response(),
            Err(error) => internal_error("reading recent updates", error),
        };
    }

    if !connector
        .resource_kinds()
        .iter()
        .any(|descriptor| descriptor.kind == kind)
    {
        return ErrorBody::message(
            StatusCode::BAD_REQUEST,
            format!("this connector instance has no resource kind named `{kind}`"),
        );
    }

    let target_id = query
        .target_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    match connector.list_resource_items(&kind, target_id).await {
        Ok(items) => Json(items).into_response(),
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

    let tags = load_instance_tags(state, &row.id).await?;

    Ok(Some(entry_for(
        &row,
        tags,
        live.as_deref(),
        snapshot.as_ref(),
    )))
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

    // A brand-new instance has never been checked, so its update status is
    // empty rather than absent — the same distinction the cache draws.
    let body = detail_for(
        &row,
        Vec::new(),
        Some(connector.as_ref()),
        snapshot.as_ref(),
        HashMap::new(),
    )
    .await;
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

    let replacement_tags = match request.tags {
        Some(tags) => match normalize_tags(tags) {
            Ok(tags) => Some(tags),
            Err(message) => return ErrorBody::message(StatusCode::BAD_REQUEST, message),
        },
        None => None,
    };
    let response_tags = match replacement_tags.as_ref() {
        Some(tags) => tags.clone(),
        None => match load_instance_tags(&state, &row.id).await {
            Ok(tags) => tags,
            Err(response) => return *response,
        },
    };

    let serialized = config.to_string();
    let mut transaction = match state.pool.begin().await {
        Ok(transaction) => transaction,
        Err(error) => return internal_error("starting connector instance update", error),
    };

    let updated = sqlx::query(
        "UPDATE connector_instances SET name = ?, config = ?, icon_override = ? WHERE id = ?",
    )
    .bind(&name)
    .bind(&serialized)
    .bind(&icon_override)
    .bind(&row.id)
    .execute(&mut *transaction)
    .await;

    if let Err(error) = updated {
        return internal_error("updating a connector instance", error);
    }

    if let Some(tags) = replacement_tags {
        if let Err(error) = sqlx::query("DELETE FROM connector_instance_tags WHERE instance_id = ?")
            .bind(&row.id)
            .execute(&mut *transaction)
            .await
        {
            return internal_error("replacing connector tags", error);
        }

        for tag in tags {
            if let Err(error) =
                sqlx::query("INSERT INTO connector_instance_tags (instance_id, tag) VALUES (?, ?)")
                    .bind(&row.id)
                    .bind(tag)
                    .execute(&mut *transaction)
                    .await
            {
                return internal_error("replacing connector tags", error);
            }
        }
    }

    if let Err(error) = transaction.commit().await {
        return internal_error("committing connector instance update", error);
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

    let update_status = match Uuid::parse_str(&row.id) {
        Ok(uuid) => state.updates.statuses_for(&uuid).await.unwrap_or_default(),
        Err(_) => HashMap::new(),
    };

    detail_for(
        &row,
        response_tags,
        Some(connector.as_ref()),
        snapshot.as_ref(),
        update_status,
    )
    .await
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
        // The update cache is keyed by instance id, and ids are not reused —
        // but a cache that keeps growing with every deleted connector is a
        // leak, and one that outlived a *recreated* instance would report a
        // stale update for a connector that has never been checked.
        state.updates.forget(&uuid).await;
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
    let raw: Value = if body.is_empty() {
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

    // New clients send `{ targetId, params }`. For compatibility with the
    // already-shipped non-target action forms, a body without either envelope
    // key remains the action params object itself. If `targetId` is present but
    // `params` is not, the remaining fields are treated as params as well.
    let (target_id, params) = match raw {
        Value::Object(mut object)
            if object.contains_key("targetId") || object.contains_key("params") =>
        {
            let target_id = match object.remove("targetId") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) if !value.trim().is_empty() => Some(value),
                Some(_) => {
                    return ErrorBody::message(
                        StatusCode::BAD_REQUEST,
                        "targetId must be a non-empty string or null",
                    )
                }
            };
            let params = object.remove("params").unwrap_or(Value::Object(object));
            (target_id, params)
        }
        params => (None, params),
    };

    match invoke_action(
        &state,
        connector,
        &id,
        &action_id,
        target_id.as_deref(),
        params,
        ActionActor::User(caller.id()),
    )
    .await
    {
        Ok(result) => Json(result).into_response(),
        Err(ActionFailure::UnknownAction(action_id)) => ErrorBody::connector(
            StatusCode::NOT_FOUND,
            ConnectorError::invalid_action(action_id),
        ),
        Err(ActionFailure::Log(error)) => internal_error("recording a connector action", error),
        Err(ActionFailure::Connector(error)) => ErrorBody::connector(status_for(&error), error),
    }
}

/// Why an invocation did not produce an `ActionResult`.
///
/// Three cases rather than one because they are three different faults: the
/// caller named something that does not exist, Loom could not record what it
/// was about to do, or the service could not be reached. The route turns them
/// into three different statuses and the scheduler logs them differently.
#[derive(Debug)]
pub(crate) enum ActionFailure {
    /// The connector advertises this id nowhere.
    UnknownAction(String),
    /// The audit row could not be written, so nothing was dispatched.
    Log(sqlx::Error),
    /// The connector could not carry the action out.
    Connector(ConnectorError),
}

/// Runs one connector action: snapshot, record, dispatch, complete.
///
/// **The single path every action takes**, whoever asked for it. The HTTP route
/// is one caller and the update scheduler is another, and they share this
/// function rather than each doing their own version — which is what makes an
/// automatic update indistinguishable from a manual one in the audit log, in
/// the pending-operation overlay, and in the poll that follows. An automation
/// with its own quieter path is an automation whose actions are invisible
/// exactly when someone is trying to work out what happened overnight.
///
/// Authorization is **not** here. It belongs to the caller: the route checks
/// `connectors.control` against the instance before it gets this far, and the
/// scheduler acts on the instance's own stored configuration, which is an
/// administrator's decision already made.
pub(crate) async fn invoke_action(
    state: &AppState,
    connector: std::sync::Arc<dyn Connector>,
    instance_id: &str,
    action_id: &str,
    target_id: Option<&str>,
    params: Value,
    actor: ActionActor<'_>,
) -> Result<ActionResult, ActionFailure> {
    // What the connector says this action is, looked up across both places it
    // can be advertised: its top-level `actions()` and the row/kind actions of
    // its resource kinds. A resource-browser action is still a connector
    // action — same dispatch, same `connectors.control` requirement, same
    // resource scoping — so recognising it here is all that browsing needed
    // from this endpoint.
    let descriptor = resolve_action(connector.as_ref(), action_id, target_id).await;

    // An id the connector advertises nowhere is rejected before it is
    // dispatched — *unless* the connector currently advertises nothing at all.
    // An empty universe is what a connector reports while its service is
    // unreachable, and answering "unknown action id" to that would be a lie
    // told with a 404. In that case the call goes through and the connector
    // gets to state its real problem.
    let advertises_nothing =
        connector.actions().await.is_empty() && connector.resource_kinds().is_empty();
    if descriptor.is_none() && !advertises_nothing {
        return Err(ActionFailure::UnknownAction(action_id.to_owned()));
    }

    // A disruptive action makes the service stop answering for a while, and a
    // poll landing in that window would report a perfectly accurate outage. The
    // marker goes up *before* the request is dispatched, because the gap
    // between sending it and recording it is exactly where that spurious Down
    // would be observed.
    //
    // Taken from the connector's own descriptor rather than from a name this
    // route recognises: which actions are disruptive is the connector author's
    // judgement, and hardcoding "restart" here would be right for Docker and
    // wrong for the next connector that calls it "recreate".
    let snapshot_ids = descriptor
        .as_ref()
        .map(|action| action.snapshot_data_point_ids.clone())
        .unwrap_or_default();
    let disruptive = descriptor
        .filter(|action| action.is_disruptive)
        .map(|action| action.label);

    let uuid = Uuid::parse_str(instance_id).ok();

    // Everything the audit trail needs is gathered *before* the action runs,
    // and the row is written before it is dispatched. Writing afterwards would
    // lose exactly the invocations most worth having: the one that never
    // returned, and the one that took the process down with it.
    let snapshot = match uuid {
        Some(uuid) => snapshot_for(state, uuid, target_id, &snapshot_ids).await,
        None => None,
    };
    // Fail closed. An action Loom cannot record is an action Loom does not
    // perform: a control plane whose audit trail is best-effort is one where
    // the interesting invocation is the one that went missing.
    let log_id = record_invocation(
        state,
        instance_id,
        action_id,
        target_id,
        &params,
        actor,
        snapshot.as_ref(),
    )
    .await
    .map_err(ActionFailure::Log)?;

    if let (Some(uuid), Some(label)) = (uuid, disruptive.as_ref()) {
        state.connectors.begin_operation(uuid, label).await;
    }

    let outcome = connector.execute_action(action_id, target_id, params).await;

    if let Some(uuid) = uuid {
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

    // Both arms are recorded, and they are recorded as different things. A
    // service that was reached and declined is `success: false` with its own
    // words; a request that never got there is `success: false` with the
    // transport's. Neither is an absence.
    match outcome {
        Ok(result) => {
            complete_invocation(state, &log_id, result.success, &result.message).await;
            Ok(result)
        }
        Err(error) => {
            complete_invocation(state, &log_id, false, &error.to_string()).await;
            Err(ActionFailure::Connector(error))
        }
    }
}

/* ------------------------------------------------------------------ */
/* Action log                                                          */
/* ------------------------------------------------------------------ */

/// Default number of log rows returned when the caller does not say.
const ACTION_LOG_DEFAULT_LIMIT: i64 = 50;

/// Hard ceiling on that, whatever the caller asks for.
///
/// A cap rather than an error for an over-large `limit`: the caller wanted "as
/// much as possible", and refusing the request teaches them a number they then
/// have to hardcode.
const ACTION_LOG_MAX_LIMIT: i64 = 200;

/// One stored invocation, as it comes back out of the database.
#[derive(Debug, sqlx::FromRow)]
struct ActionLogRow {
    id: String,
    action_id: String,
    target_id: Option<String>,
    params: String,
    invoked_by_user_id: Option<String>,
    invoked_by_username: Option<String>,
    invoked_by_system: bool,
    invoked_at: String,
    completed_at: Option<String>,
    success: Option<bool>,
    result_message: Option<String>,
    snapshot: Option<String>,
}

/// Who invoked an action, named rather than merely identified.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogActor {
    /// `null` for an action Loom invoked itself.
    id: Option<String>,
    /// `null` for a system invocation, or if the user row has gone despite the
    /// foreign key — a log read can never fail on account of an account.
    username: Option<String>,
    /// Whether Loom itself invoked this — today, the update scheduler. Never
    /// true at the same time as `id` is set; the database enforces it.
    system: bool,
}

/// One entry in `GET /connector-instances/{id}/action-log`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogResponse {
    id: String,
    action_id: String,
    target_id: Option<String>,
    /// The parameters as submitted, parsed back into JSON. A row whose stored
    /// text will not parse comes back as a JSON string rather than being
    /// dropped: an audit entry nobody can read is still evidence that
    /// something ran.
    params: Value,
    invoked_by: ActionLogActor,
    invoked_at: String,
    /// `null` while the invocation is still outstanding — or forever, if Loom
    /// never learned the outcome.
    completed_at: Option<String>,
    success: Option<bool>,
    result_message: Option<String>,
    /// The declared data points' values from just before the action ran, or
    /// `null` when the action declared none.
    snapshot: Option<Value>,
}

impl From<ActionLogRow> for ActionLogResponse {
    fn from(row: ActionLogRow) -> Self {
        Self {
            id: row.id,
            action_id: row.action_id,
            target_id: row.target_id,
            params: parse_stored_json(&row.params),
            invoked_by: ActionLogActor {
                id: row.invoked_by_user_id,
                username: row.invoked_by_username,
                system: row.invoked_by_system,
            },
            invoked_at: row.invoked_at,
            completed_at: row.completed_at,
            success: row.success,
            result_message: row.result_message,
            snapshot: row.snapshot.as_deref().map(parse_stored_json),
        }
    }
}

/// Query parameters for the action log.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionLogQuery {
    #[serde(default)]
    action_id: Option<String>,
    #[serde(default)]
    target_id: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

/// `GET /connector-instances/{id}/action-log`
///
/// Requires a global `connectors.view` grant. Newest first, optionally narrowed
/// by `actionId` and `targetId`, at most `limit` rows.
///
/// **`connectors.view`, not `connectors.control`.** Reading the history is
/// looking, not doing, and the people most in need of "what happened to this
/// service?" are exactly the ones without authority to have done it. It sits
/// behind the same global grant as the instance list, so it is not a way for a
/// caller scoped to one connector to learn about another.
///
/// Served straight from the table rather than from the runtime: the log
/// outlives every process that wrote it, which is most of the point.
pub async fn list_action_log(
    _caller: RequirePermission<ConnectorsView>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<ActionLogQuery>,
) -> Response {
    // Existence is checked first so an unknown instance is a 404 rather than an
    // empty list — "nothing has happened here" and "there is no here" are
    // different answers.
    match load_row(&state, &id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found(&id),
        Err(response) => return *response,
    }

    let limit = query
        .limit
        .unwrap_or(ACTION_LOG_DEFAULT_LIMIT)
        .clamp(1, ACTION_LOG_MAX_LIMIT);
    let action_id = trimmed(query.action_id.as_deref());
    let target_id = trimmed(query.target_id.as_deref());

    // Both filters are expressed as `(? IS NULL OR column = ?)` rather than by
    // building SQL per request: one prepared statement, no string assembly
    // anywhere near a user-supplied value.
    let rows = sqlx::query_as::<_, ActionLogRow>(
        "SELECT log.id, log.action_id, log.target_id, log.params, log.invoked_by_user_id, \
                users.username AS invoked_by_username, log.invoked_by_system, \
                log.invoked_at, log.completed_at, \
                log.success, log.result_message, log.snapshot \
         FROM connector_action_log AS log \
         LEFT JOIN users ON users.id = log.invoked_by_user_id \
         WHERE log.instance_id = ? \
           AND (?2 IS NULL OR log.action_id = ?2) \
           AND (?3 IS NULL OR log.target_id = ?3) \
         ORDER BY log.invoked_at DESC, log.rowid DESC \
         LIMIT ?4",
    )
    .bind(&id)
    .bind(action_id)
    .bind(target_id)
    .bind(limit)
    .fetch_all(&state.pool)
    .await;

    match rows {
        Ok(rows) => Json(
            rows.into_iter()
                .map(ActionLogResponse::from)
                .collect::<Vec<_>>(),
        )
        .into_response(),
        Err(error) => internal_error("reading the connector action log", error),
    }
}

/// Who is invoking an action.
///
/// The audit log names one or the other and never both — see the
/// `invoked_by_system` migration for why a reserved "system user" row was not
/// the answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionActor<'a> {
    /// A signed-in caller, by user id.
    User(&'a str),
    /// Loom itself. Today that means the update scheduler.
    System,
}

impl ActionActor<'_> {
    /// `(invoked_by_user_id, invoked_by_system)` as the row stores them.
    fn columns(&self) -> (Option<&str>, bool) {
        match self {
            Self::User(id) => (Some(id), false),
            Self::System => (None, true),
        }
    }
}

/// The values an action declared worth recording, read from the poll cache.
///
/// Deliberately the *cached* reading and not a fresh one. A snapshot must
/// describe the world as it was immediately before the action, and a live call
/// to fetch it would sit between the decision and the dispatch, slowing every
/// action down to take a reading that is no more true than the one already
/// held. Ids the connector never reported are simply absent.
///
/// Returns `None` when the action declared nothing, so "no snapshot was asked
/// for" stays distinct from "a snapshot was asked for and came back empty".
async fn snapshot_for(
    state: &AppState,
    instance_id: Uuid,
    target_id: Option<&str>,
    data_point_ids: &[String],
) -> Option<Value> {
    if data_point_ids.is_empty() {
        return None;
    }

    let status = state
        .connectors
        .cached_status(&instance_id)
        .await
        .and_then(|snapshot| snapshot.status);

    let mut captured = serde_json::Map::new();
    if let Some(status) = status {
        for data_point_id in data_point_ids {
            if let Some(value) = status.data_point_value_for(target_id, data_point_id) {
                captured.insert(data_point_id.clone(), value.clone());
            }
        }
    }
    Some(Value::Object(captured))
}

/// Writes the pending log row for an invocation about to be dispatched.
///
/// Returns the new row's id, which the completion update needs.
async fn record_invocation(
    state: &AppState,
    instance_id: &str,
    action_id: &str,
    target_id: Option<&str>,
    params: &Value,
    actor: ActionActor<'_>,
    snapshot: Option<&Value>,
) -> Result<String, sqlx::Error> {
    let log_id = Uuid::new_v4().to_string();
    let (user_id, system) = actor.columns();
    sqlx::query(
        "INSERT INTO connector_action_log \
             (id, instance_id, action_id, target_id, params, invoked_by_user_id, \
              invoked_by_system, invoked_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&log_id)
    .bind(instance_id)
    .bind(action_id)
    .bind(target_id)
    .bind(params.to_string())
    .bind(user_id)
    .bind(system)
    .bind(Utc::now().to_rfc3339())
    .execute(&state.pool)
    .await?;

    if let Some(snapshot) = snapshot {
        sqlx::query("UPDATE connector_action_log SET snapshot = ? WHERE id = ?")
            .bind(snapshot.to_string())
            .bind(&log_id)
            .execute(&state.pool)
            .await?;
    }

    Ok(log_id)
}

/// Closes out a log row once the action has returned.
///
/// Failures here are logged and swallowed, unlike the insert. The action has
/// already run by this point, and turning a successful restart into a 500
/// because the audit row could not be updated would be a worse outcome than a
/// row that stays pending — which is itself readable as "the outcome was never
/// recorded".
async fn complete_invocation(state: &AppState, log_id: &str, success: bool, result_message: &str) {
    let updated = sqlx::query(
        "UPDATE connector_action_log \
         SET completed_at = ?, success = ?, result_message = ? \
         WHERE id = ?",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(success)
    .bind(result_message)
    .bind(log_id)
    .execute(&state.pool)
    .await;

    if let Err(error) = updated {
        tracing::error!(
            %log_id,
            %error,
            "could not record the outcome of a connector action; the log row stays pending"
        );
    }
}

/// Parses stored JSON text, falling back to the raw text as a JSON string.
fn parse_stored_json(stored: &str) -> Value {
    serde_json::from_str(stored).unwrap_or_else(|_| Value::String(stored.to_owned()))
}

/// A non-empty, trimmed filter value, or `None` for an absent or blank one.
fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/* ------------------------------------------------------------------ */
/* Helpers                                                             */
/* ------------------------------------------------------------------ */

/// Finds the descriptor for one requested action across everything the
/// connector currently offers.
///
/// Two universes, one namespace. An action is either a connector-level
/// operation from [`Connector::actions`], optionally scoped to a sub-target, or
/// a resource-browser operation declared inside one of the connector's resource
/// kinds — a row action or a kind action. Both run through the same
/// `execute_action` call and are therefore the same *kind* of thing; only the
/// place they are advertised differs. Resource actions carry no `target_id` of
/// their own (which row they act on travels as `resourceId` in `params`), so
/// they match on id alone.
///
/// Returns `None` for an id the connector does not currently advertise
/// anywhere. Note *currently*: `actions()` legitimately returns an empty list
/// from a connector whose service is unreachable, which is why the caller must
/// not read `None` as "no such action" on its own.
async fn resolve_action(
    connector: &dyn Connector,
    action_id: &str,
    target_id: Option<&str>,
) -> Option<ConnectorAction> {
    if let Some(action) = connector
        .actions()
        .await
        .into_iter()
        .find(|action| action.id == action_id && action.target_id.as_deref() == target_id)
    {
        return Some(action);
    }

    connector
        .resource_kinds()
        .into_iter()
        .flat_map(|kind| kind.row_actions.into_iter().chain(kind.kind_actions))
        .find(|action| action.id == action_id)
}

/// Resolves a durable instance id to its loaded connector.
///
/// The two-step lookup is not redundant: a row can exist while its connector
/// failed to build, and those are different answers — 404 for "no such
/// instance", 400 for "it is there but nothing is behind it". `subject` names
/// what the caller wanted, so the 400 says which capability is unavailable.
async fn live_connector(
    state: &AppState,
    id: &str,
    subject: &str,
) -> RouteResult<std::sync::Arc<dyn Connector>> {
    let row = match load_row(state, id).await {
        Ok(Some(row)) => row,
        Ok(None) => return Err(Box::new(not_found(id))),
        Err(response) => return Err(response),
    };
    let Ok(uuid) = Uuid::parse_str(&row.id) else {
        return Err(Box::new(not_found(id)));
    };
    match state.connectors.get(&uuid).await {
        Some(connector) => Ok(connector),
        None => Err(Box::new(ErrorBody::message(
            StatusCode::BAD_REQUEST,
            format!("{subject} are unavailable because this connector instance is not loaded"),
        ))),
    }
}

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

async fn load_instance_tags(state: &AppState, id: &str) -> RouteResult<Vec<String>> {
    sqlx::query_scalar::<_, String>(
        "SELECT tag FROM connector_instance_tags WHERE instance_id = ? \
         ORDER BY tag COLLATE NOCASE, tag",
    )
    .bind(id)
    .fetch_all(&state.pool)
    .await
    .map_err(|error| Box::new(internal_error("loading connector tags", error)))
}

async fn load_all_tags(state: &AppState) -> RouteResult<HashMap<String, Vec<String>>> {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT instance_id, tag FROM connector_instance_tags \
         ORDER BY instance_id, tag COLLATE NOCASE, tag",
    )
    .fetch_all(&state.pool)
    .await
    .map_err(|error| Box::new(internal_error("loading connector tags", error)))?;

    let mut tags_by_instance: HashMap<String, Vec<String>> = HashMap::new();
    for (instance_id, tag) in rows {
        tags_by_instance.entry(instance_id).or_default().push(tag);
    }
    Ok(tags_by_instance)
}

fn normalize_tags(tags: Vec<String>) -> Result<Vec<String>, String> {
    let mut normalized = BTreeSet::new();
    for tag in tags {
        let tag = tag.trim();
        if tag.is_empty() {
            return Err("tags must not be empty".to_owned());
        }
        normalized.insert(tag.to_owned());
    }
    Ok(normalized.into_iter().collect())
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

async fn discover_with(connector: &dyn Connector, subject: &str) -> Response {
    if connector.discoverable_type().is_none() {
        return ErrorBody::message(
            StatusCode::BAD_REQUEST,
            format!("discovery is not supported for {subject}"),
        );
    }

    match connector.discover().await {
        Ok(resources) => Json(DiscoveryResponse {
            discovery_target_field: connector.discovery_target_field(),
            resources,
        })
        .into_response(),
        Err(error) => ErrorBody::connector(status_for(&error), error),
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
    tags: Vec<String>,
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
        tags,
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
    tags: Vec<String>,
    live: Option<&dyn Connector>,
    snapshot: Option<&ConnectorStatusSnapshot>,
    update_status: HashMap<String, crate::connectors::updates::UpdateStatus>,
) -> Response {
    let instance = entry_for(row, tags, live, snapshot);

    let (
        actions,
        data_points,
        default_layout,
        supports_sub_targets,
        discoverable_type,
        supports_update_checking,
    ) = match live {
        Some(connector) => (
            connector.actions().await,
            connector.data_points(),
            connector.default_layout(),
            connector.supports_sub_targets(),
            connector.discoverable_type(),
            connector.supports_update_checking(),
        ),
        None => (
            Vec::new(),
            Vec::new(),
            WidgetLayout::default(),
            false,
            None,
            false,
        ),
    };

    Json(ConnectorInstanceDetail {
        instance,
        config: serde_json::from_str(&row.config).unwrap_or(Value::Null),
        actions,
        data_points,
        default_layout,
        supports_sub_targets,
        discoverable_type,
        supports_update_checking,
        update_status,
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
