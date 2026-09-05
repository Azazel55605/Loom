//! Instance-wide dashboard administration.
//!
//! Ordinary `/dashboards` routes intentionally use each dashboard's local ACL.
//! These routes are separate because `dashboards.manage` is an administrative
//! override, not another local dashboard role.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::auth::extract::{DashboardsManage, RequirePermission};
use crate::error::{internal_error, ErrorBody};
use crate::state::AppState;

#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub(super) struct AdminDashboardResponse {
    id: String,
    name: String,
    owner_user_id: String,
    owner_username: String,
    hidden: bool,
    share_count: i64,
    placement_count: i64,
    created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct UpdateAdminDashboardRequest {
    name: Option<String>,
    hidden: Option<bool>,
    owner_user_id: Option<String>,
}

const SUMMARY_QUERY: &str =
    "SELECT d.id, d.name, d.owner_user_id, u.username AS owner_username, d.hidden, \
     (SELECT COUNT(*) FROM dashboard_shares ds WHERE ds.dashboard_id = d.id) AS share_count, \
     (SELECT COUNT(*) FROM dashboard_placements dp WHERE dp.dashboard_id = d.id) AS placement_count, \
     d.created_at \
     FROM dashboards d JOIN users u ON u.id = d.owner_user_id";

/// `GET /admin/dashboards` — every dashboard, independent of local ACLs.
pub(super) async fn list_dashboards(
    _: RequirePermission<DashboardsManage>,
    State(state): State<AppState>,
) -> Response {
    let query = format!("{SUMMARY_QUERY} ORDER BY lower(d.name), d.id");
    match sqlx::query_as::<_, AdminDashboardResponse>(&query)
        .fetch_all(&state.pool)
        .await
    {
        Ok(rows) => Json(rows).into_response(),
        Err(error) => internal_error("listing dashboards for administration", error),
    }
}

/// `PATCH /admin/dashboards/{id}` — administrative metadata/owner update.
pub(super) async fn update_dashboard(
    _: RequirePermission<DashboardsManage>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateAdminDashboardRequest>,
) -> Response {
    let name = match request.name.as_deref().map(str::trim) {
        Some("") => return bad_request("name must not be empty"),
        other => other,
    };

    if let Some(owner_user_id) = request.owner_user_id.as_deref() {
        let exists =
            sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users WHERE id = ?)")
                .bind(owner_user_id)
                .fetch_one(&state.pool)
                .await;
        match exists {
            Ok(true) => {}
            Ok(false) => return bad_request("ownerUserId must identify an existing user"),
            Err(error) => return internal_error("validating a dashboard owner", error),
        }
    }

    let exists =
        sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM dashboards WHERE id = ?)")
            .bind(&id)
            .fetch_one(&state.pool)
            .await;
    match exists {
        Ok(true) => {}
        Ok(false) => return not_found(),
        Err(error) => return internal_error("finding a dashboard to administer", error),
    }

    if let Some(name) = name {
        if let Err(error) = sqlx::query("UPDATE dashboards SET name = ? WHERE id = ?")
            .bind(name)
            .bind(&id)
            .execute(&state.pool)
            .await
        {
            return internal_error("renaming an administered dashboard", error);
        }
    }
    if let Some(hidden) = request.hidden {
        if let Err(error) = sqlx::query("UPDATE dashboards SET hidden = ? WHERE id = ?")
            .bind(hidden)
            .bind(&id)
            .execute(&state.pool)
            .await
        {
            return internal_error("changing an administered dashboard's hidden flag", error);
        }
    }
    if let Some(owner_user_id) = request.owner_user_id {
        if let Err(error) = sqlx::query("UPDATE dashboards SET owner_user_id = ? WHERE id = ?")
            .bind(owner_user_id)
            .bind(&id)
            .execute(&state.pool)
            .await
        {
            return internal_error("reassigning an administered dashboard", error);
        }
    }

    let query = format!("{SUMMARY_QUERY} WHERE d.id = ?");
    match sqlx::query_as::<_, AdminDashboardResponse>(&query)
        .bind(&id)
        .fetch_optional(&state.pool)
        .await
    {
        Ok(Some(row)) => Json(row).into_response(),
        Ok(None) => not_found(),
        Err(error) => internal_error("loading an administered dashboard", error),
    }
}

/// `DELETE /admin/dashboards/{id}` — cascades through existing foreign keys.
pub(super) async fn delete_dashboard(
    _: RequirePermission<DashboardsManage>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    match sqlx::query("DELETE FROM dashboards WHERE id = ?")
        .bind(id)
        .execute(&state.pool)
        .await
    {
        Ok(result) if result.rows_affected() == 1 => StatusCode::NO_CONTENT.into_response(),
        Ok(_) => not_found(),
        Err(error) => internal_error("deleting an administered dashboard", error),
    }
}

fn bad_request(message: &str) -> Response {
    ErrorBody::message(StatusCode::BAD_REQUEST, message.to_owned())
}

fn not_found() -> Response {
    ErrorBody::message(StatusCode::NOT_FOUND, "dashboard not found".to_owned())
}
