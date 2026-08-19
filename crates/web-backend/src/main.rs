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
    async fn connector_routes_still_work_after_the_stub_was_removed() {
        let app = test_app().await;

        let (status, body) = send(&app.router, get("/connectors")).await;
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
            post_json("/connectors/mock/actions/restart", serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["success"], true);

        let (status, _) = send(
            &app.router,
            post_json("/connectors/nope/actions/ping", serde_json::json!({})),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
