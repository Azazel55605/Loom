//! Login, refresh, logout, and session inspection.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

use crate::auth::password::{hash_password, verify_password};
use crate::auth::permissions::{effective_permissions, PermissionGrant};
use crate::auth::tokens::{
    issue_access_token, issue_refresh_token, revoke_refresh_token, revoke_refresh_token_by_id,
    validate_refresh_token, verify_access_token,
};
use crate::error::{internal_error, ErrorBody};
use crate::state::AppState;

/// What a caller is told when credentials do not work.
///
/// One message for "no such user" and for "wrong password", deliberately.
/// Distinguishing them turns the login endpoint into a username oracle, which
/// is worth something to an attacker enumerating accounts and worth nothing to
/// a legitimate user, whose next step is the same either way.
const INVALID_CREDENTIALS: &str = "invalid credentials";

/// What a caller is told when a refresh token is not usable.
///
/// Likewise collapsed: missing, revoked, and expired are one answer, because
/// the client's response to all three is to sign in again.
const INVALID_REFRESH_TOKEN: &str = "invalid or expired refresh token";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginRequest {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshRequest {
    refresh_token: String,
}

/// Response shared by login and refresh, so a client has one code path for
/// "I now hold a session" regardless of how it got there.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TokenResponse {
    access_token: String,
    refresh_token: String,
    /// When the **access** token expires — the value a client schedules its
    /// refresh against. The refresh token's own expiry is not sent: a client
    /// cannot act on it except by discovering its refresh failed.
    expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    authenticated: bool,
    user_id: String,
    username: String,
    permissions: Vec<PermissionGrant>,
}

#[derive(sqlx::FromRow)]
struct UserRow {
    id: String,
    username: String,
    password_hash: String,
    is_active: bool,
}

/// `POST /auth/login`
pub async fn login(State(state): State<AppState>, Json(request): Json<LoginRequest>) -> Response {
    let user = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, is_active FROM users WHERE username = ?",
    )
    .bind(request.username.trim())
    .fetch_optional(&state.pool)
    .await;

    let user = match user {
        Ok(user) => user,
        Err(error) => return internal_error("looking up the user", error),
    };

    let Some(user) = user else {
        // Hash anyway before answering. Returning immediately makes an unknown
        // username measurably faster than a known one with a wrong password,
        // which is the same enumeration leak the shared message closes — just
        // through a stopwatch instead of the response body.
        let _ = hash_password(&request.password);
        return ErrorBody::message(StatusCode::UNAUTHORIZED, INVALID_CREDENTIALS.to_owned());
    };

    if !verify_password(&request.password, &user.password_hash) {
        return ErrorBody::message(StatusCode::UNAUTHORIZED, INVALID_CREDENTIALS.to_owned());
    }

    // Checked after the password, not before: answering "that account is
    // disabled" to anyone who guesses a username is another enumeration leak.
    // A deactivated account is indistinguishable from a wrong password.
    if !user.is_active {
        return ErrorBody::message(StatusCode::UNAUTHORIZED, INVALID_CREDENTIALS.to_owned());
    }

    issue_session(&state, &user.id, &user.username).await
}

/// `POST /auth/refresh`
///
/// Rotates the presented token: a new pair is issued and the old refresh token
/// is revoked in the same breath. That way a stolen token is usable at most
/// once, and its use is detectable — the legitimate holder's next refresh fails
/// because their token was already spent.
pub async fn refresh(
    State(state): State<AppState>,
    Json(request): Json<RefreshRequest>,
) -> Response {
    let validated = match validate_refresh_token(&state.pool, &request.refresh_token).await {
        Ok(result) => result,
        Err(error) => return internal_error("validating the refresh token", error),
    };

    let Ok(valid) = validated else {
        return ErrorBody::message(StatusCode::UNAUTHORIZED, INVALID_REFRESH_TOKEN.to_owned());
    };

    // Re-read the user rather than trusting anything carried by the old token.
    // A refresh is the moment a deactivated account stops working, and the
    // moment changed permissions take effect.
    let user = sqlx::query_as::<_, UserRow>(
        "SELECT id, username, password_hash, is_active FROM users WHERE id = ?",
    )
    .bind(&valid.user_id)
    .fetch_optional(&state.pool)
    .await;

    let user = match user {
        Ok(Some(user)) if user.is_active => user,
        Ok(_) => {
            // Deleted or deactivated since the token was issued. Burn the token
            // so it cannot be tried again.
            if let Err(error) = revoke_refresh_token_by_id(&state.pool, &valid.id).await {
                return internal_error("revoking a token for an inactive user", error);
            }
            return ErrorBody::message(StatusCode::UNAUTHORIZED, INVALID_REFRESH_TOKEN.to_owned());
        }
        Err(error) => return internal_error("looking up the refreshing user", error),
    };

    if let Err(error) = revoke_refresh_token_by_id(&state.pool, &valid.id).await {
        return internal_error("rotating the refresh token", error);
    }

    issue_session(&state, &user.id, &user.username).await
}

/// `POST /auth/logout`
///
/// Always 204, whether or not the token was live. Reporting "no such token"
/// would let an unauthenticated caller test tokens, and the caller's session is
/// over either way.
///
/// Only the presented refresh token is revoked, so signing out on one device
/// leaves other sessions alone. Any access token already issued stays valid
/// until it expires — that is the cost of not checking the database on every
/// request, and the reason access tokens are short.
pub async fn logout(
    State(state): State<AppState>,
    Json(request): Json<RefreshRequest>,
) -> Response {
    match revoke_refresh_token(&state.pool, &request.refresh_token).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(error) => internal_error("revoking the refresh token", error),
    }
}

/// `GET /auth/session`
///
/// Answered from the token's own claims, with no database round trip. That is
/// the point of a signed access token: the common case costs a signature check.
/// The trade is staleness — permissions changed a minute ago may not show here
/// until the token is refreshed, bounded by the token's 15-minute life.
pub async fn session(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let Some(token) = bearer_token(&headers) else {
        return ErrorBody::message(
            StatusCode::UNAUTHORIZED,
            "missing or invalid bearer token".to_owned(),
        );
    };

    match verify_access_token(&state.jwt_secret, token) {
        Ok(claims) => Json(SessionResponse {
            authenticated: true,
            user_id: claims.sub,
            username: claims.username,
            permissions: claims.permissions,
        })
        .into_response(),
        Err(_) => ErrorBody::message(
            StatusCode::UNAUTHORIZED,
            "invalid or expired access token".to_owned(),
        ),
    }
}

/// Mints a fresh access/refresh pair for a user.
///
/// Shared by login and refresh so both paths compute permissions the same way —
/// from the database, at issuance — and neither can drift into copying stale
/// claims forward.
async fn issue_session(state: &AppState, user_id: &str, username: &str) -> Response {
    let permissions = match effective_permissions(&state.pool, user_id).await {
        Ok(permissions) => permissions,
        Err(error) => return internal_error("computing effective permissions", error),
    };

    let access = match issue_access_token(&state.jwt_secret, user_id, username, permissions) {
        Ok(access) => access,
        Err(error) => return internal_error("signing the access token", error),
    };

    let refresh_token = match issue_refresh_token(&state.pool, user_id).await {
        Ok(token) => token,
        Err(error) => return internal_error("issuing the refresh token", error),
    };

    Json(TokenResponse {
        access_token: access.token,
        refresh_token,
        expires_at: access.expires_at,
    })
    .into_response()
}

/// Extracts the token from an `Authorization: Bearer <token>` header.
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .map(str::trim)
        .filter(|token| !token.is_empty())
}

/// Unused today; kept so the authorization middleware has one place to read a
/// caller's identity from when it lands.
#[allow(
    dead_code,
    reason = "consumed by the authorization middleware, not yet written"
)]
pub async fn authenticated_user_id(
    pool: &SqlitePool,
    secret: &str,
    headers: &HeaderMap,
) -> Option<String> {
    let token = bearer_token(headers)?;
    let claims = verify_access_token(secret, token).ok()?;

    // Confirm the account still exists and is active. The token alone cannot
    // say so — it was signed before any of that could have changed.
    let row: Option<(bool,)> = sqlx::query_as("SELECT is_active FROM users WHERE id = ?")
        .bind(&claims.sub)
        .fetch_optional(pool)
        .await
        .ok()?;

    matches!(row, Some((true,))).then_some(claims.sub)
}
