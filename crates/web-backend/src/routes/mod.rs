//! HTTP routes.
//!
//! Note the absence of an `/api` prefix throughout. The backend's URL space is
//! flat: the prefix belongs to whatever routes traffic *to* the backend, not to
//! the backend itself. The web frontend is served by nginx (or the Vite dev
//! server), which proxies `/api/*` here and strips the prefix, so the browser
//! stays same-origin; the desktop and mobile clients have no proxy in the path
//! and call these paths directly. See
//! `docs/adr/0006-frontend-api-same-origin.md`.

use axum::routing::{get, patch, post};
use axum::Router;

use crate::state::AppState;

pub mod auth;
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
        .route("/connectors", get(connectors::list_connectors))
        .route(
            "/connectors/{id}/actions/{action_id}",
            post(connectors::execute_action),
        )
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
