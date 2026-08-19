//! First-run setup: naming the instance and creating its first administrator.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::password::{hash_password, MIN_PASSWORD_LENGTH};
use crate::error::{internal_error, ErrorBody};
use crate::state::AppState;

/// Id of the group seeded by `20260819000002_seed_administrators_group.sql`.
///
/// Fixed rather than looked up by name, so renaming the group cannot break
/// setup.
const ADMINISTRATORS_GROUP_ID: &str = "00000000-0000-4000-8000-000000000001";

/// `GET /setup/status` response.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatusResponse {
    setup_complete: bool,
}

/// `POST /setup` request.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupRequest {
    instance_name: String,
    admin_username: String,
    admin_password: String,
}

/// Whether the instance has an administrator yet.
///
/// Derived from `SELECT COUNT(*) FROM users` rather than from a flag in
/// `server_config`, and the choice matters. A flag is a second source of truth
/// that can disagree with reality: if setup is interrupted after the flag is
/// written but before the user row commits, a flag-based check reports a
/// configured instance that nobody can log into, and there is no way back
/// without editing the database by hand. Counting users cannot drift, because
/// the thing it counts *is* the thing that makes setup meaningful.
///
/// The user insert and the group assignment run in one transaction, so a
/// half-finished setup leaves no user at all and the wizard simply runs again.
async fn is_setup_complete(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let (count,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(pool)
        .await?;
    Ok(count > 0)
}

/// `GET /setup/status`
///
/// Unauthenticated by necessity, not by oversight: a client must be able to ask
/// this before anyone can hold a credential. It therefore reveals exactly one
/// bit and must never grow a field describing a configured instance.
pub async fn setup_status(State(state): State<AppState>) -> Response {
    match is_setup_complete(&state.pool).await {
        Ok(setup_complete) => Json(SetupStatusResponse { setup_complete }).into_response(),
        Err(error) => internal_error("setup status", error),
    }
}

/// `POST /setup`
///
/// Creates the first administrator and puts them in the seeded Administrators
/// group. Unauthenticated for the same reason as the status route — there is
/// nobody to authenticate as until this succeeds — which is exactly why it must
/// be impossible to run twice.
pub async fn complete_setup(
    State(state): State<AppState>,
    Json(request): Json<SetupRequest>,
) -> Response {
    let username = request.admin_username.trim();

    if request.instance_name.trim().is_empty() {
        return ErrorBody::message(
            StatusCode::BAD_REQUEST,
            "instanceName must not be empty".to_owned(),
        );
    }

    if username.is_empty() {
        return ErrorBody::message(
            StatusCode::BAD_REQUEST,
            "adminUsername must not be empty".to_owned(),
        );
    }

    if request.admin_password.len() < MIN_PASSWORD_LENGTH {
        return ErrorBody::message(
            StatusCode::BAD_REQUEST,
            format!("adminPassword must be at least {MIN_PASSWORD_LENGTH} characters"),
        );
    }

    let password_hash = match hash_password(&request.admin_password) {
        Ok(hash) => hash,
        Err(error) => return internal_error("hashing the setup password", error),
    };

    // One transaction for the whole thing. The check and the writes have to be
    // atomic or two concurrent setup requests could both pass the check and
    // both create an administrator — the instance-seizure case this endpoint
    // exists to prevent.
    let mut tx = match state.pool.begin().await {
        Ok(tx) => tx,
        Err(error) => return internal_error("beginning the setup transaction", error),
    };

    let existing: Result<(i64,), _> = sqlx::query_as("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *tx)
        .await;

    match existing {
        Ok((count,)) if count > 0 => {
            return ErrorBody::message(
                StatusCode::CONFLICT,
                "setup has already been completed for this instance".to_owned(),
            );
        }
        Ok(_) => {}
        Err(error) => return internal_error("checking for existing users", error),
    }

    let user_id = Uuid::new_v4().to_string();
    let now = Utc::now().to_rfc3339();

    if let Err(error) = sqlx::query(
        r#"
        INSERT INTO users (id, username, password_hash, is_active, created_at)
        VALUES (?, ?, ?, TRUE, ?)
        "#,
    )
    .bind(&user_id)
    .bind(username)
    .bind(&password_hash)
    .bind(&now)
    .execute(&mut *tx)
    .await
    {
        return internal_error("creating the administrator", error);
    }

    if let Err(error) = sqlx::query("INSERT INTO user_groups (user_id, group_id) VALUES (?, ?)")
        .bind(&user_id)
        .bind(ADMINISTRATORS_GROUP_ID)
        .execute(&mut *tx)
        .await
    {
        return internal_error("assigning the administrators group", error);
    }

    // The instance name is stored as instance state rather than discarded; it
    // is the one value from this form the server keeps.
    if let Err(error) = sqlx::query(
        r#"
        INSERT INTO server_config (key, value, updated_at)
        VALUES ('instance_name', ?, ?)
        ON CONFLICT (key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at
        "#,
    )
    .bind(request.instance_name.trim())
    .bind(&now)
    .execute(&mut *tx)
    .await
    {
        return internal_error("recording the instance name", error);
    }

    if let Err(error) = tx.commit().await {
        return internal_error("committing setup", error);
    }

    tracing::info!(username, "first-run setup completed");

    Json(SetupStatusResponse {
        setup_complete: true,
    })
    .into_response()
}
