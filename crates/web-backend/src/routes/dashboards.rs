//! User-owned dashboards, sharing, pins, and connector placements.
//!
//! Authorization in this module is the dashboard-local ACL from
//! [`crate::dashboard_access`], never the administrative permission grants in
//! access-token claims. The caller still needs a valid JWT to identify them,
//! but no `dashboards.*` permission exists or should be invented.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use loom_core::connector::WidgetBinding;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::extract::{AuthenticatedUser, ConnectorsControl, Permission};
use crate::dashboard_access::{get_dashboard_role, DashboardRole};
use crate::error::{internal_error, ErrorBody};
use crate::state::AppState;

use super::connectors::{self, ActionFailure, ConnectorInstanceResponse, CONNECTOR_RESOURCE_TYPE};
use super::present_option;

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
    hidden: bool,
}

#[derive(Debug, sqlx::FromRow)]
struct PlacementRow {
    id: String,
    /// `NULL` for a placement that shows nothing and only acts — see
    /// [`PlacementAction`]. Such a placement has no connector to read, no
    /// target, and an empty `widget_bindings`.
    connector_instance_id: Option<String>,
    target_id: Option<String>,
    /// Retained while the placement is grouped, and not read by the renderer
    /// then — the group's bounding box governs instead. This is the geometry
    /// the placement returns to when it is ungrouped, which is what makes
    /// ungrouping lossless.
    position_x: i64,
    position_y: i64,
    width: i64,
    height: i64,
    widget_bindings: String,
    /// JSON-encoded [`PlacementAction`], or `NULL` when clicking this
    /// placement does nothing beyond whatever its widgets already do.
    placement_action: Option<String>,
    /// Static-tile display metadata, `NULL` on any placement that has a
    /// connector to take its name and icon from.
    label: Option<String>,
    icon: Option<String>,
    created_at: String,
    /// `NULL` for a standalone placement. Set together with the row's
    /// `group_order` or not at all — the table's CHECK constraint enforces
    /// that, so this one column is enough to tell membership.
    ///
    /// `group_order` itself is deliberately not a field here: it is a sort key
    /// the queries order by, never a value any Rust code compares.
    group_id: Option<String>,
}

/// The stored columns of one placement group.
#[derive(Debug, sqlx::FromRow)]
struct PlacementGroupRow {
    id: String,
    name: String,
    icon: Option<String>,
    position_x: i64,
    position_y: i64,
    width: i64,
    height: i64,
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

/// What a placement does when the user clicks it.
///
/// **Composed onto the existing placement, not a new kind of placement.** A
/// tile that navigates somewhere is the same row, the same geometry and the
/// same grouping rules as any other; only this column is added. Modelling
/// "button" as its own placement type would have duplicated every one of those
/// rules and would have forbidden the case that motivated the feature — a tile
/// that both *shows* a connector's state and *goes somewhere* when clicked.
///
/// This is a dashboard concept and deliberately not a Core one: nothing in the
/// [`Connector`](loom_core::connector::Connector) trait knows that dashboards
/// exist, and a connector must not be able to name one.
///
/// Internally tagged on `type`, matching the discriminated unions the clients
/// already consume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub(super) enum PlacementAction {
    /// Go to another dashboard. The id is resolved and permission-checked
    /// **twice, independently**: once when the placement is saved (a sanity
    /// check against the editor's own access) and again, authoritatively, on
    /// every click against the person clicking. See `click_placement`.
    Navigate {
        /// The dashboard to open. Verified to exist at save time; verified
        /// again, per viewer, at click time.
        target_dashboard_id: String,
    },
    /// Invoke one pre-configured connector action.
    ///
    /// Carries its own `connector_instance_id` rather than borrowing the
    /// placement's: a tile may display one connector and act on another, and a
    /// connector-less tile can still fire an action.
    ConnectorAction {
        connector_instance_id: String,
        /// The sub-target to act on, or `null` for the instance as a whole.
        target_id: Option<String>,
        action_id: String,
        /// The action's parameters, exactly as `POST
        /// /connector-instances/{id}/actions/{actionId}` would receive them.
        #[serde(default)]
        params: Value,
    },
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DashboardSummary {
    id: String,
    name: String,
    role: DashboardRole,
    pinned: bool,
    /// Presentation only. A hidden dashboard is listed here exactly like any
    /// other and stays fully reachable by id; leaving it out of a sidebar is
    /// the client's decision to make from this flag.
    hidden: bool,
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
    /// See [`DashboardSummary::hidden`].
    hidden: bool,
    /// **Standalone placements only.** A placement that is a member of a group
    /// appears under `placement_groups`, never here, so a client renders one
    /// list of tiles without having to reconstruct the grouping itself.
    placements: Vec<PlacementResponse>,
    /// One entry per group, each carrying its ordered members.
    placement_groups: Vec<PlacementGroupResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlacementResponse {
    id: String,
    /// `null` for a placement that shows nothing and only acts. A client
    /// renders such a tile from its `placementAction` alone.
    connector: Option<ConnectorInstanceResponse>,
    /// Addressed connector sub-target, or null for the aggregate view.
    target_id: Option<String>,
    /// The placement's *standalone* geometry. Ignored by the grid while this
    /// placement is a group member, and preserved so ungrouping restores it.
    position_x: i64,
    position_y: i64,
    width: i64,
    height: i64,
    widget_bindings: Vec<WidgetBinding>,
    /// What clicking this tile does, or `null` for a tile that only displays.
    placement_action: Option<PlacementAction>,
    /// What a static tile says, and the icon it shows. Both `null` whenever
    /// `connector` is set — that tile's name and icon are the connector's.
    label: Option<String>,
    icon: Option<String>,
    created_at: String,
    /// The group this placement belongs to, or `null` when it stands alone.
    /// Redundant for a member nested under its own group, and load-bearing for
    /// the single-placement bodies that `POST` and `PATCH .../placements`
    /// return, where there is no surrounding structure to say so.
    group_id: Option<String>,
}

/// One combined tile: a box on the grid plus the placements drawn inside it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct PlacementGroupResponse {
    id: String,
    name: String,
    icon: Option<String>,
    position_x: i64,
    position_y: i64,
    width: i64,
    height: i64,
    created_at: String,
    /// Two or more, in `group_order`. A group never has fewer — see
    /// `dissolve_undersized_groups`.
    members: Vec<PlacementResponse>,
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
    /// Absent leaves the name unchanged. Optional so a client can flip
    /// `hidden` without having to echo a name back.
    name: Option<String>,
    /// Absent leaves the flag unchanged. Owner-only, the same tier as
    /// renaming — hiding a dashboard changes how it appears to everyone it is
    /// shared with, which is an owner's decision.
    hidden: Option<bool>,
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
    /// Omitted or null makes this a static tile: no connector, no data, no
    /// widget bindings. Such a placement must carry a `placementAction`,
    /// because a tile that neither shows nor does anything is not a tile.
    connector_instance_id: Option<String>,
    target_id: Option<String>,
    position_x: i64,
    position_y: i64,
    width: i64,
    height: i64,
    widget_bindings: Option<Vec<WidgetBinding>>,
    placement_action: Option<PlacementAction>,
    /// Static-tile display metadata. Rejected on a placement that names a
    /// connector, which already has a name and an icon of its own.
    label: Option<String>,
    icon: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CreatePlacementGroupRequest {
    /// At least two, all on this dashboard, none already grouped. Order here
    /// becomes the initial `group_order`.
    placement_ids: Vec<String>,
    /// Optional so older clients can continue to create groups. When omitted,
    /// the server gives the group a stable, useful name based on its initial
    /// member count rather than making presentation text the client's job.
    name: Option<String>,
    icon: Option<String>,
    position_x: i64,
    position_y: i64,
    width: i64,
    height: i64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdatePlacementGroupRequest {
    name: Option<String>,
    /// Absent leaves the icon alone; `null` clears it back to the generic
    /// group icon; a string assigns that icon reference.
    #[serde(default, deserialize_with = "present_option")]
    icon: Option<Option<String>>,
    position_x: Option<i64>,
    position_y: Option<i64>,
    width: Option<i64>,
    height: Option<i64>,
    /// Reorders the existing members. Must name exactly the current
    /// membership — no additions, no removals, no duplicates. Adding and
    /// removing members are their own endpoints, so a reorder that silently
    /// did either would be a second way to do something with different
    /// validation and a different cascade.
    member_order: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct AddPlacementGroupMemberRequest {
    placement_id: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdatePlacementRequest {
    /// Absent leaves the target unchanged; null returns to the aggregate view.
    #[serde(default, deserialize_with = "present_option")]
    target_id: Option<Option<String>>,
    position_x: Option<i64>,
    position_y: Option<i64>,
    width: Option<i64>,
    height: Option<i64>,
    widget_bindings: Option<Vec<WidgetBinding>>,
    /// Absent leaves the click behaviour alone; `null` removes it; an object
    /// replaces it. A connector-less placement may not have it removed — that
    /// would leave a tile with nothing to show and nothing to do.
    #[serde(default, deserialize_with = "present_option")]
    placement_action: Option<Option<PlacementAction>>,
    /// Absent leaves it alone; `null` clears it. Same connector-less-only rule
    /// as create.
    #[serde(default, deserialize_with = "present_option")]
    label: Option<Option<String>>,
    #[serde(default, deserialize_with = "present_option")]
    icon: Option<Option<String>>,
}

/// `GET /dashboards` — every dashboard the authenticated caller can access.
pub(super) async fn list_dashboards(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
) -> Response {
    let rows = sqlx::query_as::<_, (String, String, bool)>(
        "SELECT DISTINCT d.id, d.name, d.hidden \
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
    for (id, name, hidden) in rows {
        let role = match get_dashboard_role(&state.pool, caller.id(), &id).await {
            Ok(Some(role)) => role,
            Ok(None) => continue,
            Err(error) => return internal_error("resolving a dashboard role", error),
        };
        let pinned = match is_pinned(&state.pool, caller.id(), &id).await {
            Ok(pinned) => pinned,
            Err(error) => return internal_error("reading a dashboard pin", error),
        };
        // Hidden dashboards are listed, deliberately. Filtering them out here
        // would also hide them from the pin list, from search, and from the
        // only screen that can unhide one.
        dashboards.push(DashboardSummary {
            id,
            name,
            role,
            pinned,
            hidden,
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
            hidden: false,
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

/// `PATCH /dashboards/{id}` — only the owner may rename or hide.
///
/// Hiding sits at the owner tier alongside renaming rather than at the editor
/// tier, because it changes what every person the dashboard is shared with
/// sees in their own list, and that is not an editor's call.
pub(super) async fn update_dashboard(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateDashboardRequest>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Owner).await {
        return *response;
    }

    let name = match request.name.as_deref().map(str::trim) {
        Some("") => return bad_request("name must not be empty"),
        other => other,
    };

    if let Some(name) = name {
        if let Err(error) = sqlx::query("UPDATE dashboards SET name = ? WHERE id = ?")
            .bind(name)
            .bind(&id)
            .execute(&state.pool)
            .await
        {
            return internal_error("renaming a dashboard", error);
        }
    }

    if let Some(hidden) = request.hidden {
        if let Err(error) = sqlx::query("UPDATE dashboards SET hidden = ? WHERE id = ?")
            .bind(hidden)
            .bind(&id)
            .execute(&state.pool)
            .await
        {
            return internal_error("changing a dashboard's hidden flag", error);
        }
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
///
/// Two shapes share this endpoint. With a `connectorInstanceId` it is the
/// connector tile it has always been. Without one it is a **static tile**: no
/// connector, no target, no widget bindings, and a `placementAction` that is
/// then mandatory, because a tile with nothing to show and nothing to do is a
/// blank rectangle a user cannot remove any meaning from.
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

    let bindings = match request.connector_instance_id.as_deref() {
        Some(connector_id) => {
            if request.label.is_some() || request.icon.is_some() {
                return bad_request(
                    "label and icon belong to a placement with no connectorInstanceId: \
                     a connector tile takes both from its connector",
                );
            }
            match validate_placement(
                &state,
                connector_id,
                request.target_id.as_deref(),
                request.width,
                request.height,
                request.widget_bindings,
                None,
            )
            .await
            {
                Ok(bindings) => bindings,
                Err(response) => return *response,
            }
        }
        None => {
            // There is no connector to check a minimum size against, so the
            // only geometry rule left is the table's own.
            if let Some(response) = reject_bad_placement_box(request.width, request.height) {
                return response;
            }
            if request.target_id.is_some() {
                return bad_request(
                    "targetId requires a connectorInstanceId: there is no connector to address",
                );
            }
            if request.placement_action.is_none() {
                return bad_request(
                    "a placement without a connectorInstanceId must have a placementAction",
                );
            }
            // Nothing to bind, so binding validation is skipped entirely
            // rather than run against an absent connector.
            Vec::new()
        }
    };

    if let Some(action) = &request.placement_action {
        if let Err(response) = validate_placement_action(&state, caller.id(), action).await {
            return *response;
        }
    }

    let serialized = match serde_json::to_string(&bindings) {
        Ok(value) => value,
        Err(error) => return internal_error("serializing widget bindings", error),
    };
    let serialized_action = match encode_placement_action(request.placement_action.as_ref()) {
        Ok(value) => value,
        Err(response) => return *response,
    };

    let label = trimmed_or_none(request.label.as_deref());
    let icon = trimmed_or_none(request.icon.as_deref());

    let placement_id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();
    if let Err(error) = sqlx::query(
        "INSERT INTO dashboard_placements \
         (id, dashboard_id, connector_instance_id, position_x, position_y, width, height, \
          target_id, widget_bindings, placement_action, label, icon, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&placement_id)
    .bind(&id)
    .bind(&request.connector_instance_id)
    .bind(request.position_x)
    .bind(request.position_y)
    .bind(request.width)
    .bind(request.height)
    .bind(&request.target_id)
    .bind(&serialized)
    .bind(&serialized_action)
    .bind(&label)
    .bind(&icon)
    .bind(&created_at)
    .execute(&state.pool)
    .await
    {
        return internal_error("creating a dashboard placement", error);
    }

    let row = PlacementRow {
        id: placement_id,
        connector_instance_id: request.connector_instance_id,
        target_id: request.target_id,
        position_x: request.position_x,
        position_y: request.position_y,
        width: request.width,
        height: request.height,
        widget_bindings: serialized,
        placement_action: serialized_action,
        label,
        icon,
        created_at,
        // A new placement always stands alone. Grouping is a separate,
        // retroactive action — see `create_placement_group`.
        group_id: None,
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

    let existing = match load_placement_row(&state, &id, &placement_id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return ErrorBody::message(
                StatusCode::NOT_FOUND,
                format!("no such dashboard placement: {placement_id}"),
            )
        }
        Err(response) => return *response,
    };

    let stored_bindings: Vec<WidgetBinding> = match serde_json::from_str(&existing.widget_bindings)
    {
        Ok(bindings) => bindings,
        Err(error) => return internal_error("reading stored widget bindings", error),
    };
    let width = request.width.unwrap_or(existing.width);
    let height = request.height.unwrap_or(existing.height);
    let target_id = request
        .target_id
        .unwrap_or_else(|| existing.target_id.clone());

    // Absent leaves what is stored; `null` clears it; a value replaces it.
    let action = match &request.placement_action {
        None => match decode_placement_action(existing.placement_action.as_deref()) {
            Ok(action) => action,
            Err(response) => return *response,
        },
        Some(replacement) => replacement.clone(),
    };

    let bindings = match existing.connector_instance_id.as_deref() {
        Some(connector_id) => {
            if request.label.is_some() || request.icon.is_some() {
                return bad_request(
                    "label and icon belong to a placement with no connectorInstanceId: \
                     a connector tile takes both from its connector",
                );
            }
            match validate_placement(
                &state,
                connector_id,
                target_id.as_deref(),
                width,
                height,
                request.widget_bindings,
                Some(stored_bindings),
            )
            .await
            {
                Ok(bindings) => bindings,
                Err(response) => return *response,
            }
        }
        None => {
            if let Some(response) = reject_bad_placement_box(width, height) {
                return response;
            }
            if target_id.is_some() {
                return bad_request(
                    "targetId requires a connectorInstanceId: there is no connector to address",
                );
            }
            if action.is_none() {
                return bad_request(
                    "a placement without a connectorInstanceId must keep its placementAction",
                );
            }
            if request
                .widget_bindings
                .as_ref()
                .is_some_and(|bindings| !bindings.is_empty())
            {
                return bad_request(
                    "a placement without a connectorInstanceId cannot have widget bindings",
                );
            }
            Vec::new()
        }
    };

    if let Some(action) = &action {
        if let Err(response) = validate_placement_action(&state, caller.id(), action).await {
            return *response;
        }
    }

    let serialized = match serde_json::to_string(&bindings) {
        Ok(value) => value,
        Err(error) => return internal_error("serializing widget bindings", error),
    };
    let serialized_action = match encode_placement_action(action.as_ref()) {
        Ok(value) => value,
        Err(response) => return *response,
    };
    let label = match &request.label {
        None => existing.label.clone(),
        Some(replacement) => trimmed_or_none(replacement.as_deref()),
    };
    let icon = match &request.icon {
        None => existing.icon.clone(),
        Some(replacement) => trimmed_or_none(replacement.as_deref()),
    };

    let position_x = request.position_x.unwrap_or(existing.position_x);
    let position_y = request.position_y.unwrap_or(existing.position_y);
    if let Err(error) = sqlx::query(
        "UPDATE dashboard_placements \
         SET position_x = ?, position_y = ?, width = ?, height = ?, target_id = ?, \
             widget_bindings = ?, placement_action = ?, label = ?, icon = ? \
         WHERE id = ? AND dashboard_id = ?",
    )
    .bind(position_x)
    .bind(position_y)
    .bind(width)
    .bind(height)
    .bind(&target_id)
    .bind(&serialized)
    .bind(&serialized_action)
    .bind(&label)
    .bind(&icon)
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
        target_id,
        widget_bindings: serialized,
        placement_action: serialized_action,
        label,
        icon,
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
        Ok(result) if result.rows_affected() == 0 => {
            return ErrorBody::message(
                StatusCode::NOT_FOUND,
                format!("no such dashboard placement: {placement_id}"),
            )
        }
        Ok(_) => {}
        Err(error) => return internal_error("deleting a dashboard placement", error),
    }

    // Deleting a placement is one of the ways a group loses a member, so the
    // below-two rule applies here exactly as it does on the group's own member
    // endpoint. Removing half of a pair through this route and through that one
    // must leave the same dashboard behind.
    if let Err(error) = dissolve_undersized_groups(&state.pool).await {
        return internal_error("dissolving an undersized placement group", error);
    }

    StatusCode::NO_CONTENT.into_response()
}

/// `POST /dashboards/{id}/placements/{placementId}/click` — Viewer or better.
///
/// Runs the placement's pre-configured [`PlacementAction`]. Viewer is the right
/// tier because clicking is *using* the dashboard, not editing it; what the
/// click is allowed to reach is then checked again against the clicker, which
/// is the whole point of the endpoint existing at all.
///
/// **Two independent permission checks, by design.** Whoever created this
/// placement had their own access to the target verified at save time, and that
/// verdict is not reused here: shares are revoked, group memberships change,
/// and the person clicking is very often not the person who placed the tile.
/// The check that governs is always the fresh one below.
pub(super) async fn click_placement(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path((id, placement_id)): Path<(String, String)>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Viewer).await
    {
        return *response;
    }

    let placement = match load_placement_row(&state, &id, &placement_id).await {
        Ok(Some(row)) => row,
        Ok(None) => {
            return ErrorBody::message(
                StatusCode::NOT_FOUND,
                format!("no such dashboard placement: {placement_id}"),
            )
        }
        Err(response) => return *response,
    };

    let action = match decode_placement_action(placement.placement_action.as_deref()) {
        Ok(Some(action)) => action,
        Ok(None) => return bad_request("this placement has no click action"),
        Err(response) => return *response,
    };

    match action {
        PlacementAction::Navigate {
            target_dashboard_id,
        } => {
            // "Gone" and "not yours" are two different things and are answered
            // as two different things. Collapsing them would tell a user whose
            // access was revoked to go looking for a deleted dashboard, and
            // would tell someone whose target really was deleted to go asking
            // its owner for a share that would not help.
            let exists = sqlx::query_scalar::<_, bool>(
                "SELECT EXISTS(SELECT 1 FROM dashboards WHERE id = ?)",
            )
            .bind(&target_dashboard_id)
            .fetch_one(&state.pool)
            .await;
            match exists {
                Ok(true) => {}
                Ok(false) => {
                    return ErrorBody::message(
                        StatusCode::NOT_FOUND,
                        format!(
                            "the dashboard this tile points at no longer exists: \
                             {target_dashboard_id}"
                        ),
                    )
                }
                Err(error) => return internal_error("resolving a navigate target", error),
            }

            // Evaluated fresh, against the caller, right now — never against
            // whoever configured the tile.
            match get_dashboard_role(&state.pool, caller.id(), &target_dashboard_id).await {
                Ok(Some(_)) => {}
                Ok(None) => {
                    return ErrorBody::message(
                        StatusCode::FORBIDDEN,
                        "you do not have access to the dashboard this tile points at",
                    )
                }
                Err(error) => {
                    return internal_error("resolving access to a navigate target", error)
                }
            }

            // The navigation itself is the client's: this endpoint answers
            // "may you, and where to", and the router does the rest without a
            // page load.
            Json(NavigateResponse {
                target_dashboard_id,
            })
            .into_response()
        }
        PlacementAction::ConnectorAction {
            connector_instance_id,
            target_id,
            action_id,
            params,
        } => {
            // Identical to the direct action endpoint, deliberately: the same
            // resource-scoped grant, the same invocation path, and therefore
            // the same audit row and the same pending-operation overlay. A
            // dashboard tile is a shortcut to that endpoint, never a way
            // around it — being able to see a dashboard has never granted
            // `connectors.control` and does not start to here.
            if let Some(denied) = caller.deny_unless(
                ConnectorsControl::KEY,
                Some(CONNECTOR_RESOURCE_TYPE),
                Some(&connector_instance_id),
            ) {
                return denied;
            }

            let connector = match Uuid::parse_str(&connector_instance_id) {
                Ok(uuid) => state.connectors.get(&uuid).await,
                Err(_) => None,
            };
            let Some(connector) = connector else {
                return connectors::not_found(&connector_instance_id);
            };

            match connectors::invoke_action(
                &state,
                connector,
                &connector_instance_id,
                &action_id,
                target_id.as_deref(),
                params,
                connectors::ActionActor::User(caller.id()),
            )
            .await
            {
                Ok(result) => Json(result).into_response(),
                Err(ActionFailure::UnknownAction(action_id)) => ErrorBody::connector(
                    StatusCode::NOT_FOUND,
                    loom_core::connector::ConnectorError::invalid_action(action_id),
                ),
                Err(ActionFailure::Log(error)) => {
                    internal_error("recording a connector action", error)
                }
                Err(ActionFailure::Connector(error)) => {
                    ErrorBody::connector(connectors::status_for(&error), error)
                }
            }
        }
    }
}

/// The body of a successful `navigate` click.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct NavigateResponse {
    target_dashboard_id: String,
}

/* ------------------------------------------------------------------ */
/* Placement groups                                                    */
/* ------------------------------------------------------------------ */

/// `POST /dashboards/{id}/placement-groups` — Editor or Owner.
///
/// Combines two or more existing placements into one tile. Retroactive by
/// design: nothing about how a placement was created decides whether it can be
/// grouped later, and members need not share a connector type — the group
/// knows only that it holds placements.
///
/// Members keep their own `positionX`/`positionY`/`width`/`height`. Those are
/// ignored by the grid while grouped and are what each placement returns to
/// when it is ungrouped, which is what makes grouping a reversible experiment
/// rather than a decision.
///
/// Rejects, all as 400 because all of them are failures of the request body:
/// fewer than two ids, a repeated id, an id that is not a placement on this
/// dashboard, or an id already in a group. The last names the offenders — "a
/// placement can only be in one group" is not actionable without knowing which.
pub(super) async fn create_placement_group(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<CreatePlacementGroupRequest>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Editor).await
    {
        return *response;
    }

    if let Some(response) = reject_bad_box(request.width, request.height) {
        return response;
    }

    let mut seen: HashSet<&str> = HashSet::new();
    let mut duplicates: Vec<&str> = Vec::new();
    for placement_id in &request.placement_ids {
        if !seen.insert(placement_id.as_str()) {
            duplicates.push(placement_id);
        }
    }
    if !duplicates.is_empty() {
        duplicates.sort_unstable();
        duplicates.dedup();
        return bad_request(format!(
            "placementIds repeats {}; a placement can appear in a group once",
            duplicates.join(", ")
        ));
    }

    // A group of one is the placement it contains, drawn with an extra layer of
    // indirection. Refused here rather than tolerated, because the alternative
    // is a tile whose behaviour differs from a standalone placement in no
    // observable way and which the auto-dissolve rule would delete anyway.
    if request.placement_ids.len() < 2 {
        return bad_request("a placement group needs at least 2 placements");
    }

    let name = match request.name.as_deref() {
        Some(name) if name.trim().is_empty() => return bad_request("name must not be empty"),
        Some(name) => name.trim().to_owned(),
        None => format!("Group of {}", request.placement_ids.len()),
    };

    let (unknown, already_grouped) =
        match classify_candidates(&state, &id, &request.placement_ids).await {
            Ok(split) => split,
            Err(response) => return *response,
        };
    if let Some(response) = reject_unusable_candidates(&unknown, &already_grouped) {
        return response;
    }

    let group_id = Uuid::new_v4().to_string();
    let created_at = Utc::now().to_rfc3339();

    // One transaction: a group row whose members were not all updated is a
    // group that is instantly undersized, and a half-written membership is
    // exactly the state the auto-dissolve rule exists to make impossible.
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => return internal_error("starting a placement group transaction", error),
    };

    if let Err(error) = sqlx::query(
        "INSERT INTO dashboard_placement_groups \
         (id, dashboard_id, name, icon, position_x, position_y, width, height, created_at) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&group_id)
    .bind(&id)
    .bind(&name)
    .bind(&request.icon)
    .bind(request.position_x)
    .bind(request.position_y)
    .bind(request.width)
    .bind(request.height)
    .bind(&created_at)
    .execute(&mut *tx)
    .await
    {
        return internal_error("creating a placement group", error);
    }

    // Input order is the initial order. No collision is possible: every one of
    // these placements was just confirmed ungrouped.
    for (index, placement_id) in request.placement_ids.iter().enumerate() {
        if let Err(error) = sqlx::query(
            "UPDATE dashboard_placements SET group_id = ?, group_order = ? \
             WHERE id = ? AND dashboard_id = ?",
        )
        .bind(&group_id)
        .bind(index as i64)
        .bind(placement_id)
        .bind(&id)
        .execute(&mut *tx)
        .await
        {
            return internal_error("adding a placement to its group", error);
        }
    }

    if let Err(error) = tx.commit().await {
        return internal_error("committing a placement group", error);
    }

    match group_response(&state, &id, &group_id).await {
        Ok(Some(group)) => (StatusCode::CREATED, Json(group)).into_response(),
        Ok(None) => internal_error("reloading a placement group", sqlx::Error::RowNotFound),
        Err(response) => *response,
    }
}

/// `PATCH /dashboards/{id}/placement-groups/{group_id}` — Editor or Owner.
///
/// Moves or resizes the group tile itself, and/or reorders its members. Every
/// field is optional; an empty body is a no-op that returns the group as it
/// stands.
///
/// `memberOrder` must name **exactly** the current membership — same ids, no
/// duplicates, nothing added, nothing missing. Reordering is not a back door
/// for joining or leaving a group: those have their own endpoints, with their
/// own validation and, in the leaving case, a cascade this one must not
/// silently trigger.
pub(super) async fn update_placement_group(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path((id, group_id)): Path<(String, String)>,
    Json(request): Json<UpdatePlacementGroupRequest>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Editor).await
    {
        return *response;
    }

    let existing = match load_group_row(&state, &id, &group_id).await {
        Ok(Some(row)) => row,
        Ok(None) => return no_such_group(&group_id),
        Err(response) => return *response,
    };

    let name = match request.name.as_deref() {
        Some(name) if name.trim().is_empty() => return bad_request("name must not be empty"),
        Some(name) => name.trim().to_owned(),
        None => existing.name.clone(),
    };
    let icon = request.icon.unwrap_or(existing.icon.clone());

    let width = request.width.unwrap_or(existing.width);
    let height = request.height.unwrap_or(existing.height);
    if let Some(response) = reject_bad_box(width, height) {
        return response;
    }
    let position_x = request.position_x.unwrap_or(existing.position_x);
    let position_y = request.position_y.unwrap_or(existing.position_y);

    let reorder = match request.member_order {
        None => None,
        Some(requested) => {
            let current = match member_ids(&state, &group_id).await {
                Ok(ids) => ids,
                Err(response) => return *response,
            };
            let requested_set: HashSet<&str> = requested.iter().map(String::as_str).collect();
            let current_set: HashSet<&str> = current.iter().map(String::as_str).collect();
            if requested.len() != requested_set.len() || requested_set != current_set {
                return bad_request(format!(
                    "memberOrder must list exactly the group's current {} member(s), each once",
                    current.len()
                ));
            }
            Some(requested)
        }
    };

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => return internal_error("starting a placement group transaction", error),
    };

    if let Err(error) = sqlx::query(
        "UPDATE dashboard_placement_groups \
         SET name = ?, icon = ?, position_x = ?, position_y = ?, width = ?, height = ? \
         WHERE id = ? AND dashboard_id = ?",
    )
    .bind(&name)
    .bind(&icon)
    .bind(position_x)
    .bind(position_y)
    .bind(width)
    .bind(height)
    .bind(&group_id)
    .bind(&id)
    .execute(&mut *tx)
    .await
    {
        return internal_error("updating a placement group", error);
    }

    if let Some(order) = reorder {
        // Two passes, because `(group_id, group_order)` is uniquely indexed and
        // that index is checked per statement, not at commit. Any permutation
        // that is not the identity would collide part-way through a single
        // pass. The interim values are negative, which the final 0..n-1 values
        // can never be, so the two passes cannot collide with each other.
        for (index, placement_id) in order.iter().enumerate() {
            if let Err(error) = sqlx::query(
                "UPDATE dashboard_placements SET group_order = ? WHERE id = ? AND group_id = ?",
            )
            .bind(-(index as i64) - 1)
            .bind(placement_id)
            .bind(&group_id)
            .execute(&mut *tx)
            .await
            {
                return internal_error("reordering placement group members", error);
            }
        }
        for (index, placement_id) in order.iter().enumerate() {
            if let Err(error) = sqlx::query(
                "UPDATE dashboard_placements SET group_order = ? WHERE id = ? AND group_id = ?",
            )
            .bind(index as i64)
            .bind(placement_id)
            .bind(&group_id)
            .execute(&mut *tx)
            .await
            {
                return internal_error("reordering placement group members", error);
            }
        }
    }

    if let Err(error) = tx.commit().await {
        return internal_error("committing a placement group update", error);
    }

    match group_response(&state, &id, &group_id).await {
        Ok(Some(group)) => Json(group).into_response(),
        Ok(None) => no_such_group(&group_id),
        Err(response) => *response,
    }
}

/// `POST /dashboards/{id}/placement-groups/{group_id}/members` — Editor or Owner.
///
/// Appends one standalone placement to the group, after the current last
/// member. A placement already in *another* group is refused rather than
/// moved: leaving a group can dissolve it, and a request that says "add" should
/// not be the thing that deletes a different tile. Ungroup it first.
pub(super) async fn add_placement_group_member(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path((id, group_id)): Path<(String, String)>,
    Json(request): Json<AddPlacementGroupMemberRequest>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Editor).await
    {
        return *response;
    }

    match load_group_row(&state, &id, &group_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return no_such_group(&group_id),
        Err(response) => return *response,
    }

    let candidates = std::slice::from_ref(&request.placement_id);
    let (unknown, already_grouped) = match classify_candidates(&state, &id, candidates).await {
        Ok(split) => split,
        Err(response) => return *response,
    };
    if let Some(response) = reject_unusable_candidates(&unknown, &already_grouped) {
        return response;
    }

    // Append past the current maximum rather than at the member count: removals
    // leave gaps, and `count` would land on an order that is already taken.
    let next_order = sqlx::query_scalar::<_, Option<i64>>(
        "SELECT MAX(group_order) FROM dashboard_placements WHERE group_id = ?",
    )
    .bind(&group_id)
    .fetch_one(&state.pool)
    .await;
    let next_order = match next_order {
        Ok(highest) => highest.unwrap_or(-1) + 1,
        Err(error) => return internal_error("reading placement group ordering", error),
    };

    if let Err(error) = sqlx::query(
        "UPDATE dashboard_placements SET group_id = ?, group_order = ? \
         WHERE id = ? AND dashboard_id = ?",
    )
    .bind(&group_id)
    .bind(next_order)
    .bind(&request.placement_id)
    .bind(&id)
    .execute(&state.pool)
    .await
    {
        return internal_error("adding a placement to its group", error);
    }

    match group_response(&state, &id, &group_id).await {
        Ok(Some(group)) => Json(group).into_response(),
        Ok(None) => no_such_group(&group_id),
        Err(response) => *response,
    }
}

/// `DELETE /dashboards/{id}/placement-groups/{group_id}/members/{placement_id}`
/// — Editor or Owner.
///
/// Removes one member. Its `group_id`/`group_order` are cleared and it returns
/// to standalone at the position and size it has been carrying all along.
///
/// # This can delete the group
///
/// **If the removal leaves the group with fewer than two members, the group is
/// dissolved outright**: any remaining member is also returned to standalone
/// and the group row is deleted. Removing one member of a pair therefore
/// un-groups *both* placements and destroys the tile, which is not what
/// "remove a member" sounds like — it is what it has to mean, because a group
/// of one is not a group.
///
/// So the response is 204 and carries nothing. A caller cannot patch its local
/// state from a body here: the number of tiles on the dashboard may have
/// changed, and a placement it was not asking about may have moved. Re-read
/// `GET /dashboards/{id}`.
pub(super) async fn delete_placement_group_member(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path((id, group_id, placement_id)): Path<(String, String, String)>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Editor).await
    {
        return *response;
    }

    // Scoped to the dashboard *and* the group, so a placement that exists but
    // is not in this group is a 404 on this URL rather than a silent no-op.
    let removed = sqlx::query(
        "UPDATE dashboard_placements SET group_id = NULL, group_order = NULL \
         WHERE id = ? AND dashboard_id = ? AND group_id = ?",
    )
    .bind(&placement_id)
    .bind(&id)
    .bind(&group_id)
    .execute(&state.pool)
    .await;
    match removed {
        Ok(result) if result.rows_affected() == 0 => {
            return ErrorBody::message(
                StatusCode::NOT_FOUND,
                format!("placement {placement_id} is not a member of group {group_id}"),
            )
        }
        Ok(_) => {}
        Err(error) => return internal_error("removing a placement group member", error),
    }

    if let Err(error) = dissolve_undersized_groups(&state.pool).await {
        return internal_error("dissolving an undersized placement group", error);
    }

    StatusCode::NO_CONTENT.into_response()
}

/// `DELETE /dashboards/{id}/placement-groups/{group_id}` — Editor or Owner.
///
/// Splits the tile apart in one action: every member returns to standalone at
/// its preserved position and size, and the group row is deleted. No placement
/// is deleted.
///
/// Distinct from removing members one at a time, which for a group of three
/// would take two requests and dissolve the group on the second anyway.
pub(super) async fn delete_placement_group(
    caller: AuthenticatedUser,
    State(state): State<AppState>,
    Path((id, group_id)): Path<(String, String)>,
) -> Response {
    if let Err(response) = require_role(&state.pool, caller.id(), &id, DashboardRole::Editor).await
    {
        return *response;
    }

    match load_group_row(&state, &id, &group_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return no_such_group(&group_id),
        Err(response) => return *response,
    }

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => return internal_error("starting a placement group transaction", error),
    };

    // Membership first. The foreign key has no `ON DELETE` action, so deleting
    // the group while a member still references it fails rather than orphaning
    // anything — the order of these two statements is load-bearing.
    if let Err(error) = sqlx::query(
        "UPDATE dashboard_placements SET group_id = NULL, group_order = NULL WHERE group_id = ?",
    )
    .bind(&group_id)
    .execute(&mut *tx)
    .await
    {
        return internal_error("ungrouping placements", error);
    }
    if let Err(error) =
        sqlx::query("DELETE FROM dashboard_placement_groups WHERE id = ? AND dashboard_id = ?")
            .bind(&group_id)
            .bind(&id)
            .execute(&mut *tx)
            .await
    {
        return internal_error("deleting a placement group", error);
    }

    if let Err(error) = tx.commit().await {
        return internal_error("committing a placement group deletion", error);
    }

    StatusCode::NO_CONTENT.into_response()
}

/// Deletes every group left with fewer than two members, returning each
/// survivor to standalone.
///
/// The rule is enforced here, in one place, rather than at each call site,
/// because membership can fall below two through more than the obvious route:
///
/// - a member is removed from the group;
/// - a member placement is deleted from the dashboard outright;
/// - the **connector instance** behind a member is deleted, which cascades its
///   placements away without this module being involved at all.
///
/// That last one is why `connectors::delete_instance` calls this too. A rule
/// that held on two of its three paths would leave one-member groups on real
/// dashboards, and they would be invisible until someone wondered why a tile
/// could not be dragged.
///
/// Returns the number of groups dissolved, which is only used by tests — the
/// endpoints treat "nothing to do" and "cleaned something up" identically.
pub(super) async fn dissolve_undersized_groups(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let undersized = sqlx::query_scalar::<_, String>(
        "SELECT g.id FROM dashboard_placement_groups g \
         LEFT JOIN dashboard_placements p ON p.group_id = g.id \
         GROUP BY g.id HAVING COUNT(p.id) < 2",
    )
    .fetch_all(pool)
    .await?;

    if undersized.is_empty() {
        return Ok(0);
    }

    let mut tx = pool.begin().await?;
    for group_id in &undersized {
        sqlx::query(
            "UPDATE dashboard_placements SET group_id = NULL, group_order = NULL \
             WHERE group_id = ?",
        )
        .bind(group_id)
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM dashboard_placement_groups WHERE id = ?")
            .bind(group_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    Ok(undersized.len() as u64)
}

/// Sorts requested placement ids into "not a placement on this dashboard" and
/// "already in a group", for the two endpoints that take them from a body.
async fn classify_candidates<'a>(
    state: &AppState,
    dashboard_id: &str,
    placement_ids: &'a [String],
) -> RouteResult<(Vec<&'a str>, Vec<&'a str>)> {
    let mut unknown = Vec::new();
    let mut already_grouped = Vec::new();

    // One query per id rather than a built-up `IN (?, ?, …)`. A group is a
    // handful of tiles, and interpolating a list into SQL to save a few
    // round-trips on a request that happens when a person clicks a button is
    // not a trade worth making.
    for placement_id in placement_ids {
        let existing = sqlx::query_scalar::<_, Option<String>>(
            "SELECT group_id FROM dashboard_placements WHERE id = ? AND dashboard_id = ?",
        )
        .bind(placement_id)
        .bind(dashboard_id)
        .fetch_optional(&state.pool)
        .await
        .map_err(|error| Box::new(internal_error("checking a placement's group", error)))?;

        match existing {
            None => unknown.push(placement_id.as_str()),
            Some(Some(_)) => already_grouped.push(placement_id.as_str()),
            Some(None) => {}
        }
    }

    Ok((unknown, already_grouped))
}

/// The shared 400 for ids that cannot be grouped. Both lists are named, because
/// "some placements are unusable" sends the caller looking through all of them.
fn reject_unusable_candidates(unknown: &[&str], already_grouped: &[&str]) -> Option<Response> {
    let mut problems: Vec<String> = Vec::new();
    if !unknown.is_empty() {
        problems.push(format!(
            "not placements on this dashboard: {}",
            unknown.join(", ")
        ));
    }
    if !already_grouped.is_empty() {
        problems.push(format!(
            "already in a group: {} (ungroup first — a placement can be in one group at a time)",
            already_grouped.join(", ")
        ));
    }
    (!problems.is_empty()).then(|| bad_request(problems.join("; ")))
}

/// Rejects a non-positive tile. The table CHECKs this too; catching it here is
/// what makes it a 400 naming the field instead of a 500 naming a constraint.
fn reject_bad_box(width: i64, height: i64) -> Option<Response> {
    (width < 1 || height < 1)
        .then(|| bad_request("a placement group's width and height must both be at least 1"))
}

async fn load_group_row(
    state: &AppState,
    dashboard_id: &str,
    group_id: &str,
) -> RouteResult<Option<PlacementGroupRow>> {
    sqlx::query_as::<_, PlacementGroupRow>(
        "SELECT id, name, icon, position_x, position_y, width, height, created_at \
         FROM dashboard_placement_groups WHERE id = ? AND dashboard_id = ?",
    )
    .bind(group_id)
    .bind(dashboard_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|error| Box::new(internal_error("loading a placement group", error)))
}

/// The group's current members, in order.
async fn member_ids(state: &AppState, group_id: &str) -> RouteResult<Vec<String>> {
    sqlx::query_scalar::<_, String>(
        "SELECT id FROM dashboard_placements WHERE group_id = ? ORDER BY group_order",
    )
    .bind(group_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|error| Box::new(internal_error("listing placement group members", error)))
}

/// Rebuilds one group's full response, members included.
///
/// Goes back through `load_placements` rather than assembling the group from
/// what the handler already had in hand: the response a mutation returns and
/// the response a subsequent `GET` returns are then the same code, so they
/// cannot disagree about ordering or about which placements are members.
async fn group_response(
    state: &AppState,
    dashboard_id: &str,
    group_id: &str,
) -> RouteResult<Option<PlacementGroupResponse>> {
    let (_, groups) = load_placements(state, dashboard_id).await?;
    Ok(groups.into_iter().find(|group| group.id == group_id))
}

fn no_such_group(group_id: &str) -> Response {
    ErrorBody::message(
        StatusCode::NOT_FOUND,
        format!("no such dashboard placement group: {group_id}"),
    )
}

async fn dashboard_detail(state: &AppState, id: &str, role: DashboardRole) -> Response {
    let row = sqlx::query_as::<_, DashboardRow>(
        "SELECT d.id, d.name, d.owner_user_id, u.username AS owner_username, d.created_at, \
                d.hidden \
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

    let (placements, placement_groups) = match load_placements(state, id).await {
        Ok(split) => split,
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
        hidden: row.hidden,
        placements,
        placement_groups,
    })
    .into_response()
}

/// Loads one dashboard's tiles, already separated into standalone placements
/// and groups.
///
/// The split happens here rather than in the client because the server is the
/// only party that knows it: a flat array plus a `groupId` on each entry would
/// make every client re-derive the same partition, and get the member ordering
/// wrong in a different way each time.
async fn load_placements(
    state: &AppState,
    dashboard_id: &str,
) -> RouteResult<(Vec<PlacementResponse>, Vec<PlacementGroupResponse>)> {
    // Standalone placements keep the ordering they have always had.
    // `group_order` is the tiebreak for members and is NULL here, so one query
    // serves both: the ordering clause is simply inert for the standalone half.
    let rows = sqlx::query_as::<_, PlacementRow>(
        "SELECT id, connector_instance_id, target_id, position_x, position_y, width, height, \
                widget_bindings, placement_action, label, icon, created_at, group_id \
         FROM dashboard_placements WHERE dashboard_id = ? \
         ORDER BY group_order, position_y, position_x, created_at",
    )
    .bind(dashboard_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|error| Box::new(internal_error("listing dashboard placements", error)))?;

    let group_rows = sqlx::query_as::<_, PlacementGroupRow>(
        "SELECT id, name, icon, position_x, position_y, width, height, created_at \
         FROM dashboard_placement_groups WHERE dashboard_id = ? \
         ORDER BY position_y, position_x, created_at",
    )
    .bind(dashboard_id)
    .fetch_all(&state.pool)
    .await
    .map_err(|error| Box::new(internal_error("listing dashboard placement groups", error)))?;

    let mut standalone = Vec::new();
    let mut members: HashMap<String, Vec<PlacementResponse>> = HashMap::new();
    for row in rows {
        let group_id = row.group_id.clone();
        let placement = placement_response(state, row).await?;
        match group_id {
            Some(group_id) => members.entry(group_id).or_default().push(placement),
            None => standalone.push(placement),
        }
    }

    let groups = group_rows
        .into_iter()
        .map(|group| PlacementGroupResponse {
            members: members.remove(&group.id).unwrap_or_default(),
            id: group.id,
            name: group.name,
            icon: group.icon,
            position_x: group.position_x,
            position_y: group.position_y,
            width: group.width,
            height: group.height,
            created_at: group.created_at,
        })
        .collect();

    Ok((standalone, groups))
}

async fn placement_response(state: &AppState, row: PlacementRow) -> RouteResult<PlacementResponse> {
    let bindings = serde_json::from_str(&row.widget_bindings)
        .map_err(|error| Box::new(internal_error("reading stored widget bindings", error)))?;
    // Absent for a static tile, which never had one. A placement that *does*
    // name a connector and cannot resolve it is still the referential-integrity
    // failure it always was, and still says so.
    let connector = match &row.connector_instance_id {
        Some(connector_id) => Some(
            connectors::instance_summary(state, connector_id)
                .await?
                .ok_or_else(|| {
                    Box::new(ErrorBody::message(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "a dashboard placement references a missing connector",
                    ))
                })?,
        ),
        None => None,
    };
    let placement_action = decode_placement_action(row.placement_action.as_deref())?;

    Ok(PlacementResponse {
        id: row.id,
        connector,
        target_id: row.target_id,
        position_x: row.position_x,
        position_y: row.position_y,
        width: row.width,
        height: row.height,
        widget_bindings: bindings,
        placement_action,
        label: row.label,
        icon: row.icon,
        created_at: row.created_at,
        group_id: row.group_id,
    })
}

/// A user-supplied display string, or `None` when it was blank.
///
/// Whitespace is not a label. Storing `"   "` would give a tile an invisible
/// name the client would render as an empty box, which is indistinguishable on
/// screen from a bug.
fn trimmed_or_none(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}

/// Reads one placement, scoped to the dashboard it must belong to.
///
/// Shared by update, delete-adjacent lookups and the click endpoint so
/// "placement `x` of dashboard `y`" is resolved the same way everywhere — a
/// placement id from another dashboard is `None`, never a row.
async fn load_placement_row(
    state: &AppState,
    dashboard_id: &str,
    placement_id: &str,
) -> RouteResult<Option<PlacementRow>> {
    sqlx::query_as::<_, PlacementRow>(
        "SELECT id, connector_instance_id, target_id, position_x, position_y, width, height, \
                widget_bindings, placement_action, label, icon, created_at, group_id \
         FROM dashboard_placements WHERE id = ? AND dashboard_id = ?",
    )
    .bind(placement_id)
    .bind(dashboard_id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|error| Box::new(internal_error("loading a dashboard placement", error)))
}

/// JSON for the `placement_action` column, or `NULL` for no click behaviour.
fn encode_placement_action(action: Option<&PlacementAction>) -> RouteResult<Option<String>> {
    action
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| Box::new(internal_error("serializing a placement action", error)))
}

/// The stored `placement_action` column, back as a value.
fn decode_placement_action(stored: Option<&str>) -> RouteResult<Option<PlacementAction>> {
    stored
        .map(serde_json::from_str)
        .transpose()
        .map_err(|error| Box::new(internal_error("reading a stored placement action", error)))
}

/// Width and height a placement row would accept.
///
/// Distinct from `reject_bad_box`, which words its message for a group. Both
/// enforce the same `> 0` the table's CHECK constraints do, so a violation is a
/// 400 with an explanation rather than a 500 from the database.
fn reject_bad_placement_box(width: i64, height: i64) -> Option<Response> {
    (width < 1 || height < 1)
        .then(|| bad_request("a placement's width and height must both be at least 1"))
}

/// Checks a [`PlacementAction`] as the *editor* saving it, at save time.
///
/// This is a sanity check and nothing more. Its job is to stop a tile being
/// saved that points at a dashboard that does not exist or an action the
/// connector does not have — mistakes worth catching in the editor rather than
/// on someone else's click. It is explicitly **not** the authorization that
/// governs a click: see `click_placement`, which re-checks against whoever is
/// clicking, whenever they click.
async fn validate_placement_action(
    state: &AppState,
    caller_id: &str,
    action: &PlacementAction,
) -> RouteResult<()> {
    match action {
        PlacementAction::Navigate {
            target_dashboard_id,
        } => {
            // "No such dashboard" and "not one you can see" are answered
            // identically *here*, and only here: the id is arbitrary caller
            // input at this point, so distinguishing them would turn this
            // endpoint into a way to enumerate other people's dashboard ids.
            // That is the same reasoning `GET /dashboards/{id}` already
            // follows. At click time the id is one the caller was already
            // shown, so the two states are separated there.
            match get_dashboard_role(&state.pool, caller_id, target_dashboard_id).await {
                Ok(Some(_)) => Ok(()),
                Ok(None) => Err(Box::new(ErrorBody::message(
                    StatusCode::FORBIDDEN,
                    "the navigate target is not a dashboard you can access",
                ))),
                Err(error) => Err(Box::new(internal_error(
                    "resolving a navigate target",
                    error,
                ))),
            }
        }
        PlacementAction::ConnectorAction {
            connector_instance_id,
            target_id,
            action_id,
            ..
        } => {
            let connector = match Uuid::parse_str(connector_instance_id) {
                Ok(uuid) => state.connectors.get(&uuid).await,
                Err(_) => None,
            }
            .ok_or_else(|| {
                Box::new(bad_request(format!(
                    "the placement action's connector instance is not currently available: \
                     {connector_instance_id}"
                )))
            })?;

            // The same descriptor lookup the action endpoint itself uses, so a
            // tile can only be configured with an action that endpoint would
            // accept — including a resource-kind row or kind action, which is
            // an ordinary action advertised somewhere else.
            if connectors::resolve_action(connector.as_ref(), action_id, target_id.as_deref())
                .await
                .is_none()
            {
                return Err(Box::new(bad_request(format!(
                    "the placement action names an action this connector does not advertise: \
                     {action_id}"
                ))));
            }
            Ok(())
        }
    }
}

async fn validate_placement(
    state: &AppState,
    connector_id: &str,
    target_id: Option<&str>,
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

    if let Some(target_id) = target_id {
        if !connector.supports_sub_targets() {
            return Err(Box::new(bad_request(
                "this connector instance does not support targetId",
            )));
        }
        let targets = connector.list_sub_targets().await.map_err(|error| {
            Box::new(bad_request(format!(
                "could not validate targetId against the live connector: {error}"
            )))
        })?;
        if !targets.iter().any(|target| target.id == target_id) {
            return Err(Box::new(bad_request(format!(
                "no such connector sub-target: {target_id}"
            ))));
        }
    }

    let bindings = requested_bindings
        .or(existing_bindings)
        .unwrap_or_else(|| connector.default_layout_for(target_id).bindings);
    // Each binding kind resolves against its own namespace: a display binding
    // names a data point, an action binding names an action, and a
    // resource-kind binding names a browsable kind. They are all strings and
    // they are not interchangeable, so checking one list for all three would
    // either reject valid bindings or wave through bindings that can never
    // render.
    let data_point_ids: HashSet<(String, Option<String>)> = connector
        .data_points()
        .into_iter()
        .map(|point| (point.id, point.target_id))
        .collect();
    let action_ids: HashSet<(String, Option<String>)> = connector
        .actions()
        .await
        .into_iter()
        .map(|action| (action.id, action.target_id))
        .collect();
    // The third namespace. Read for this placement's target specifically,
    // because `resource_kinds` takes one exactly so a kind can be absent at one
    // scope and present at another.
    let resource_kinds: HashSet<String> = connector
        .resource_kinds(target_id)
        .into_iter()
        .map(|kind| kind.kind)
        .collect();
    let selected_target = target_id.map(str::to_owned);

    let mut unknown_data_points: Vec<&str> = Vec::new();
    let mut unknown_actions: Vec<&str> = Vec::new();
    let mut unknown_resource_kinds: Vec<&str> = Vec::new();
    for binding in &bindings {
        match binding {
            WidgetBinding::Display { data_point_id, .. }
                if !data_point_ids.contains(&(data_point_id.clone(), selected_target.clone())) =>
            {
                unknown_data_points.push(data_point_id);
            }
            WidgetBinding::Action { action_id, .. }
                if !action_ids.contains(&(action_id.clone(), selected_target.clone())) =>
            {
                unknown_actions.push(action_id);
            }
            WidgetBinding::ResourceKindDisplay { resource_kind }
                if !resource_kinds.contains(resource_kind) =>
            {
                unknown_resource_kinds.push(resource_kind);
            }
            _ => {}
        }
    }
    for list in [
        &mut unknown_data_points,
        &mut unknown_actions,
        &mut unknown_resource_kinds,
    ] {
        list.sort_unstable();
        list.dedup();
    }

    if !unknown_data_points.is_empty()
        || !unknown_actions.is_empty()
        || !unknown_resource_kinds.is_empty()
    {
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
        if !unknown_resource_kinds.is_empty() {
            problems.push(format!(
                "unknown resource kinds: {}",
                unknown_resource_kinds.join(", ")
            ));
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
