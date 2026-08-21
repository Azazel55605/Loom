//! HTTP routes.
//!
//! Note the absence of an `/api` prefix throughout. The backend's URL space is
//! flat: the prefix belongs to whatever routes traffic *to* the backend, not to
//! the backend itself. The web frontend is served by nginx (or the Vite dev
//! server), which proxies `/api/*` here and strips the prefix, so the browser
//! stays same-origin; the desktop and mobile clients have no proxy in the path
//! and call these paths directly. See
//! `docs/adr/0006-frontend-api-same-origin.md`.

use axum::extract::DefaultBodyLimit;
use axum::routing::{get, patch, post};
use axum::Router;

use crate::state::AppState;

/// Deserializes a field that distinguishes *absent* from *present and null*.
///
/// `Option<Option<T>>` alone does not do this, which is a trap worth naming:
/// with a plain `#[serde(default)]`, an explicit `null` deserializes to the
/// **outer** `None` — indistinguishable from the field being missing. Any
/// handler reading it as "absent leaves it alone, null clears it" therefore
/// silently loses the ability to clear, and does so without a type error.
///
/// This wraps whatever was there in `Some`, so the outer layer means "the
/// caller mentioned this field" and the inner one carries the value:
///
/// - field absent      → `None`        (leave it alone; supplied by `default`)
/// - `"name": null`    → `Some(None)`  (clear it)
/// - `"name": "value"` → `Some(Some(…))`
///
/// Use with `#[serde(default, deserialize_with = "present_option")]`.
pub fn present_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

pub mod account;
pub mod auth;
pub mod connector_socket;
pub mod connectors;
pub mod groups;
pub mod setup;
pub mod users;

/// Every application route, ready to be merged into the root router.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/setup/status", get(setup::setup_status))
        .route("/setup", post(setup::complete_setup))
        .route("/auth/login", post(auth::login))
        .route("/auth/refresh", post(auth::refresh))
        .route("/auth/logout", post(auth::logout))
        .route("/auth/session", get(auth::session))
        .route(
            "/account",
            get(account::get_account).patch(account::update_account),
        )
        .route("/account/password", post(account::change_password))
        .route(
            "/account/avatar",
            post(account::upload_avatar)
                .delete(account::delete_avatar)
                // On this method router, not on the `Router` — `Router::layer`
                // applies to every route registered before it, which would
                // quietly raise the limit on half the API depending on where
                // the call sits in this chain. axum's global default is 2 MB,
                // which an avatar at exactly the limit would breach once
                // multipart framing is counted. DELETE carries no body, so
                // sharing the raised limit with it costs nothing.
                .layer(DefaultBodyLimit::max(
                    account::MAX_AVATAR_BYTES + account::AVATAR_BODY_SLACK_BYTES,
                )),
        )
        .route("/connector-types", get(connectors::list_connector_types))
        .route(
            "/connector-instances",
            get(connectors::list_instances).post(connectors::create_instance),
        )
        .route(
            "/connector-instances/{id}",
            get(connectors::get_instance)
                .patch(connectors::update_instance)
                .delete(connectors::delete_instance),
        )
        .route(
            "/connector-instances/{id}/actions/{action_id}",
            post(connectors::execute_action),
        )
        .route("/ws", get(connector_socket::connector_status_socket))
        .route("/users", get(users::list_users).post(users::create_user))
        .route(
            "/users/{id}",
            patch(users::update_user).delete(users::delete_user),
        )
        .route(
            "/groups",
            get(groups::list_groups).post(groups::create_group),
        )
        .route(
            "/groups/{id}",
            patch(groups::update_group).delete(groups::delete_group),
        )
        .route("/permissions", get(groups::list_permissions))
}
