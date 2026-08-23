//! User-owned dashboards, sharing, pins, and connector placements.
//!
//! Authorization in this module is the dashboard-local ACL from
//! [`crate::dashboard_access`], never the administrative permission grants in
//! access-token claims. The caller still needs a valid JWT to identify them,
//! but no `dashboards.*` permission exists or should be invented.

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use loom_core::connector::WidgetBinding;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::extract::AuthenticatedUser;
use crate::dashboard_access::{get_dashboard_role, DashboardRole};
use crate::error::{internal_error, ErrorBody};
use crate::state::AppState;

use super::connectors::{self, ConnectorInstanceResponse};

/// Internal helpers return complete HTTP failures. Boxing keeps the uncommon
/// error path small enough for Clippy's `result_large_err` threshold across
/// Rust releases without changing any response body or status.
type RouteResult<T> = Result<T, Box<Response>>;

#[derive(Debug, sqlx::FromRow)]
struct DashboardRow {
    id: String,
    name: String,
    owner_user_id: String,
    owner_username: String,
    created_at: String,
}

#[derive(Debug, sqlx::FromRow)]
struct PlacementRow {
    id: String,
    connector_instance_id: String,
    position_x: i64,
    position_y: i64,
    width: i64,
    height: i64,
    widget_bindings: String,
    created_at: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum ShareTargetType {
    User,
    Group,
}

impl ShareTargetType {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "group" => Some(Self::Group),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Group => "group",
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "lowercase")]
enum ShareRole {
    View,
    Edit,
}

impl ShareRole {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "view" => Some(Self::View),
            "edit" => Some(Self::Edit),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::View => "view",
            Self::Edit => "edit",
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardSummary {
    id: String,
    name: String,
    role: DashboardRole,
    pinned: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardOwner {
    id: String,
    username: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardDetail {
    id: String,
    name: String,
    owner: DashboardOwner,
    role: DashboardRole,
    created_at: String,
    placements: Vec<PlacementResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlacementResponse {
    id: String,
    connector: ConnectorInstanceResponse,
    position_x: i64,
    position_y: i64,
    width: i64,
    height: i64,
    widget_bindings: Vec<WidgetBinding>,
    created_at: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShareResponse {
    id: String,
    target_type: ShareTargetType,
    target_id: String,
    role: ShareRole,
    resolved_name: String,
    created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateDashboardRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateDashboardRequest {
    name: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreateShareRequest {
    target_type: String,
    target_id: String,
    role: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreatePlacementRequest {
    connector_instance_id: String,
    position_x: i64,
    position_y: i64,
    width: i64,
    height: i64,
    widget_bindings: Option<Vec<WidgetBinding>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdatePlacementRequest {
    position_x: Option<i64>,
    position_y: Option<i64>,
    width: Option<i64>,
    height: Option<i64>,
    widget_bindings: Option<Vec<WidgetBinding>>,
}

/// `GET /dashboards` — every dashboard the authenticated caller can access.
pub(super) async fn list_dashboards(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
) -> Response {
    let rows = sqlx::query_as::<_, (String, String)>(
        "SELECT DISTINCT d.id, d.name \
         FROM dashboards d \
         LEFT JOIN dashboard_shares ds ON ds.dashboard_id = d.id \
         LEFT JOIN user_groups ug \
           ON ds.target_type = 'group' \
          AND ds.target_id = ug.group_id \
          AND ug.user_id = ? \
         WHERE d.owner_user_id = ? \
            OR (ds.target_type = 'user' AND ds.target_id = ?) \
            OR (ds.target_type = 'group' AND ug.user_id IS NOT NULL)",
    )
    .bind(caller.id())
    .bind(caller.id())
    .bind(caller.id())
    .fetch_all(&state.pool)
    .await;

    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => return internal_error("listing accessible dashboards", error),
    };

    let mut dashboards = Vec::with_capacity(rows.len());
    for (id, name) in rows {
        let role = match get_dashboard_role(&state.pool, caller.id(), &id).await {
            Ok(Some(role)) => role,
            Ok(None) => continue,
            Err(error) => return internal_error("resolving a dashboard role", error),
        };
        let pinned = match is_pinned(&state.pool, caller.id(), &id).await {
            Ok(pinned) => pinned,
            Err(error) => return internal_error("reading a dashboard pin", error),
        };
        dashboards.push(DashboardSummary {
            id,
            name,
            role,
            pinned,
        });
    }

    dashboards.sort_by(|left, right| {
        right
            .pinned
            .cmp(&left.pinned)
            .then_with(|| left.name.cmp(&right.name))
    });
    Json(dashboards).into_response()
}

/// `POST /dashboards` — ownership is always the authenticated caller.
pub(super) async fn create_dashboard(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Json(request): Json<CreateDashboardRequest>,
) -> Response {
    let name = request.name.trim();
    if name.is_empty() {
        return bad_request("name must not be empty");
    }

    let id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    if let Err(error) = sqlx::query(
        "INSERT INTO dashboards (id, name, owner_user_id, created_at) VALUES (?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(name)
    .bind(caller.id())
    .bind(&created_at)
    .execute(&state.pool)
    .await
    {
        return internal_error("creating a dashboard", error);
    }

    (
        StatusCode::CREATED,
        Json(DashboardSummary {
            id,
            name: name.to_owned(),
            role: DashboardRole::Owner,
            pinned: false,
        }),
    )
        .into_response()
}

/// `GET /dashboards/{id}` — Viewer or better.
pub(super) async fn get_dashboard(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let role = match require_role(&state.pool, caller.id(), &id, DashboardRole::Viewer).await {
        Ok(role) => role,
        Err(response) => return *response,
    };
    dashboard_detail(&state, &id, role).await
}

/// `PATCH /dashboards/{id}` — only the owner may rename.
pub(super) async fn update_dashboard(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateDashboardRequest>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Owner).await {
        return *response;
    }
    let name = request.name.trim();
    if name.is_empty() {
        return bad_request("name must not be empty");
    }

    if let Err(error) = sqlx::query("UPDATE dashboards SET name = ? WHERE id = ?")
        .bind(name)
        .bind(&id)
        .execute(&state.pool)
        .await
    {
        return internal_error("renaming a dashboard", error);
    }

    dashboard_detail(&state, &id, DashboardRole::Owner).await
}

/// `DELETE /dashboards/{id}` — only the owner may delete.
pub(super) async fn delete_dashboard(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Owner).await {
        return *response;
    }

    if let Err(error) = sqlx::query("DELETE FROM dashboards WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await
    {
        return internal_error("deleting a dashboard", error);
    }

    StatusCode::NO_CONTENT.into_response()
}

/// `POST /dashboards/{id}/pin` — pins only the caller's own list entry.
pub(super) async fn pin_dashboard(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Viewer).await
    {
        return *response;
    }

    if let Err(error) = sqlx::query(
        "INSERT INTO dashboard_pins (user_id, dashboard_id, pinned_at) VALUES (?, ?, ?) \
         ON CONFLICT (user_id, dashboard_id) DO UPDATE SET pinned_at = excluded.pinned_at",
    )
    .bind(caller.id())
    .bind(&id)
    .bind(Utc::now().to_rfc3339())
    .execute(&state.pool)
    .await
    {
        return internal_error("pinning a dashboard", error);
    }

    StatusCode::NO_CONTENT.into_response()
}

/// `DELETE /dashboards/{id}/pin` — removes only the caller's own pin.
pub(super) async fn unpin_dashboard(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Viewer).await
    {
        return *response;
    }

    if let Err(error) =
        sqlx::query("DELETE FROM dashboard_pins WHERE user_id = ? AND dashboard_id = ?")
            .bind(caller.id())
            .bind(&id)
            .execute(&state.pool)
            .await
    {
        return internal_error("unpinning a dashboard", error);
    }

    StatusCode::NO_CONTENT.into_response()
}

/// `GET /dashboards/{id}/shares` — only the owner sees the share list.
pub(super) async fn list_shares(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Owner).await {
        return *response;
    }

    let rows = sqlx::query_as::<_, (String, String, String, String, String, String)>(
        "SELECT ds.id, ds.target_type, ds.target_id, ds.role, ds.created_at, \
                CASE ds.target_type \
                  WHEN 'user' THEN u.username \
                  WHEN 'group' THEN g.name \
                END AS resolved_name \
         FROM dashboard_shares ds \
         LEFT JOIN users u ON ds.target_type = 'user' AND ds.target_id = u.id \
         LEFT JOIN groups g ON ds.target_type = 'group' AND ds.target_id = g.id \
         WHERE ds.dashboard_id = ? \
         ORDER BY resolved_name",
    )
    .bind(&id)
    .fetch_all(&state.pool)
    .await;

    let rows = match rows {
        Ok(rows) => rows,
        Err(error) => return internal_error("listing dashboard shares", error),
    };
    let mut shares = Vec::with_capacity(rows.len());
    for (share_id, target_type, target_id, role, created_at, resolved_name) in rows {
        let (Some(target_type), Some(role)) = (
            ShareTargetType::parse(&target_type),
            ShareRole::parse(&role),
        ) else {
            return ErrorBody::message(
                StatusCode::INTERNAL_SERVER_ERROR,
                "a stored dashboard share is invalid",
            );
        };
        shares.push(ShareResponse {
            id: share_id,
            target_type,
            target_id,
            role,
            resolved_name,
            created_at,
        });
    }

    Json(shares).into_response()
}

/// `POST /dashboards/{id}/shares` — only the owner may share.
pub(super) async fn create_share(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<CreateShareRequest>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Owner).await {
        return *response;
    }

    let Some(target_type) = ShareTargetType::parse(&request.target_type) else {
        return bad_request("targetType must be user or group");
    };
    let Some(role) = ShareRole::parse(&request.role) else {
        return bad_request("role must be view or edit");
    };

    let resolved_name =
        match resolve_share_target(&state.pool, target_type, &request.target_id).await {
            Ok(Some(name)) => name,
            Ok(None) => return bad_request("the share target does not exist"),
            Err(error) => return internal_error("validating a dashboard share target", error),
        };

    let share_id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    let inserted = sqlx::query(
        "INSERT INTO dashboard_shares \
         (id, dashboard_id, target_type, target_id, role, created_at) \
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&share_id)
    .bind(&id)
    .bind(target_type.as_str())
    .bind(&request.target_id)
    .bind(role.as_str())
    .bind(&created_at)
    .execute(&state.pool)
    .await;

    if let Err(error) = inserted {
        if error
            .as_database_error()
            .is_some_and(|database| database.is_unique_violation())
        {
            return ErrorBody::message(
                StatusCode::CONFLICT,
                "that dashboard target is already shared",
            );
        }
        return internal_error("creating a dashboard share", error);
    }

    (
        StatusCode::CREATED,
        Json(ShareResponse {
            id: share_id,
            target_type,
            target_id: request.target_id,
            role,
            resolved_name,
            created_at,
        }),
    )
        .into_response()
}

/// `DELETE /dashboards/{id}/shares/{share_id}` — only the owner may revoke.
pub(super) async fn delete_share(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path((id, share_id)): Path<(String, String)>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Owner).await {
        return *response;
    }

    let deleted = sqlx::query("DELETE FROM dashboard_shares WHERE id = ? AND dashboard_id = ?")
        .bind(&share_id)
        .bind(&id)
        .execute(&state.pool)
        .await;
    match deleted {
        Ok(result) if result.rows_affected() == 0 => ErrorBody::message(
            StatusCode::NOT_FOUND,
            format!("no such dashboard share: {share_id}"),
        ),
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => internal_error("deleting a dashboard share", error),
    }
}

/// `POST /dashboards/{id}/placements` — Editor or Owner.
pub(super) async fn create_placement(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<CreatePlacementRequest>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Editor).await
    {
        return *response;
    }

    let bindings = match validate_placement(
        &state,
        &request.connector_instance_id,
        request.width,
        request.height,
        request.widget_bindings,
        None,
    )
    .await
    {
        Ok(bindings) => bindings,
        Err(response) => return *response,
    };
    let serialized = match serde_json::to_string(&bindings) {
        Ok(value) => value,
        Err(error) => return internal_error("serializing widget bindings", error),
    };

    let placement_id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    if let Err(error) = sqlx::query(
        "INSERT INTO dashboard_placements \
         (id, dashboard_id, connector_instance_id, position_x, position_y, width, height, \
          widget_bindings, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&placement_id)
    .bind(&id)
    .bind(&request.connector_instance_id)
    .bind(request.position_x)
    .bind(request.position_y)
    .bind(request.width)
    .bind(request.height)
    .bind(&serialized)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    {
        return internal_error("creating a dashboard placement", error);
    }

    let row = PlacementRow {
        id: placement_id,
        connector_instance_id: request.connector_instance_id,
        position_x: request.position_x,
        position_y: request.position_y,
        width: request.width,
        height: request.height,
        widget_bindings: serialized,
        created_at,
    };
    match placement_response(&state, row).await {
        Ok(response) => (StatusCode::CREATED, Json(response)).into_response(),
        Err(response) => *response,
    }
}

/// `PATCH /dashboards/{id}/placements/{placement_id}` — Editor or Owner.
pub(super) async fn update_placement(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path((id, placement_id)): Path<(String, String)>,
    Json(request): Json<UpdatePlacementRequest>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Editor).await
    {
        return *response;
    }

    let existing = sqlx::query_as::<_, PlacementRow>(
        "SELECT id, connector_instance_id, position_x, position_y, width, height, \
                widget_bindings, created_at \
         FROM dashboard_placements WHERE id = ? AND dashboard_id = ?",
    )
    .bind(&placement_id)
    .bind(&id)
    .fetch_optional(&state.pool)
    .await;
    let existing = match existing {
        Ok(Some(row)) => row,
        Ok(None) => {
            return ErrorBody::message(
                StatusCode::NOT_FOUND,
                format!("no such dashboard placement: {placement_id}"),
            )
        }
        Err(error) => return internal_error("loading a dashboard placement", error),
    };

    let stored_bindings: Vec<WidgetBinding> = match serde_json::from_str(&existing.widget_bindings)
    {
        Ok(bindings) => bindings,
        Err(error) => return internal_error("reading stored widget bindings", error),
    };
    let width = request.width.unwrap_or(existing.width);
    let height = request.height.unwrap_or(existing.height);
    let bindings = match validate_placement(
        &state,
        &existing.connector_instance_id,
        width,
        height,
        request.widget_bindings,
        Some(stored_bindings),
    )
    .await
    {
        Ok(bindings) => bindings,
        Err(response) => return *response,
    };
    let serialized = match serde_json::to_string(&bindings) {
        Ok(value) => value,
        Err(error) => return internal_error("serializing widget bindings", error),
    };

    let position_x = request.position_x.unwrap_or(existing.position_x);
    let position_y = request.position_y.unwrap_or(existing.position_y);
    if let Err(error) = sqlx::query(
        "UPDATE dashboard_placements \
         SET position_x = ?, position_y = ?, width = ?, height = ?, widget_bindings = ? \
         WHERE id = ? AND dashboard_id = ?",
    )
    .bind(position_x)
    .bind(position_y)
    .bind(width)
    .bind(height)
    .bind(&serialized)
    .bind(&placement_id)
    .bind(&id)
    .execute(&state.pool)
    .await
    {
        return internal_error("updating a dashboard placement", error);
    }

    let row = PlacementRow {
        position_x,
        position_y,
        width,
        height,
        widget_bindings: serialized,
        ..existing
    };
    match placement_response(&state, row).await {
        Ok(response) => Json(response).into_response(),
        Err(response) => *response,
    }
}

/// `DELETE /dashboards/{id}/placements/{placement_id}` — Editor or Owner.
pub(super) async fn delete_placement(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path((id, placement_id)): Path<(String, String)>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Editor).await
    {
        return *response;
    }

    let deleted = sqlx::query("DELETE FROM dashboard_placements WHERE id = ? AND dashboard_id = ?")
        .bind(&placement_id)
        .bind(&id)
        .execute(&state.pool)
        .await;
    match deleted {
        Ok(result) if result.rows_affected() == 0 => ErrorBody::message(
            StatusCode::NOT_FOUND,
            format!("no such dashboard placement: {placement_id}"),
        ),
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => internal_error("deleting a dashboard placement", error),
    }
}

async fn dashboard_detail(state: &AppState, id: &str, role: DashboardRole) -> Response {
    let row = sqlx::query_as::<_, DashboardRow>(
        "SELECT d.id, d.name, d.owner_user_id, u.username AS owner_username, d.created_at \
         FROM dashboards d JOIN users u ON u.id = d.owner_user_id WHERE d.id = ?",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await;
    let row = match row {
        Ok(Some(row)) => row,
        Ok(None) => return forbidden_dashboard(DashboardRole::Viewer),
        Err(error) => return internal_error("loading a dashboard", error),
    };

    let placements = match load_placements(state, id).await {
        Ok(placements) => placements,
        Err(response) => return *response,
    };
    Json(DashboardDetail {
        id: row.id,
        name: row.name,
        owner: DashboardOwner {
            id: row.owner_user_id,
            username: row.owner_username,
        },
        role,
        created_at: row.created_at,
        placements,
    })
    .into_response()
}

async fn load_placements(
    state: &AppState,
    dashboard_id: &str,
) -> RouteResult<Vec<PlacementResponse>> {
    let rows = sqlx::query_as::<_, PlacementRow>(
        "SELECT id, connector_instance_id, position_x, position_y, width, height, \
                widget_bindings, created_at \
         FROM dashboard_placements WHERE dashboard_id = ? \
         ORDER BY position_y, position_x, created_at",
    )
    .bind(dashboard_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|error| Box::new(internal_error("listing dashboard placements", error)))?;

    let mut placements = Vec::with_capacity(rows.len());
    for row in rows {
        placements.push(placement_response(state, row).await?);
    }
    Ok(placements)
}

async fn placement_response(state: &AppState, row: PlacementRow) -> RouteResult<PlacementResponse> {
    let bindings = serde_json::from_str(&row.widget_bindings)
        .map_err(|error| Box::new(internal_error("reading stored widget bindings", error)))?;
    let connector = connectors::instance_summary(state, &row.connector_instance_id)
        .await?
        .ok_or_else(|| {
            Box::new(ErrorBody::message(
                StatusCode::INTERNAL_SERVER_ERROR,
                "a dashboard placement references a missing connector",
            ))
        })?;

    Ok(PlacementResponse {
        id: row.id,
        connector,
        position_x: row.position_x,
        position_y: row.position_y,
        width: row.width,
        height: row.height,
        widget_bindings: bindings,
        created_at: row.created_at,
    })
}

async fn validate_placement(
    state: &AppState,
    connector_id: &str,
    width: i64,
    height: i64,
    requested_bindings: Option<Vec<WidgetBinding>>,
    existing_bindings: Option<Vec<WidgetBinding>>,
) -> RouteResult<Vec<WidgetBinding>> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM connector_instances WHERE id = ?)",
    )
    .bind(connector_id)
    .fetch_one(&state.pool)
    .await
    .map_err(|error| Box::new(internal_error("checking a placement connector", error)))?;
    if !exists {
        return Err(Box::new(bad_request(
            "the connector instance does not exist",
        )));
    }

    let connector = match Uuid::parse_str(connector_id) {
        Ok(id) => state.connectors.get(&id).await,
        Err(_) => None,
    }
    .ok_or_else(|| {
        Box::new(bad_request(
            "the connector instance is not currently available",
        ))
    })?;

    let (minimum_width, minimum_height) = connector.metadata().min_size;
    if width < i64::from(minimum_width) || height < i64::from(minimum_height) {
        return Err(Box::new(ErrorBody::message(
            StatusCode::BAD_REQUEST,
            format!(
                "placement size must be at least {minimum_width}x{minimum_height} for this connector"
            ),
        )));
    }

    let bindings = requested_bindings
        .or(existing_bindings)
        .unwrap_or_else(|| connector.default_layout().bindings);
    // Each binding kind resolves against its own namespace: a display binding
    // names a data point, an action binding names an action. They are both
    // strings and they are not interchangeable, so checking one list for both
    // would either reject valid action bindings or wave through bindings that
    // can never fire.
    let data_point_ids: HashSet<String> = connector
        .data_points()
        .into_iter()
        .map(|point| point.id)
        .collect();
    let action_ids: HashSet<String> = connector
        .actions()
        .await
        .into_iter()
        .map(|action| action.id)
        .collect();

    let mut unknown_data_points: Vec<&str> = Vec::new();
    let mut unknown_actions: Vec<&str> = Vec::new();
    for binding in &bindings {
        match binding {
            WidgetBinding::Display { data_point_id, .. }
                if !data_point_ids.contains(data_point_id) =>
            {
                unknown_data_points.push(data_point_id);
            }
            WidgetBinding::Action { action_id, .. } if !action_ids.contains(action_id) => {
                unknown_actions.push(action_id);
            }
            _ => {}
        }
    }
    for list in [&mut unknown_data_points, &mut unknown_actions] {
        list.sort_unstable();
        list.dedup();
    }

    if !unknown_data_points.is_empty() || !unknown_actions.is_empty() {
        // Named separately, because "unknown data point restart" would send
        // someone looking in the wrong half of the connector.
        let mut problems: Vec<String> = Vec::new();
        if !unknown_data_points.is_empty() {
            problems.push(format!(
                "unknown data points: {}",
                unknown_data_points.join(", ")
            ));
        }
        if !unknown_actions.is_empty() {
            problems.push(format!("unknown actions: {}", unknown_actions.join(", ")));
        }
        return Err(Box::new(ErrorBody::message(
            StatusCode::BAD_REQUEST,
            format!("widget bindings reference {}", problems.join("; ")),
        )));
    }

    // Dashboard visibility and connector control are orthogonal. Storing a
    // placement never grants `connectors.view` or `connectors.control`; action
    // requests still pass through the existing connector endpoint and are
    // checked against the viewer's own grants there.
    Ok(bindings)
}

async fn require_role(
    pool: &SqlitePool,
    user_id: &str,
    dashboard_id: &str,
    required: DashboardRole,
) -> RouteResult<DashboardRole> {
    match get_dashboard_role(pool, user_id, dashboard_id).await {
        Ok(Some(role)) if role.at_least(required) => Ok(role),
        Ok(_) => Err(Box::new(forbidden_dashboard(required))),
        Err(error) => Err(Box::new(internal_error(
            "resolving dashboard access",
            error,
        ))),
    }
}

fn forbidden_dashboard(required: DashboardRole) -> Response {
    let role = match required {
        DashboardRole::Owner => "owner",
        DashboardRole::Editor => "editor",
        DashboardRole::Viewer => "viewer",
    };
    ErrorBody::message(
        StatusCode::FORBIDDEN,
        format!("this action requires at least the dashboard {role} role"),
    )
}

fn bad_request(message: impl Into<String>) -> Response {
    ErrorBody::message(StatusCode::BAD_REQUEST, message)
}

async fn is_pinned(
    pool: &SqlitePool,
    user_id: &str,
    dashboard_id: &str,
) -> Result<bool, sqlx::Error> {
    sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM dashboard_pins WHERE user_id = ? AND dashboard_id = ?)",
    )
    .bind(user_id)
    .bind(dashboard_id)
    .fetch_one(pool)
    .await
}

async fn resolve_share_target(
    pool: &SqlitePool,
    target_type: ShareTargetType,
    target_id: &str,
) -> Result<Option<String>, sqlx::Error> {
    match target_type {
        ShareTargetType::User => {
            sqlx::query_scalar("SELECT username FROM users WHERE id = ?")
                .bind(target_id)
                .fetch_optional(pool)
                .await
        }
        ShareTargetType::Group => {
            sqlx::query_scalar("SELECT name FROM groups WHERE id = ?")
                .bind(target_id)
                .fetch_optional(pool)
                .await
        }
    }
}
