//! User administration. Every route requires a global `users.manage` grant.
//!
//! ## The safeguards
//!
//! Two rules here exist to stop an instance being administered into a state
//! nobody can administer it out of. Both are checked inside the same
//! transaction as the write they guard, because a check made outside one is
//! only a guess about what will be true when the write lands.
//!
//! 1. **The last active administrator cannot be removed.** Deactivating,
//!    deleting, or removing from the Administrators group the only remaining
//!    active member of that group is refused with 409. Losing it means losing
//!    `users.manage` and `groups.manage` for the whole instance, with no route
//!    back short of editing the database by hand.
//!
//! 2. **Nobody may remove themselves.** Independent of the first rule, and
//!    refused even when other administrators exist. An accidental self-deletion
//!    is unrecoverable by the person best placed to notice it, and "delete
//!    someone else" is always available for a genuine departure.
//!
//! Neither is a security control — an administrator can still hand the instance
//! to someone else and have them do it. They guard against mistakes, which is
//! the failure mode that actually occurs.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::{Sqlite, SqliteConnection, Transaction};
use uuid::Uuid;

use crate::auth::extract::{RequirePermission, UsersManage};
use crate::auth::password::{hash_password, MIN_PASSWORD_LENGTH};
use crate::error::{internal_error, ErrorBody};
use crate::state::AppState;

/// A user as the API reports them.
///
/// There is no field for the password hash and there must never be one. It is
/// not secret in the sense a password is, but publishing it hands an attacker
/// an offline target they can attack at their own pace.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserResponse {
    id: String,
    username: String,
    is_active: bool,
    created_at: String,
    /// Ids of every group the user belongs to.
    group_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateUserRequest {
    username: String,
    password: String,
    /// Groups to place the user in. Absent or empty means no groups, which is
    /// a valid account that can sign in and do nothing.
    #[serde(default)]
    group_ids: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateUserRequest {
    /// Absent leaves the flag alone; present sets it.
    is_active: Option<bool>,
    /// Absent leaves membership alone; present **replaces** it wholesale.
    ///
    /// Replace rather than add/remove deltas: a caller sends the membership it
    /// wants and gets exactly that, with no dependence on what it believed the
    /// previous state to be.
    group_ids: Option<Vec<String>>,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    username: String,
    is_active: bool,
    created_at: String,
}

/// `GET /users`
pub async fn list_users(
    _caller: RequirePermission<UsersManage>,
    State(state): State<AppState>,
) -> Response {
    let users = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, is_active, created_at FROM users ORDER BY username",
    )
    .fetch_all(&state.pool)
    .await;

    let users = match users {
        Ok(users) => users,
        Err(error) => return internal_error("listing users", error),
    };

    // Memberships in one query rather than one per user: the N+1 is avoidable
    // and this list is rendered on every visit to the admin screen.
    let memberships =
        sqlx::query_as::<_, (String, String)>("SELECT user_id, group_id FROM user_groups")
            .fetch_all(&state.pool)
            .await;

    let memberships = match memberships {
        Ok(memberships) => memberships,
        Err(error) => return internal_error("listing group memberships", error),
    };

    let responses: Vec<UserResponse> = users
        .into_iter()
        .map(|user| UserResponse {
            group_ids: memberships
                .iter()
                .filter(|(user_id, _)| user_id == &user.id)
                .map(|(_, group_id)| group_id.clone())
                .collect(),
            id: user.id,
            username: user.username,
            is_active: user.is_active,
            created_at: user.created_at,
        })
        .collect();

    Json(responses).into_response()
}

/// `POST /users`
pub async fn create_user(
    _caller: RequirePermission<UsersManage>,
    State(state): State<AppState>,
    Json(request): Json<CreateUserRequest>,
) -> Response {
    let username = request.username.trim();

    if username.is_empty() {
        return ErrorBody::message(
            StatusCode::BAD_REQUEST,
            "username must not be empty".to_owned(),
        );
    }

    // The same floor setup applies, from the same constant, so the rule cannot
    // drift between the two ways an account is created.
    if request.password.len() < MIN_PASSWORD_LENGTH {
        return ErrorBody::message(
            StatusCode::BAD_REQUEST,
            format!("password must be at least {MIN_PASSWORD_LENGTH} characters"),
        );
    }

    let password_hash = match hash_password(&request.password) {
        Ok(hash) => hash,
        Err(error) => return internal_error("hashing a new user's password", error),
    };

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => return internal_error("beginning the create-user transaction", error),
    };

    // Checked explicitly rather than relying on the UNIQUE constraint, so the
    // caller gets a 409 naming the problem instead of a 500 from a violated
    // index. The constraint still backstops a race.
    match sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM users WHERE username = ?")
        .bind(username)
        .fetch_one(&mut *tx)
        .await
    {
        Ok((0,)) => {}
        Ok(_) => {
            return ErrorBody::message(
                StatusCode::CONFLICT,
                format!("a user named {username} already exists"),
            );
        }
        Err(error) => return internal_error("checking username uniqueness", error),
    }

    let user_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    if let Err(error) = sqlx::query(
        "INSERT INTO users (id, username, password_hash, is_active, created_at) \
         VALUES (?, ?, ?, TRUE, ?)",
    )
    .bind(&user_id)
    .bind(username)
    .bind(&password_hash)
    .bind(&now)
    .execute(&mut *tx)
    .await
    {
        return internal_error("creating the user", error);
    }

    if let Err(response) = set_group_memberships(&mut tx, &user_id, &request.group_ids).await {
        return *response;
    }

    if let Err(error) = tx.commit().await {
        return internal_error("committing the new user", error);
    }

    tracing::info!(username, "user created");

    (
        StatusCode::CREATED,
        Json(UserResponse {
            id: user_id,
            username: username.to_owned(),
            is_active: true,
            created_at: now,
            group_ids: request.group_ids,
        }),
    )
        .into_response()
}

/// `PATCH /users/{id}`
pub async fn update_user(
    caller: RequirePermission<UsersManage>,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(request): Json<UpdateUserRequest>,
) -> Response {
    // Self-removal is refused before anything else, so the answer does not
    // depend on how many other administrators happen to exist.
    if id == caller.id() && request.is_active == Some(false) {
        return ErrorBody::message(
            StatusCode::CONFLICT,
            "you cannot deactivate your own account; ask another administrator".to_owned(),
        );
    }

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => return internal_error("beginning the update-user transaction", error),
    };

    let existing = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, is_active, created_at FROM users WHERE id = ?",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await;

    let existing = match existing {
        Ok(Some(user)) => user,
        Ok(None) => {
            return ErrorBody::message(StatusCode::NOT_FOUND, format!("no such user: {id}"));
        }
        Err(error) => return internal_error("loading the user", error),
    };

    if let Some(is_active) = request.is_active {
        if let Err(error) = sqlx::query("UPDATE users SET is_active = ? WHERE id = ?")
            .bind(is_active)
            .bind(&id)
            .execute(&mut *tx)
            .await
        {
            return internal_error("updating the user", error);
        }
    }

    if let Some(group_ids) = &request.group_ids {
        if let Err(response) = set_group_memberships(&mut tx, &id, group_ids).await {
            return *response;
        }
    }

    // Checked *after* applying the change and before committing, so it asks
    // about the state the commit would actually produce rather than predicting
    // it. Any path that empties the Administrators group — deactivation,
    // membership replacement, or both at once — is caught by this one check.
    match administrators_remain(&mut tx).await {
        Ok(true) => {}
        Ok(false) => return last_administrator_conflict(),
        Err(error) => return internal_error("checking remaining administrators", error),
    }

    let group_ids = match request.group_ids {
        Some(group_ids) => group_ids,
        None => match current_group_ids(&mut tx, &id).await {
            Ok(ids) => ids,
            Err(error) => return internal_error("reading group memberships", error),
        },
    };

    if let Err(error) = tx.commit().await {
        return internal_error("committing the user update", error);
    }

    Json(UserResponse {
        id,
        username: existing.username,
        is_active: request.is_active.unwrap_or(existing.is_active),
        created_at: existing.created_at,
        group_ids,
    })
    .into_response()
}

/// `DELETE /users/{id}`
///
/// A hard delete when the account owns no dashboards: memberships and refresh
/// tokens cascade, ending its sessions. Dashboard ownership is deliberately a
/// restricting foreign key instead. Silently deleting user-authored dashboards
/// as a side effect of account administration would be data loss, so an owner
/// must delete their dashboards first; deactivation remains available when the
/// content must be retained.
pub async fn delete_user(
    caller: RequirePermission<UsersManage>,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    if id == caller.id() {
        return ErrorBody::message(
            StatusCode::CONFLICT,
            "you cannot delete your own account; ask another administrator".to_owned(),
        );
    }

    let owns_dashboards = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM dashboards WHERE owner_user_id = ?)",
    )
    .bind(&id)
    .fetch_one(&state.pool)
    .await;
    match owns_dashboards {
        Ok(true) => {
            return ErrorBody::message(
                StatusCode::CONFLICT,
                "this user owns dashboards; delete those dashboards before deleting the account",
            )
        }
        Ok(false) => {}
        Err(error) => return internal_error("checking dashboard ownership", error),
    }

    // The action log's foreign key has no `ON DELETE` action on purpose: an
    // audit trail whose attribution a later account deletion can erase is not
    // an audit trail. Without this check the database would refuse the delete
    // anyway, as a 500 that explains nothing; checked here, it is the same
    // refusal with a reason, in the same shape as the dashboards one above.
    let has_action_history = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM connector_action_log WHERE invoked_by_user_id = ?)",
    )
    .bind(&id)
    .fetch_one(&state.pool)
    .await;
    match has_action_history {
        Ok(true) => {
            return ErrorBody::message(
                StatusCode::CONFLICT,
                "this user has invoked connector actions and is named in the action log;                  deactivate the account instead of deleting it",
            )
        }
        Ok(false) => {}
        Err(error) => return internal_error("checking connector action history", error),
    }

    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => return internal_error("beginning the delete-user transaction", error),
    };

    let deleted = sqlx::query("DELETE FROM users WHERE id = ?")
        .bind(&id)
        .execute(&mut *tx)
        .await;

    match deleted {
        Ok(result) if result.rows_affected() == 0 => {
            return ErrorBody::message(StatusCode::NOT_FOUND, format!("no such user: {id}"));
        }
        Ok(_) => {}
        Err(error) => return internal_error("deleting the user", error),
    }

    match administrators_remain(&mut tx).await {
        Ok(true) => {}
        // The transaction is dropped without committing, so the delete never
        // happened.
        Ok(false) => return last_administrator_conflict(),
        Err(error) => return internal_error("checking remaining administrators", error),
    }

    if let Err(error) = tx.commit().await {
        return internal_error("committing the user deletion", error);
    }

    tracing::info!(user_id = %id, "user deleted");

    StatusCode::NO_CONTENT.into_response()
}

/// Replaces a user's group memberships with exactly `group_ids`.
///
/// Delete-then-insert rather than a diff: the caller stated the membership it
/// wants, and computing a minimal delta would add a way to be subtly wrong for
/// no gain at this size.
async fn set_group_memberships(
    tx: &mut Transaction<'_, Sqlite>,
    user_id: &str,
    group_ids: &[String],
) -> Result<(), Box<Response>> {
    if let Err(error) = sqlx::query("DELETE FROM user_groups WHERE user_id = ?")
        .bind(user_id)
        .execute(&mut **tx)
        .await
    {
        return Err(Box::new(internal_error(
            "clearing group memberships",
            error,
        )));
    }

    for group_id in group_ids {
        // The foreign key rejects an unknown group id. Reported as a 400
        // because it is the caller's input that is wrong, not the server.
        if let Err(error) = sqlx::query("INSERT INTO user_groups (user_id, group_id) VALUES (?, ?)")
            .bind(user_id)
            .bind(group_id)
            .execute(&mut **tx)
            .await
        {
            return Err(Box::new(ErrorBody::message(
                StatusCode::BAD_REQUEST,
                format!("no such group: {group_id} ({error})"),
            )));
        }
    }

    Ok(())
}

/// The group ids a user currently belongs to.
async fn current_group_ids(
    conn: &mut SqliteConnection,
    user_id: &str,
) -> Result<Vec<String>, sqlx::Error> {
    let rows = sqlx::query_as::<_, (String,)>(
        "SELECT group_id FROM user_groups WHERE user_id = ? ORDER BY group_id",
    )
    .bind(user_id)
    .fetch_all(conn)
    .await?;

    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// Whether at least one active user still belongs to a protected group.
///
/// Keyed on `is_protected` rather than the group's name or id: the flag is what
/// marks a group as load-bearing, and a rename must not quietly disable the
/// safeguard.
///
/// Run inside the caller's transaction, against the uncommitted state, so it
/// reflects what committing would actually leave behind.
async fn administrators_remain(conn: &mut SqliteConnection) -> Result<bool, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*)
        FROM user_groups ug
        JOIN users  u ON u.id = ug.user_id
        JOIN groups g ON g.id = ug.group_id
        WHERE g.is_protected = TRUE AND u.is_active = TRUE
        "#,
    )
    .fetch_one(conn)
    .await?;

    Ok(count > 0)
}

/// The 409 both safeguards return, worded so the operator knows the way out.
fn last_administrator_conflict() -> Response {
    ErrorBody::message(
        StatusCode::CONFLICT,
        "this would leave the instance with no active administrator; \
         grant another user the Administrators group first"
            .to_owned(),
    )
}
