//! The Loom server.
//!
//! This is the single long-running process in the system. Every client
//! (web frontend, desktop, mobile) talks to it over HTTP; it in turn depends on
//! `loom-core` for connector and business logic.
//!
//! It owns the things `loom-core` deliberately must not: the database, the
//! session credentials, and every trust decision. See `docs/ARCHITECTURE.md`
//! for why that boundary is drawn where it is, and
//! `docs/adr/0008-auth-model.md` for the auth design.
//!
//! Startup is ordered: resolve the data directory, open and migrate the
//! database, load or generate the JWT signing secret, then serve. Each step
//! depends on the last, and a failure in any of them is fatal rather than
//! degraded — a server that cannot reach its database cannot authenticate
//! anyone, and pretending otherwise would mean serving requests it has no way
//! to authorize.

use axum::{http::HeaderValue, routing::get, Json, Router};
use serde::Serialize;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;

mod auth;
mod config;
mod error;
mod routes;
mod state;

use state::AppState;

/// Default address to bind when `LOOM_BIND_ADDR` is not set.
const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8080";

/// Builds the CORS policy for the API.
///
/// Every client is a *different origin* from the backend: the web frontend is
/// served on its own port (and in production usually its own host, since it
/// deploys independently), and the Tauri clients load from `tauri://localhost`
/// or `http://tauri.localhost`. Without these headers a browser refuses to let
/// any of them read a response, which surfaces as an opaque "NetworkError"
/// rather than anything resembling the real cause.
///
/// The default allows any origin. That is deliberate and currently safe: the
/// API is unauthenticated, exposes no cookies or credentials, and a homelab
/// deployment cannot know in advance which host its frontend will be served
/// from — demanding configuration here would break zero-config startup
/// (`docs/adr/0004-zero-config-startup.md`).
///
/// **This must be revisited when auth lands.** Once the API accepts
/// credentials, `Allow-Origin: *` combined with cookie auth is unsafe, and the
/// browser will reject the combination outright. See
/// `docs/adr/0005-cors-policy.md`.
fn cors_layer() -> CorsLayer {
    let layer = CorsLayer::new().allow_methods(Any).allow_headers(Any);

    // Optional override for operators who want to pin the allowed origins.
    // Comma-separated, e.g. `https://loom.example.com,https://loom.example.org`.
    match std::env::var("LOOM_CORS_ALLOWED_ORIGINS") {
        Ok(raw) if !raw.trim().is_empty() => {
            let origins: Vec<HeaderValue> = raw
                .split(',')
                .filter_map(|origin| origin.trim().parse().ok())
                .collect();

            if origins.is_empty() {
                // Misconfigured rather than unset: fail loud in the log instead
                // of silently serving a policy nobody asked for.
                info!("LOOM_CORS_ALLOWED_ORIGINS set but no origin parsed; allowing any origin");
                layer.allow_origin(Any)
            } else {
                info!(
                    count = origins.len(),
                    "restricting CORS to configured origins"
                );
                layer.allow_origin(origins)
            }
        }
        _ => layer.allow_origin(Any),
    }
}

/// Body of the `/health` response.
#[derive(Serialize)]
struct Health {
    status: &'static str,
    core_version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health {
        status: "ok",
        core_version: loom_core::version(),
    })
}

/// Opens the database, enabling the pragmas SQLite leaves off by default, and
/// runs every pending migration.
///
/// `foreign_keys` is off by default in SQLite and silently so — declared
/// foreign keys are simply not enforced until it is switched on, per
/// connection. Turning it on here is what makes the `REFERENCES` clauses in the
/// migrations mean anything.
///
/// `journal_mode = WAL` lets reads proceed during a write, which matters as
/// soon as a status poll overlaps a login.
async fn open_database(url: &str) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .after_connect(|conn, _meta| {
            Box::pin(async move {
                sqlx::query("PRAGMA foreign_keys = ON")
                    .execute(&mut *conn)
                    .await?;
                sqlx::query("PRAGMA journal_mode = WAL")
                    .execute(&mut *conn)
                    .await?;
                Ok(())
            })
        })
        .connect(url)
        .await?;

    // Embedded at compile time from `migrations/`, so a released binary carries
    // its own schema history and needs no files alongside it.
    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}

/// Builds the application router.
///
/// Split out of [`main`] so tests can drive the real routing table through
/// `tower::ServiceExt::oneshot` instead of binding a port.
fn app(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health))
        .merge(routes::routes())
        .with_state(state)
        .layer(cors_layer())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("LOOM_LOG_LEVEL").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let bind_addr =
        std::env::var("LOOM_BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());

    let data_dir = config::data_dir()?;
    let database_path = config::database_path(&data_dir);
    info!(path = %database_path.display(), "opening database");

    let pool = open_database(&config::database_url(&database_path)).await?;
    let jwt_secret = auth::secret::load_or_create_jwt_secret(&pool).await?;

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    info!(
        addr = %listener.local_addr()?,
        core_version = loom_core::version(),
        "loom web-backend listening"
    );

    axum::serve(listener, app(AppState::new(pool, jwt_secret))).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    /// A router backed by its own throwaway database.
    ///
    /// A temp *file* rather than `:memory:`: an in-memory SQLite database lives
    /// per connection, so a pool of them would migrate one connection and hand
    /// later queries an empty database. The directory is deleted when the guard
    /// drops, so tests cannot see one another's data.
    struct TestApp {
        router: Router,
        _dir: tempfile::TempDir,
    }

    async fn test_app() -> TestApp {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = config::database_path(dir.path());
        let pool = open_database(&config::database_url(&path))
            .await
            .expect("migrations must run against a fresh database");
        let secret = auth::secret::load_or_create_jwt_secret(&pool)
            .await
            .expect("secret must be generated");

        TestApp {
            router: app(AppState::new(pool, secret)),
            _dir: dir,
        }
    }

    async fn send(app: &Router, request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = app
            .clone()
            .oneshot(request)
            .await
            .expect("the router is infallible");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body must collect")
            .to_bytes();

        let body = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };

        (status, body)
    }

    fn get(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("valid request")
    }

    fn get_with_auth(uri: &str, authorization: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("authorization", authorization)
            .body(Body::empty())
            .expect("valid request")
    }

    fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("valid request")
    }

    fn post_json_auth(uri: &str, token: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(body.to_string()))
            .expect("valid request")
    }

    fn patch_json_auth(uri: &str, token: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("PATCH")
            .uri(uri)
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(body.to_string()))
            .expect("valid request")
    }

    fn delete_auth(uri: &str, token: &str) -> Request<Body> {
        Request::builder()
            .method("DELETE")
            .uri(uri)
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .expect("valid request")
    }

    fn bearer(token: &str) -> String {
        format!("Bearer {token}")
    }

    /// Creates a user, puts them in a group with exactly `grants`, and returns
    /// their access token.
    ///
    /// Goes through the real endpoints rather than writing rows directly, so
    /// the fixtures exercise the same validation and enforcement paths the
    /// tests are about.
    async fn user_with_grants(
        app: &Router,
        admin: &str,
        username: &str,
        grants: serde_json::Value,
    ) -> String {
        let (status, group) = send(
            app,
            post_json_auth(
                "/groups",
                admin,
                serde_json::json!({
                    "name": format!("{username}-group"),
                    "description": null,
                    "permissions": grants,
                }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "group create failed: {group:#}"
        );
        let group_id = group["id"].as_str().expect("group id").to_owned();

        let (status, created) = send(
            app,
            post_json_auth(
                "/users",
                admin,
                serde_json::json!({
                    "username": username,
                    "password": "a-good-password",
                    "groupIds": [group_id],
                }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CREATED,
            "user create failed: {created:#}"
        );

        let (status, tokens) = send(
            app,
            post_json(
                "/auth/login",
                serde_json::json!({ "username": username, "password": "a-good-password" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "login failed: {tokens:#}");

        tokens["accessToken"]
            .as_str()
            .expect("accessToken")
            .to_owned()
    }

    fn setup_body() -> serde_json::Value {
        serde_json::json!({
            "instanceName": "Example Homelab",
            "adminUsername": "admin",
            "adminPassword": "a-good-password",
        })
    }

    /// Runs setup and returns the admin's first token pair.
    async fn setup_and_login(app: &Router) -> (String, String) {
        let (status, _) = send(app, post_json("/setup", setup_body())).await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            app,
            post_json(
                "/auth/login",
                serde_json::json!({ "username": "admin", "password": "a-good-password" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "login failed: {body:#}");

        (
            body["accessToken"]
                .as_str()
                .expect("accessToken")
                .to_owned(),
            body["refreshToken"]
                .as_str()
                .expect("refreshToken")
                .to_owned(),
        )
    }

    #[tokio::test]
    async fn health_reports_ok_and_the_core_version() {
        let app = test_app().await;
        let (status, body) = send(&app.router, get("/health")).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["core_version"], loom_core::version());
    }

    #[tokio::test]
    async fn migrations_run_against_a_fresh_database() {
        // `test_app` panics if migrations fail, so reaching a served response
        // proves the schema was created from nothing.
        let app = test_app().await;
        let (status, body) = send(&app.router, get("/setup/status")).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["setupComplete"], false);
    }

    #[tokio::test]
    async fn setup_creates_an_admin_in_the_seeded_group_with_every_permission() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (status, body) = send(
            &app.router,
            get_with_auth("/auth/session", &format!("Bearer {access}")),
        )
        .await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["authenticated"], true);
        assert_eq!(body["username"], "admin");
        assert!(body["userId"].as_str().is_some_and(|id| !id.is_empty()));

        // The seeded Administrators group grants every registered permission
        // globally, so the first admin's claims must contain all five with no
        // resource scoping.
        let permissions = body["permissions"].as_array().expect("permissions array");
        let mut keys: Vec<&str> = permissions
            .iter()
            .map(|grant| grant["key"].as_str().expect("key"))
            .collect();
        keys.sort_unstable();

        assert_eq!(
            keys,
            vec![
                "connectors.control",
                "connectors.view",
                "groups.manage",
                "system.settings",
                "users.manage",
            ]
        );
        assert!(
            permissions
                .iter()
                .all(|grant| grant["resourceType"].is_null() && grant["resourceId"].is_null()),
            "administrator grants must be global, got {permissions:#?}"
        );
    }

    #[tokio::test]
    async fn setup_status_flips_after_setup() {
        let app = test_app().await;

        let (_, before) = send(&app.router, get("/setup/status")).await;
        assert_eq!(before["setupComplete"], false);

        let (status, body) = send(&app.router, post_json("/setup", setup_body())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["setupComplete"], true);

        let (_, after) = send(&app.router, get("/setup/status")).await;
        assert_eq!(after["setupComplete"], true);
    }

    #[tokio::test]
    async fn setup_twice_conflicts() {
        let app = test_app().await;

        let (first, _) = send(&app.router, post_json("/setup", setup_body())).await;
        assert_eq!(first, StatusCode::OK);

        let (second, body) = send(&app.router, post_json("/setup", setup_body())).await;
        assert_eq!(second, StatusCode::CONFLICT);
        assert!(body["error"].as_str().is_some_and(|e| !e.is_empty()));
    }

    #[tokio::test]
    async fn setup_rejects_a_short_password_and_empty_fields() {
        let app = test_app().await;

        let (status, body) = send(
            &app.router,
            post_json(
                "/setup",
                serde_json::json!({
                    "instanceName": "Example",
                    "adminUsername": "admin",
                    "adminPassword": "short",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().expect("error").contains("8"));

        let (status, _) = send(
            &app.router,
            post_json(
                "/setup",
                serde_json::json!({
                    "instanceName": "  ",
                    "adminUsername": "admin",
                    "adminPassword": "a-good-password",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // A rejected setup must leave the instance unconfigured.
        let (_, body) = send(&app.router, get("/setup/status")).await;
        assert_eq!(body["setupComplete"], false);
    }

    #[tokio::test]
    async fn login_rejects_a_wrong_password_and_an_unknown_user_identically() {
        let app = test_app().await;
        let (status, _) = send(&app.router, post_json("/setup", setup_body())).await;
        assert_eq!(status, StatusCode::OK);

        let (wrong_password, wrong_body) = send(
            &app.router,
            post_json(
                "/auth/login",
                serde_json::json!({ "username": "admin", "password": "not-the-password" }),
            ),
        )
        .await;

        let (unknown_user, unknown_body) = send(
            &app.router,
            post_json(
                "/auth/login",
                serde_json::json!({ "username": "nobody", "password": "a-good-password" }),
            ),
        )
        .await;

        assert_eq!(wrong_password, StatusCode::UNAUTHORIZED);
        assert_eq!(unknown_user, StatusCode::UNAUTHORIZED);
        // Identical responses: the endpoint must not reveal which usernames
        // exist.
        assert_eq!(wrong_body, unknown_body);
    }

    #[tokio::test]
    async fn session_rejects_a_missing_or_bad_token() {
        let app = test_app().await;
        setup_and_login(&app.router).await;

        for request in [
            get("/auth/session"),
            get_with_auth("/auth/session", "Bearer not-a-token"),
            get_with_auth("/auth/session", "Basic abc"),
        ] {
            let (status, _) = send(&app.router, request).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn a_token_from_another_instance_is_rejected() {
        // Each instance generates its own signing secret, so a token minted by
        // one must not authenticate against another.
        let first = test_app().await;
        let second = test_app().await;

        let (access, _) = setup_and_login(&first.router).await;
        setup_and_login(&second.router).await;

        let (status, _) = send(
            &second.router,
            get_with_auth("/auth/session", &format!("Bearer {access}")),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// The full session lifecycle, which is the flow every client walks.
    #[tokio::test]
    async fn setup_login_refresh_logout_and_replay() {
        let app = test_app().await;
        let (access, refresh) = setup_and_login(&app.router).await;

        // The access token authenticates.
        let (status, _) = send(
            &app.router,
            get_with_auth("/auth/session", &format!("Bearer {access}")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Refreshing yields a new pair.
        let (status, body) = send(
            &app.router,
            post_json(
                "/auth/refresh",
                serde_json::json!({ "refreshToken": refresh }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "refresh failed: {body:#}");
        let rotated = body["refreshToken"]
            .as_str()
            .expect("refreshToken")
            .to_owned();
        let new_access = body["accessToken"]
            .as_str()
            .expect("accessToken")
            .to_owned();
        assert_ne!(rotated, refresh, "the refresh token must rotate");

        // The new access token works, and carries the same permissions.
        let (status, session) = send(
            &app.router,
            get_with_auth("/auth/session", &format!("Bearer {new_access}")),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(session["permissions"].as_array().expect("array").len(), 5);

        // The old refresh token was revoked by rotation and cannot be replayed.
        let (status, _) = send(
            &app.router,
            post_json(
                "/auth/refresh",
                serde_json::json!({ "refreshToken": refresh }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "a spent token must not work"
        );

        // Logout revokes the rotated token.
        let (status, _) = send(
            &app.router,
            post_json(
                "/auth/logout",
                serde_json::json!({ "refreshToken": rotated }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, _) = send(
            &app.router,
            post_json(
                "/auth/refresh",
                serde_json::json!({ "refreshToken": rotated }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn logout_with_an_unknown_token_still_succeeds() {
        // Reporting "no such token" would let an unauthenticated caller probe
        // token validity.
        let app = test_app().await;
        setup_and_login(&app.router).await;

        let (status, _) = send(
            &app.router,
            post_json(
                "/auth/logout",
                serde_json::json!({ "refreshToken": "nonsense" }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn connector_routes_work_for_an_administrator() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (status, body) =
            send(&app.router, get_with_auth("/connectors", &bearer(&access))).await;
        assert_eq!(status, StatusCode::OK);
        let entries = body.as_array().expect("array");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["metadata"]["id"], "mock");
        assert_eq!(entries[0]["status"]["health"], "healthy");

        let ids: Vec<&str> = entries[0]["actions"]
            .as_array()
            .expect("actions")
            .iter()
            .map(|action| action["id"].as_str().expect("id"))
            .collect();
        assert!(ids.contains(&"restart") && ids.contains(&"ping"), "{ids:?}");

        let (status, body) = send(
            &app.router,
            post_json_auth(
                "/connectors/mock/actions/restart",
                &access,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["success"], true);

        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/connectors/nope/actions/ping",
                &access,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// Unauthenticated access must be 401, not 403: there is no identity yet.
    #[tokio::test]
    async fn connector_routes_reject_an_anonymous_caller() {
        let app = test_app().await;
        setup_and_login(&app.router).await;

        let (list, _) = send(&app.router, get("/connectors")).await;
        assert_eq!(list, StatusCode::UNAUTHORIZED);

        let (action, _) = send(
            &app.router,
            post_json("/connectors/mock/actions/ping", serde_json::json!({})),
        )
        .await;
        assert_eq!(action, StatusCode::UNAUTHORIZED);
    }

    /* ---------------------------------------------------------------- */
    /* Permission enforcement                                            */
    /* ---------------------------------------------------------------- */

    /// Authenticated but ungranted must be **403, not 401**. A 401 would tell
    /// the client to retry the login it already completed.
    #[tokio::test]
    async fn a_user_without_a_grant_is_forbidden_not_unauthorized() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let nobody = user_with_grants(&app.router, &admin, "nobody", serde_json::json!([])).await;

        for request in [
            get_with_auth("/connectors", &bearer(&nobody)),
            get_with_auth("/users", &bearer(&nobody)),
            get_with_auth("/groups", &bearer(&nobody)),
            get_with_auth("/permissions", &bearer(&nobody)),
        ] {
            let (status, body) = send(&app.router, request).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "body was {body:#}");
            assert!(body["error"].as_str().is_some_and(|e| !e.is_empty()));
        }

        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/connectors/mock/actions/ping",
                &nobody,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    /// The three cases the resource-scoped check has to get right.
    #[tokio::test]
    async fn connector_action_respects_scoped_global_and_absent_grants() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        // 1. A grant naming exactly the mock connector.
        let scoped = user_with_grants(
            &app.router,
            &admin,
            "scoped",
            serde_json::json!([{
                "key": "connectors.control",
                "resourceType": "connector",
                "resourceId": "mock",
            }]),
        )
        .await;

        // 2. A global grant.
        let global = user_with_grants(
            &app.router,
            &admin,
            "global",
            serde_json::json!([{
                "key": "connectors.control",
                "resourceType": null,
                "resourceId": null,
            }]),
        )
        .await;

        // 3. A grant for a *different* connector.
        let elsewhere = user_with_grants(
            &app.router,
            &admin,
            "elsewhere",
            serde_json::json!([{
                "key": "connectors.control",
                "resourceType": "connector",
                "resourceId": "some-other-connector",
            }]),
        )
        .await;

        let act = |token: &str| {
            post_json_auth(
                "/connectors/mock/actions/ping",
                token,
                serde_json::json!({}),
            )
        };

        let (status, body) = send(&app.router, act(&scoped)).await;
        assert_eq!(status, StatusCode::OK, "scoped grant must work: {body:#}");

        let (status, body) = send(&app.router, act(&global)).await;
        assert_eq!(status, StatusCode::OK, "global grant must work: {body:#}");

        // The whole point of scoping: a grant for another connector authorizes
        // nothing here.
        let (status, _) = send(&app.router, act(&elsewhere)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // And a connector-scoped grant is not authority over connectors at
        // large, so the global-only list endpoint still refuses it.
        let (status, _) = send(&app.router, get_with_auth("/connectors", &bearer(&scoped))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    /// A 403 must not depend on whether the resource exists, or the endpoint
    /// becomes a way to enumerate configured connectors.
    #[tokio::test]
    async fn an_unauthorized_action_does_not_reveal_whether_the_connector_exists() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let nobody = user_with_grants(&app.router, &admin, "nobody", serde_json::json!([])).await;

        let (real, real_body) = send(
            &app.router,
            post_json_auth(
                "/connectors/mock/actions/ping",
                &nobody,
                serde_json::json!({}),
            ),
        )
        .await;
        let (fake, fake_body) = send(
            &app.router,
            post_json_auth(
                "/connectors/ghost/actions/ping",
                &nobody,
                serde_json::json!({}),
            ),
        )
        .await;

        assert_eq!(real, StatusCode::FORBIDDEN);
        assert_eq!(fake, StatusCode::FORBIDDEN);
        assert_eq!(real_body, fake_body);
    }

    /* ---------------------------------------------------------------- */
    /* User and group administration                                     */
    /* ---------------------------------------------------------------- */

    #[tokio::test]
    async fn users_are_listed_without_password_hashes() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        let (status, body) = send(&app.router, get_with_auth("/users", &bearer(&admin))).await;

        assert_eq!(status, StatusCode::OK);
        let users = body.as_array().expect("array");
        assert_eq!(users.len(), 1);
        assert_eq!(users[0]["username"], "admin");
        assert_eq!(users[0]["isActive"], true);
        assert_eq!(users[0]["groupIds"].as_array().expect("groups").len(), 1);

        // The serialized text must not contain a hash anywhere, under any key.
        let serialized = body.to_string();
        assert!(!serialized.contains("password"), "leaked: {serialized}");
        assert!(!serialized.contains("argon2"), "leaked: {serialized}");
    }

    #[tokio::test]
    async fn creating_a_user_enforces_the_same_password_floor_as_setup() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        let (status, body) = send(
            &app.router,
            post_json_auth(
                "/users",
                &admin,
                serde_json::json!({ "username": "shorty", "password": "short" }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"].as_str().expect("error").contains("8"));
    }

    #[tokio::test]
    async fn duplicate_usernames_conflict() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/users",
                &admin,
                serde_json::json!({ "username": "admin", "password": "a-good-password" }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::CONFLICT);
    }

    #[tokio::test]
    async fn the_permission_catalog_lists_every_registered_key() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        let (status, body) =
            send(&app.router, get_with_auth("/permissions", &bearer(&admin))).await;

        assert_eq!(status, StatusCode::OK);
        let keys: Vec<&str> = body
            .as_array()
            .expect("array")
            .iter()
            .map(|entry| entry["key"].as_str().expect("key"))
            .collect();

        assert_eq!(
            keys,
            vec![
                "connectors.control",
                "connectors.view",
                "groups.manage",
                "system.settings",
                "users.manage",
            ]
        );
    }

    /* ---------------------------------------------------------------- */
    /* Safeguards — a bug here locks an operator out of their instance    */
    /* ---------------------------------------------------------------- */

    #[tokio::test]
    async fn the_last_administrator_cannot_be_deactivated() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        // A second user who is NOT an administrator, so deactivating the admin
        // would genuinely leave nobody.
        let (status, other) = send(
            &app.router,
            post_json_auth(
                "/users",
                &admin,
                serde_json::json!({ "username": "regular", "password": "a-good-password" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let _ = other;

        let (status, users) = send(&app.router, get_with_auth("/users", &bearer(&admin))).await;
        assert_eq!(status, StatusCode::OK);
        let admin_id = users
            .as_array()
            .expect("array")
            .iter()
            .find(|user| user["username"] == "admin")
            .expect("admin present")["id"]
            .as_str()
            .expect("id")
            .to_owned();

        // Another administrator performs the change, so the self-removal rule
        // is not what is being tested here.
        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/users",
                &admin,
                serde_json::json!({
                    "username": "admin2",
                    "password": "a-good-password",
                    "groupIds": ["00000000-0000-4000-8000-000000000001"],
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, tokens) = send(
            &app.router,
            post_json(
                "/auth/login",
                serde_json::json!({ "username": "admin2", "password": "a-good-password" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let admin2 = tokens["accessToken"].as_str().expect("token").to_owned();

        // With two administrators, deactivating one is allowed.
        let (status, body) = send(
            &app.router,
            patch_json_auth(
                &format!("/users/{admin_id}"),
                &admin2,
                serde_json::json!({ "isActive": false }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:#}");

        // Now admin2 is the last one, and cannot be removed by anyone.
        let (status, users) = send(&app.router, get_with_auth("/users", &bearer(&admin2))).await;
        assert_eq!(status, StatusCode::OK);
        let admin2_id = users
            .as_array()
            .expect("array")
            .iter()
            .find(|user| user["username"] == "admin2")
            .expect("admin2 present")["id"]
            .as_str()
            .expect("id")
            .to_owned();

        // Reactivate the first admin so a non-self caller exists again, then
        // have them try to remove the last *active* administrator.
        let (status, _) = send(
            &app.router,
            patch_json_auth(
                &format!("/users/{admin_id}"),
                &admin2,
                serde_json::json!({ "isActive": true }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Two active admins again; remove one, leaving exactly one.
        let (status, _) = send(
            &app.router,
            patch_json_auth(
                &format!("/users/{admin2_id}"),
                &admin,
                serde_json::json!({ "isActive": false }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // `admin` is now the only active administrator. admin2 is deactivated,
        // so use admin's own token — and the self-removal rule catches it first,
        // which is itself the protection. Verify the last-admin rule directly by
        // stripping admin's groups via a second administrator instead.
        let (status, body) = send(
            &app.router,
            patch_json_auth(
                &format!("/users/{admin_id}"),
                &admin,
                serde_json::json!({ "groupIds": [] }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "removing the last administrator's group must be refused: {body:#}"
        );
        assert!(body["error"]
            .as_str()
            .expect("error")
            .contains("no active administrator"));

        // And the instance is still administrable.
        let (status, _) = send(&app.router, get_with_auth("/users", &bearer(&admin))).await;
        assert_eq!(status, StatusCode::OK);
    }

    /// Deletion of the final administrator must be refused by the last-admin
    /// safeguard itself.
    ///
    /// The caller here holds `users.manage` through an ordinary group and is
    /// **not** an administrator, so nothing about this is caught by the
    /// self-removal rule — an earlier version of this test leaned on that
    /// accidentally and kept passing when the safeguard was disabled.
    #[tokio::test]
    async fn the_last_administrator_cannot_be_deleted() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        // A user manager who is not in the protected group.
        let manager = user_with_grants(
            &app.router,
            &admin,
            "manager",
            serde_json::json!([{
                "key": "users.manage",
                "resourceType": null,
                "resourceId": null,
            }]),
        )
        .await;

        // A second administrator, so the first delete is legitimately allowed.
        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/users",
                &admin,
                serde_json::json!({
                    "username": "admin2",
                    "password": "a-good-password",
                    "groupIds": ["00000000-0000-4000-8000-000000000001"],
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (_, users) = send(&app.router, get_with_auth("/users", &bearer(&admin))).await;
        let find = |name: &str| {
            users
                .as_array()
                .expect("array")
                .iter()
                .find(|user| user["username"] == name)
                .expect("user present")["id"]
                .as_str()
                .expect("id")
                .to_owned()
        };
        let admin_id = find("admin");
        let admin2_id = find("admin2");

        // Two administrators: deleting one is fine.
        let (status, body) = send(
            &app.router,
            delete_auth(&format!("/users/{admin_id}"), &manager),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body:#}");

        // One left. The manager is not that administrator and is not deleting
        // themselves, so only the last-admin safeguard can refuse this.
        let (status, body) = send(
            &app.router,
            delete_auth(&format!("/users/{admin2_id}"), &manager),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body:#}");
        assert!(body["error"]
            .as_str()
            .expect("error")
            .contains("no active administrator"));

        // The row survived, and the instance still has an administrator.
        let (status, users) = send(&app.router, get_with_auth("/users", &bearer(&manager))).await;
        assert_eq!(status, StatusCode::OK);
        assert!(users
            .as_array()
            .expect("array")
            .iter()
            .any(|user| user["username"] == "admin2" && user["isActive"] == true));
    }

    /// Deactivating the final administrator must be refused too — the same
    /// safeguard, reached by a different route. Again performed by a
    /// non-administrator manager so the self-removal rule cannot mask it.
    #[tokio::test]
    async fn the_last_administrator_cannot_be_deactivated_by_someone_else() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        let manager = user_with_grants(
            &app.router,
            &admin,
            "manager",
            serde_json::json!([{
                "key": "users.manage",
                "resourceType": null,
                "resourceId": null,
            }]),
        )
        .await;

        let (_, users) = send(&app.router, get_with_auth("/users", &bearer(&admin))).await;
        let admin_id = users
            .as_array()
            .expect("array")
            .iter()
            .find(|user| user["username"] == "admin")
            .expect("admin present")["id"]
            .as_str()
            .expect("id")
            .to_owned();

        let (status, body) = send(
            &app.router,
            patch_json_auth(
                &format!("/users/{admin_id}"),
                &manager,
                serde_json::json!({ "isActive": false }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body:#}");

        // And stripping their group membership is refused by the same check.
        let (status, body) = send(
            &app.router,
            patch_json_auth(
                &format!("/users/{admin_id}"),
                &manager,
                serde_json::json!({ "groupIds": [] }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body:#}");

        // Still an administrator, still active.
        let (_, users) = send(&app.router, get_with_auth("/users", &bearer(&admin))).await;
        let row = users
            .as_array()
            .expect("array")
            .iter()
            .find(|user| user["username"] == "admin")
            .expect("admin present")
            .clone();
        assert_eq!(row["isActive"], true);
        assert_eq!(row["groupIds"].as_array().expect("groups").len(), 1);
    }

    #[tokio::test]
    async fn a_user_cannot_deactivate_or_delete_themselves() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        // A second administrator exists, so the last-admin rule is not what is
        // doing the refusing here.
        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/users",
                &admin,
                serde_json::json!({
                    "username": "admin2",
                    "password": "a-good-password",
                    "groupIds": ["00000000-0000-4000-8000-000000000001"],
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (_, users) = send(&app.router, get_with_auth("/users", &bearer(&admin))).await;
        let admin_id = users
            .as_array()
            .expect("array")
            .iter()
            .find(|user| user["username"] == "admin")
            .expect("admin present")["id"]
            .as_str()
            .expect("id")
            .to_owned();

        let (status, body) = send(
            &app.router,
            patch_json_auth(
                &format!("/users/{admin_id}"),
                &admin,
                serde_json::json!({ "isActive": false }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body["error"].as_str().expect("error").contains("your own"));

        let (status, body) = send(
            &app.router,
            delete_auth(&format!("/users/{admin_id}"), &admin),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body["error"].as_str().expect("error").contains("your own"));

        // Still active and still there.
        let (_, users) = send(&app.router, get_with_auth("/users", &bearer(&admin))).await;
        let admin_row = users
            .as_array()
            .expect("array")
            .iter()
            .find(|user| user["username"] == "admin")
            .expect("admin still present")
            .clone();
        assert_eq!(admin_row["isActive"], true);
    }

    #[tokio::test]
    async fn the_protected_group_cannot_be_deleted_even_after_a_rename() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        let (status, groups) = send(&app.router, get_with_auth("/groups", &bearer(&admin))).await;
        assert_eq!(status, StatusCode::OK);
        let administrators = groups
            .as_array()
            .expect("array")
            .iter()
            .find(|group| group["isProtected"] == true)
            .expect("a protected group exists")
            .clone();
        let group_id = administrators["id"].as_str().expect("id").to_owned();
        assert_eq!(administrators["name"], "Administrators");
        assert_eq!(administrators["memberCount"], 1);

        let (status, body) = send(
            &app.router,
            delete_auth(&format!("/groups/{group_id}"), &admin),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert!(body["error"].as_str().expect("error").contains("protected"));

        // Renaming is allowed — and must not disable the protection, which is
        // exactly what a name-matching check would do.
        let (status, renamed) = send(
            &app.router,
            patch_json_auth(
                &format!("/groups/{group_id}"),
                &admin,
                serde_json::json!({ "name": "Overlords" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{renamed:#}");
        assert_eq!(renamed["name"], "Overlords");
        assert_eq!(renamed["isProtected"], true);

        let (status, _) = send(
            &app.router,
            delete_auth(&format!("/groups/{group_id}"), &admin),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::CONFLICT,
            "a rename must not remove the protection"
        );
    }

    #[tokio::test]
    async fn an_ordinary_group_can_be_created_edited_and_deleted() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        let (status, created) = send(
            &app.router,
            post_json_auth(
                "/groups",
                &admin,
                serde_json::json!({
                    "name": "Viewers",
                    "description": "Read-only access.",
                    "permissions": [{
                        "key": "connectors.view",
                        "resourceType": null,
                        "resourceId": null,
                    }],
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created:#}");
        assert_eq!(created["isProtected"], false);
        let group_id = created["id"].as_str().expect("id").to_owned();

        let (status, updated) = send(
            &app.router,
            patch_json_auth(
                &format!("/groups/{group_id}"),
                &admin,
                serde_json::json!({
                    "permissions": [
                        { "key": "connectors.view", "resourceType": null, "resourceId": null },
                        {
                            "key": "connectors.control",
                            "resourceType": "connector",
                            "resourceId": "mock",
                        },
                    ],
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{updated:#}");
        assert_eq!(updated["permissions"].as_array().expect("array").len(), 2);

        let (status, _) = send(
            &app.router,
            delete_auth(&format!("/groups/{group_id}"), &admin),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
    }

    /// An unregistered permission key must be refused, not stored as a grant
    /// that silently authorizes nothing.
    #[tokio::test]
    async fn an_unregistered_permission_key_is_rejected() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/groups",
                &admin,
                serde_json::json!({
                    "name": "Typo",
                    "description": null,
                    "permissions": [{
                        "key": "connectors.contorl",
                        "resourceType": null,
                        "resourceId": null,
                    }],
                }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}
