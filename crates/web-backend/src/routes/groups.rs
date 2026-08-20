//! Group and permission-grant administration. Every route requires a global
//! `groups.manage` grant.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, Transaction};
use uuid::Uuid;

use crate::auth::extract::{GroupsManage, RequirePermission};
use crate::auth::permissions::PermissionGrant;
use crate::error::{internal_error, ErrorBody};
use crate::state::AppState;

/// A group with everything needed to render and edit it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GroupResponse {
    id: String,
    name: String,
    description: Option<String>,
    created_at: String,
    /// True for groups that must not be deleted — today only Administrators.
    /// Clients should hide or disable the delete control rather than let the
    /// user discover the rule through a 409.
    is_protected: bool,
    /// How many users belong to this group.
    member_count: i64,
    /// Every grant held by this group.
    permissions: Vec<PermissionGrant>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateGroupRequest {
    name: String,
    description: Option<String>,
    /// Grants to create with the group. Absent means none.
    #[serde(default)]
    permissions: Vec<PermissionGrant>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateGroupRequest {
    /// Absent leaves it alone.
    name: Option<String>,
    /// Absent leaves it alone. Present-and-null clears it.
    ///
    /// The custom deserializer is load-bearing, not decoration: without it an
    /// explicit `null` arrives as the outer `None` and reads as "absent", so
    /// clearing a description would silently do nothing. See
    /// [`crate::routes::present_option`].
    #[serde(default, deserialize_with = "crate::routes::present_option")]
    description: Option<Option<String>>,
    /// Absent leaves grants alone; present **replaces** them wholesale.
    permissions: Option<Vec<PermissionGrant>>,
}

#[derive(sqlx::FromRow)]
struct GroupRow {
    id: String,
    name: String,
    description: Option<String>,
    created_at: String,
    is_protected: bool,
}

/// `GET /groups`
pub async fn list_groups(
    _caller: RequirePermission<GroupsManage>,
    State(state): State<AppState>,
) -> Response {
    let groups = sqlx::query_as::<_, GroupRow>(
        "SELECT id, name, description, created_at, is_protected FROM groups ORDER BY name",
    )
    .fetch_all(&state.pool)
    .await;

    let groups = match groups {
        Ok(groups) => groups,
        Err(error) => return internal_error("listing groups", error),
    };

    // Grants and member counts in one query each rather than per group: this is
    // the screen an administrator opens to see everything at once.
    let grants = sqlx::query_as::<_, (String, String, Option<String>, Option<String>)>(
        "SELECT group_id, permission_key, resource_type, resource_id FROM group_permissions",
    )
    .fetch_all(&state.pool)
    .await;

    let grants = match grants {
        Ok(grants) => grants,
        Err(error) => return internal_error("listing group grants", error),
    };

    let counts = sqlx::query_as::<_, (String, i64)>(
        "SELECT group_id, COUNT(*) FROM user_groups GROUP BY group_id",
    )
    .fetch_all(&state.pool)
    .await;

    let counts = match counts {
        Ok(counts) => counts,
        Err(error) => return internal_error("counting group members", error),
    };

    let responses: Vec<GroupResponse> = groups
        .into_iter()
        .map(|group| GroupResponse {
            member_count: counts
                .iter()
                .find(|(group_id, _)| group_id == &group.id)
                .map_or(0, |(_, count)| *count),
            permissions: grants
                .iter()
                .filter(|(group_id, ..)| group_id == &group.id)
                .map(|(_, key, resource_type, resource_id)| PermissionGrant {
                    key: key.clone(),
                    resource_type: resource_type.clone(),
                    resource_id: resource_id.clone(),
                })
                .collect(),
            id: group.id,
            name: group.name,
            description: group.description,
            created_at: group.created_at,
            is_protected: group.is_protected,
        })
        .collect();

    Json(responses).into_response()
}

/// `POST /groups`
pub async fn create_group(
    _caller: RequirePermission<GroupsManage>,
    State(state): State<AppState>,
    Json(request): Json<CreateGroupRequest>,
) -> Response {
    let name = request.name.trim();
    if name.is_empty() {
        return ErrorBody::message(StatusCode::BAD_REQUEST, "name must not be empty".to_owned());
    }

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => return internal_error("beginning the create-group transaction", error),
    };

    match sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM groups WHERE name = ?")
        .bind(name)
        .fetch_one(&mut *tx)
        .await
    {
        Ok((0,)) => {}
        Ok(_) => {
            return ErrorBody::message(
                StatusCode::CONFLICT,
                format!("a group named {name} already exists"),
            );
        }
        Err(error) => return internal_error("checking group name uniqueness", error),
    }

    let group_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    if let Err(error) = sqlx::query(
        "INSERT INTO groups (id, name, description, created_at, is_protected) \
         VALUES (?, ?, ?, ?, FALSE)",
    )
    .bind(&group_id)
    .bind(name)
    .bind(&request.description)
    .bind(&now)
    .execute(&mut *tx)
    .await
    {
        return internal_error("creating the group", error);
    }

    if let Err(response) = set_grants(&mut tx, &group_id, &request.permissions).await {
        return response;
    }

    if let Err(error) = tx.commit().await {
        return internal_error("committing the new group", error);
    }

    (
        StatusCode::CREATED,
        Json(GroupResponse {
            id: group_id,
            name: name.to_owned(),
            description: request.description,
            created_at: now,
            is_protected: false,
            member_count: 0,
            permissions: request.permissions,
        }),
    )
        .into_response()
}

/// `PATCH /groups/{id}`
///
/// A protected group may be renamed and re-granted; only deletion is refused.
/// Editing it is a legitimate administrative act, and blocking that would force
/// operators to work around the safeguard rather than with it.
pub async fn update_group(
    _caller: RequirePermission<GroupsManage>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateGroupRequest>,
) -> Response {
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => return internal_error("beginning the update-group transaction", error),
    };

    let existing = sqlx::query_as::<_, GroupRow>(
        "SELECT id, name, description, created_at, is_protected FROM groups WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await;

    let existing = match existing {
        Ok(Some(group)) => group,
        Ok(None) => {
            return ErrorBody::message(StatusCode::NOT_FOUND, format!("no such group: {id}"));
        }
        Err(error) => return internal_error("loading the group", error),
    };

    let name = match &request.name {
        Some(name) if name.trim().is_empty() => {
            return ErrorBody::message(
                StatusCode::BAD_REQUEST,
                "name must not be empty".to_owned(),
            );
        }
        Some(name) => name.trim().to_owned(),
        None => existing.name.clone(),
    };

    let description = match &request.description {
        Some(description) => description.clone(),
        None => existing.description.clone(),
    };

    if let Err(error) = sqlx::query("UPDATE groups SET name = ?, description = ? WHERE id = ?")
        .bind(&name)
        .bind(&description)
        .bind(&id)
        .execute(&mut *tx)
        .await
    {
        return internal_error("updating the group", error);
    }

    if let Some(permissions) = &request.permissions {
        if let Err(response) = set_grants(&mut tx, &id, permissions).await {
            return response;
        }
    }

    let permissions = match &request.permissions {
        Some(permissions) => permissions.clone(),
        None => match current_grants(&mut tx, &id).await {
            Ok(grants) => grants,
            Err(error) => return internal_error("reading group grants", error),
        },
    };

    let member_count =
        match sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM user_groups WHERE group_id = ?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
        {
            Ok((count,)) => count,
            Err(error) => return internal_error("counting group members", error),
        };

    if let Err(error) = tx.commit().await {
        return internal_error("committing the group update", error);
    }

    Json(GroupResponse {
        id,
        name,
        description,
        created_at: existing.created_at,
        is_protected: existing.is_protected,
        member_count,
        permissions,
    })
    .into_response()
}

/// `DELETE /groups/{id}`
///
/// Refuses to delete a protected group. Checked on the `is_protected` column
/// rather than the name, so renaming Administrators does not quietly remove the
/// protection — a name is a label users change, not an identity to make
/// security decisions on.
pub async fn delete_group(
    _caller: RequirePermission<GroupsManage>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let protected = sqlx::query_as::<_, (bool,)>("SELECT is_protected FROM groups WHERE id = ?")
        .bind(&id)
        .fetch_optional(&state.pool)
        .await;

    match protected {
        Ok(None) => {
            return ErrorBody::message(StatusCode::NOT_FOUND, format!("no such group: {id}"));
        }
        Ok(Some((true,))) => {
            return ErrorBody::message(
                StatusCode::CONFLICT,
                "this group is protected and cannot be deleted; it is how the \
                 instance grants administrative access"
                    .to_owned(),
            );
        }
        Ok(Some((false,))) => {}
        Err(error) => return internal_error("checking group protection", error),
    }

    // Memberships and grants go with it via ON DELETE CASCADE. Users are not
    // touched — losing a group removes what it granted, not the accounts.
    if let Err(error) = sqlx::query("DELETE FROM groups WHERE id = ?")
        .bind(&id)
        .execute(&state.pool)
        .await
    {
        return internal_error("deleting the group", error);
    }

    tracing::info!(group_id = %id, "group deleted");

    StatusCode::NO_CONTENT.into_response()
}

/// One row of the permission catalog.
#[derive(Debug, Serialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PermissionCatalogEntry {
    key: String,
    description: String,
}

/// `GET /permissions`
///
/// The registered permission set, so a client can build a grant-assignment form
/// without hardcoding a list that would silently fall out of date the next time
/// a migration adds a key.
///
/// Requires `groups.manage`, since assigning grants is the only thing this is
/// for.
pub async fn list_permissions(
    _caller: RequirePermission<GroupsManage>,
    State(state): State<AppState>,
) -> Response {
    match sqlx::query_as::<_, PermissionCatalogEntry>(
        "SELECT key, description FROM permissions ORDER BY key",
    )
    .fetch_all(&state.pool)
    .await
    {
        Ok(entries) => Json(entries).into_response(),
        Err(error) => internal_error("listing permissions", error),
    }
}

/// Replaces a group's grants with exactly `grants`.
async fn set_grants(
    tx: &mut Transaction<'_, Sqlite>,
    group_id: &str,
    grants: &[PermissionGrant],
) -> Result<(), Response> {
    if let Err(error) = sqlx::query("DELETE FROM group_permissions WHERE group_id = ?")
        .bind(group_id)
        .execute(&mut **tx)
        .await
    {
        return Err(internal_error("clearing group grants", error));
    }

    for grant in grants {
        // The foreign key onto `permissions` rejects an unregistered key, which
        // is what stops a typo becoming a grant that authorizes nothing and
        // looks fine in the UI. Reported as a 400: the input is wrong.
        if let Err(error) = sqlx::query(
            "INSERT INTO group_permissions (id, group_id, permission_key, resource_type, resource_id) \
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(group_id)
        .bind(&grant.key)
        .bind(&grant.resource_type)
        .bind(&grant.resource_id)
        .execute(&mut **tx)
        .await
        {
            return Err(ErrorBody::message(
                StatusCode::BAD_REQUEST,
                format!("invalid permission grant {}: {error}", grant.key),
            ));
        }
    }

    Ok(())
}

/// The grants a group currently holds.
async fn current_grants(
    tx: &mut Transaction<'_, Sqlite>,
    group_id: &str,
) -> Result<Vec<PermissionGrant>, sqlx::Error> {
    sqlx::query_as::<_, PermissionGrant>(
        "SELECT permission_key AS key, resource_type, resource_id \
         FROM group_permissions WHERE group_id = ? ORDER BY permission_key",
    )
    .bind(group_id)
    .fetch_all(&mut **tx)
    .await
}
