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
//! database, load or generate the JWT signing and connector-config encryption
//! keys, then serve. Each step depends on the last, and a failure in any of
//! them is fatal rather than degraded — a server that cannot reach its database
//! or recover its durable keys cannot safely serve requests.

use std::path::Path;

use axum::{http::HeaderValue, routing::get, Json, Router};
use serde::Serialize;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::SqlitePool;
use tower_http::cors::{Any, CorsLayer};
use tower_http::services::ServeDir;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod auth;
mod config;
mod connectors;
mod dashboard_access;
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
/// deploys independently), Desktop loads from `tauri://localhost` or a mapped
/// localhost scheme, and Android's default mapped origin is
/// `http://tauri.localhost`. Without these headers a webview refuses to let any
/// of them read a response, which surfaces as an opaque "NetworkError" rather
/// than anything resembling the real cause.
///
/// The browser frontend is same-origin in the normal proxy deployment. The
/// explicit localhost origin supports direct development, and operators may
/// append other browser origins through `LOOM_CORS_ALLOWED_ORIGINS`.
///
/// Tauri's known webview origins are always present. Loom authenticates with a
/// Bearer token in `Authorization`, not a cookie, so a different web page has
/// no ambient credential for its browser to attach. This avoids the classic
/// cookie-CSRF risk while keeping arbitrary browser origins out of the policy.
fn cors_layer() -> CorsLayer {
    cors_layer_from(std::env::var("LOOM_CORS_ALLOWED_ORIGINS").ok().as_deref())
}

fn cors_layer_from(configured: Option<&str>) -> CorsLayer {
    const BUILT_IN_ORIGINS: [&str; 4] = [
        "http://localhost:3000",
        "tauri://localhost",
        "https://tauri.localhost",
        "http://tauri.localhost",
    ];

    let mut origins: Vec<HeaderValue> = BUILT_IN_ORIGINS
        .into_iter()
        .map(HeaderValue::from_static)
        .collect();

    if let Some(raw) = configured.filter(|value| !value.trim().is_empty()) {
        let configured_origins: Vec<HeaderValue> = raw
            .split(',')
            .filter_map(|origin| origin.trim().parse().ok())
            .collect();
        info!(
            count = configured_origins.len(),
            "appending configured CORS origins"
        );
        origins.extend(configured_origins);
    }

    CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_origin(origins)
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
    // Cloned before `with_state` consumes the state: the file service and the
    // upload handler must point at the same directory, and taking it from one
    // place is what guarantees they do.
    let avatars_dir = state.avatars_dir.as_ref().clone();

    Router::new()
        .route("/health", get(health))
        .merge(routes::routes())
        .with_state(state)
        .nest_service("/avatars", avatar_service(&avatars_dir))
        .layer(cors_layer())
}

/// Read-only static serving for uploaded avatars.
///
/// `ServeDir` answers GET and HEAD and nothing else, so there is no write path
/// here regardless of what a client sends. It resolves requests inside the
/// given directory and rejects anything escaping it, which is what stops a
/// `..` in a URL reading the database file sitting one level up.
///
/// Directory listing is off — `ServeDir` has no listing behaviour to begin
/// with, and `append_index_html_on_directories(false)` also stops a request for
/// a directory being answered with an `index.html` that happened to be uploaded
/// into it.
///
/// The files are served unauthenticated. That is a deliberate, narrow
/// trade-off: an avatar URL is embedded in `<img>` tags all over the interface,
/// and browsers do not attach an `Authorization` header to image loads, so
/// authenticating them would mean either cookies or signed URLs. What leaks is
/// a profile picture to whoever can already reach the server *and* guess a
/// random UUIDv4 filename. **Revisit if avatars ever stop being the only thing
/// in this directory.**
fn avatar_service(dir: &Path) -> ServeDir {
    ServeDir::new(dir).append_index_html_on_directories(false)
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
    let loaded_config_key =
        connectors::config_secrets::load_or_create_config_encryption_key(&pool).await?;
    if loaded_config_key.generated {
        info!("generated connector config encryption key");
    } else {
        info!("loaded existing connector config encryption key");
    }
    let avatars_dir = config::avatars_dir(&data_dir)?;

    // Built after the database is migrated: the runtime's whole job is to hold
    // the live form of what is stored in `connector_instances`.
    let connectors = connectors::ConnectorRuntime::load(
        &pool,
        connectors::builtin_registry(),
        &loaded_config_key.key,
    )
    .await?;
    let _poller = connectors.spawn_poller();

    let state = AppState::new(
        pool,
        jwt_secret,
        loaded_config_key.key,
        avatars_dir,
        connectors,
    );
    // A second background task, deliberately not folded into the poller: one
    // asks a local daemon how things are every few seconds, the other asks a
    // third-party registry what exists every few hours. See
    // `connectors::updates`.
    let _update_scheduler = connectors::updates::spawn_scheduler(state.clone());

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    info!(
        addr = %listener.local_addr()?,
        core_version = loom_core::version(),
        "loom web-backend listening"
    );

    axum::serve(
        listener,
        app(state).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use axum::{
        body::Body,
        extract::ConnectInfo,
        http::{Request, StatusCode},
    };
    use chrono::TimeZone;
    use http_body_util::BodyExt;
    use loom_core::connector::debug::DebugConnector;
    use loom_core::connector::{
        ActionResult, Connector, ConnectorAction, ConnectorError, ConnectorMetadata,
        ConnectorStatus, DataPointDescriptor, DisplayField, WidgetLayout,
    };
    use serde_json::Value;
    use std::net::SocketAddr;
    use std::time::Duration;
    use tower::ServiceExt;

    /// A router backed by its own throwaway database.
    ///
    /// A temp *file* rather than `:memory:`: an in-memory SQLite database lives
    /// per connection, so a pool of them would migrate one connection and hand
    /// later queries an empty database. The directory is deleted when the guard
    /// drops, so tests cannot see one another's data.
    struct TestApp {
        router: Router,
        connectors: connectors::ConnectorRuntime,
        pool: SqlitePool,
        /// The same state the router serves from, so a test can drive a
        /// background task — the update scheduler — against the very instances
        /// the HTTP tests are looking at.
        state: AppState,
        /// Kept so tests can look at the avatar directory on disk — the point
        /// of several of them is that a file is really there, or really gone.
        dir: tempfile::TempDir,
    }

    impl TestApp {
        /// Path the avatar files are written to.
        fn avatars_dir(&self) -> std::path::PathBuf {
            self.dir.path().join(config::AVATARS_DIRNAME)
        }

        /// The files currently in the avatar directory.
        fn avatar_files(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(self.avatars_dir())
                .expect("avatar directory must exist")
                .map(|entry| {
                    entry
                        .expect("readable entry")
                        .file_name()
                        .to_string_lossy()
                        .into_owned()
                })
                .collect();
            names.sort();
            names
        }
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
        let config_key = connectors::config_secrets::load_or_create_config_encryption_key(&pool)
            .await
            .expect("config encryption key must be generated")
            .key;
        let avatars = config::avatars_dir(dir.path()).expect("avatar directory must be created");
        let connectors =
            connectors::ConnectorRuntime::load(&pool, connectors::builtin_registry(), &config_key)
                .await
                .expect("an empty connector table must load");
        connectors.poll_once().await;

        let state = AppState::new(
            pool.clone(),
            secret,
            config_key,
            avatars,
            connectors.clone(),
        );

        TestApp {
            router: app(state.clone()),
            connectors,
            pool,
            state,
            dir,
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

    async fn send_response(app: &Router, request: Request<Body>) -> axum::response::Response {
        app.clone()
            .oneshot(request)
            .await
            .expect("the router is infallible")
    }

    fn get(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .body(Body::empty())
            .expect("valid request")
    }

    fn websocket_upgrade(uri: &str) -> Request<Body> {
        Request::builder()
            .uri(uri)
            .header("connection", "upgrade")
            .header("upgrade", "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .body(Body::empty())
            .expect("valid websocket upgrade request")
    }

    #[tokio::test]
    async fn connector_websocket_rejects_missing_and_invalid_access_tokens() {
        let app = test_app().await;

        for uri in ["/ws", "/ws?token=not-a-jwt"] {
            let (status, _) = send(&app.router, websocket_upgrade(uri)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn cors_allows_known_webview_origins_and_configured_browser_origins() {
        let allowed = [
            "tauri://localhost",
            "https://tauri.localhost",
            "http://tauri.localhost",
            "https://loom.example.com",
        ];

        for origin in allowed {
            let response = Router::new()
                .route("/health", axum::routing::get(health))
                .layer(cors_layer_from(Some("https://loom.example.com")))
                .oneshot(
                    Request::builder()
                        .uri("/health")
                        .header("origin", origin)
                        .body(Body::empty())
                        .expect("valid request"),
                )
                .await
                .expect("the router is infallible");

            assert_eq!(
                response.headers().get("access-control-allow-origin"),
                Some(&HeaderValue::from_str(origin).expect("valid test origin")),
                "origin {origin} should be allowed"
            );
        }

        let response = Router::new()
            .route("/health", axum::routing::get(health))
            .layer(cors_layer_from(None))
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .header("origin", "https://unlisted.example")
                    .body(Body::empty())
                    .expect("valid request"),
            )
            .await
            .expect("the router is infallible");

        assert!(response
            .headers()
            .get("access-control-allow-origin")
            .is_none());
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

    fn with_client_context(
        mut request: Request<Body>,
        address: [u8; 4],
        user_agent: &str,
    ) -> Request<Body> {
        request
            .extensions_mut()
            .insert(ConnectInfo(SocketAddr::from((address, 4242))));
        request.headers_mut().insert(
            axum::http::header::USER_AGENT,
            HeaderValue::from_str(user_agent).expect("valid user agent"),
        );
        request
    }

    async fn login_from(
        app: &Router,
        username: &str,
        password: &str,
        address: [u8; 4],
        user_agent: &str,
    ) -> (StatusCode, serde_json::Value) {
        send(
            app,
            with_client_context(
                post_json(
                    "/auth/login",
                    serde_json::json!({ "username": username, "password": password }),
                ),
                address,
                user_agent,
            ),
        )
        .await
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

    async fn current_user_id(app: &Router, token: &str) -> String {
        let (status, session) = send(app, get_with_auth("/auth/session", &bearer(token))).await;
        assert_eq!(status, StatusCode::OK, "session failed: {session:#}");
        session["userId"].as_str().expect("user id").to_owned()
    }

    async fn group_id_named(app: &Router, admin: &str, name: &str) -> String {
        let (status, groups) = send(app, get_with_auth("/groups", &bearer(admin))).await;
        assert_eq!(status, StatusCode::OK, "group list failed: {groups:#}");
        groups
            .as_array()
            .expect("groups")
            .iter()
            .find(|group| group["name"] == name)
            .and_then(|group| group["id"].as_str())
            .expect("named group")
            .to_owned()
    }

    async fn create_user_in_group(
        app: &Router,
        admin: &str,
        username: &str,
        group_id: &str,
    ) -> (String, String) {
        let (status, user) = send(
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
        assert_eq!(status, StatusCode::CREATED, "user create failed: {user:#}");

        let (status, tokens) = send(
            app,
            post_json(
                "/auth/login",
                serde_json::json!({ "username": username, "password": "a-good-password" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "login failed: {tokens:#}");
        (
            user["id"].as_str().expect("user id").to_owned(),
            tokens["accessToken"]
                .as_str()
                .expect("access token")
                .to_owned(),
        )
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
        // globally, so the first admin's claims must contain all six with no
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
                "connectors.manage",
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
            with_client_context(
                post_json(
                    "/auth/refresh",
                    serde_json::json!({ "refreshToken": refresh }),
                ),
                [198, 51, 100, 23],
                "RotatedClient/1.0",
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
        assert_eq!(session["permissions"].as_array().expect("array").len(), 6);
        let user_id = session["userId"].as_str().expect("user id");
        let (status, sessions) = send(
            &app.router,
            get_with_auth(&format!("/users/{user_id}/sessions"), &bearer(&new_access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{sessions:#}");
        assert_eq!(sessions.as_array().expect("sessions").len(), 1);
        assert_eq!(sessions[0]["isCurrent"], true);
        assert_eq!(sessions[0]["userAgent"], "RotatedClient/1.0");
        assert_eq!(sessions[0]["ipAddress"], "198.51.100.23");

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
    async fn a_user_can_recognise_and_revoke_their_own_sessions_without_manage() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let ordinary =
            user_with_grants(&app.router, &admin, "session-owner", serde_json::json!([])).await;
        let user_id = current_user_id(&app.router, &ordinary).await;
        let admin_id = current_user_id(&app.router, &admin).await;

        let (status, second) = login_from(
            &app.router,
            "session-owner",
            "a-good-password",
            [198, 51, 100, 24],
            "ExampleBrowser/1.0 ExampleOS",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{second:#}");
        let second_access = second["accessToken"].as_str().expect("access token");

        let (status, sessions) = send(
            &app.router,
            get_with_auth(
                &format!("/users/{user_id}/sessions"),
                &bearer(second_access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{sessions:#}");
        let sessions = sessions.as_array().expect("sessions");
        assert_eq!(sessions.len(), 2);
        let current = sessions
            .iter()
            .find(|session| session["isCurrent"] == true)
            .expect("current session");
        assert_eq!(current["userAgent"], "ExampleBrowser/1.0 ExampleOS");
        assert_eq!(current["ipAddress"], "198.51.100.24");

        let other_id = sessions
            .iter()
            .find(|session| session["isCurrent"] == false)
            .and_then(|session| session["id"].as_str())
            .expect("other session");
        let (status, body) = send(
            &app.router,
            delete_auth(
                &format!("/users/{user_id}/sessions/{other_id}"),
                second_access,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body:#}");

        let (status, remaining) = send(
            &app.router,
            get_with_auth(
                &format!("/users/{user_id}/sessions"),
                &bearer(second_access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(remaining.as_array().expect("sessions").len(), 1);
        assert_eq!(remaining[0]["isCurrent"], true);

        let (status, _) = send(
            &app.router,
            get_with_auth(
                &format!("/users/{admin_id}/sessions"),
                &bearer(second_access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn an_admin_can_manage_another_users_sessions_and_revoke_them_all() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let ordinary = user_with_grants(
            &app.router,
            &admin,
            "managed-sessions",
            serde_json::json!([]),
        )
        .await;
        let user_id = current_user_id(&app.router, &ordinary).await;
        let (status, second) = login_from(
            &app.router,
            "managed-sessions",
            "a-good-password",
            [198, 51, 100, 25],
            "ManagedClient/2.0",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{second:#}");
        let refresh = second["refreshToken"]
            .as_str()
            .expect("refresh token")
            .to_owned();

        let (status, sessions) = send(
            &app.router,
            get_with_auth(&format!("/users/{user_id}/sessions"), &bearer(&admin)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{sessions:#}");
        assert_eq!(sessions.as_array().expect("sessions").len(), 2);
        assert!(sessions
            .as_array()
            .expect("sessions")
            .iter()
            .all(|session| session["isCurrent"] == false));

        let (status, body) = send(
            &app.router,
            delete_auth(&format!("/users/{user_id}/sessions"), &admin),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body:#}");

        let (status, sessions) = send(
            &app.router,
            get_with_auth(&format!("/users/{user_id}/sessions"), &bearer(&admin)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(sessions.as_array().expect("sessions").is_empty());

        let (status, _) = send(
            &app.router,
            post_json(
                "/auth/refresh",
                serde_json::json!({ "refreshToken": refresh }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn self_revoke_all_includes_the_callers_current_session() {
        let app = test_app().await;
        let (access, first_refresh) = setup_and_login(&app.router).await;
        let user_id = current_user_id(&app.router, &access).await;
        let (status, second) = login_from(
            &app.router,
            "admin",
            "a-good-password",
            [198, 51, 100, 26],
            "SecondDevice/1.0",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{second:#}");
        let second_access = second["accessToken"].as_str().expect("access token");
        let second_refresh = second["refreshToken"].as_str().expect("refresh token");

        let (status, body) = send(
            &app.router,
            delete_auth(&format!("/users/{user_id}/sessions"), second_access),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body:#}");

        for refresh in [&first_refresh, second_refresh] {
            let (status, _) = send(
                &app.router,
                post_json(
                    "/auth/refresh",
                    serde_json::json!({ "refreshToken": refresh }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn login_failures_are_limited_per_peer_and_success_resets_the_window() {
        let app = test_app().await;
        let (status, _) = send(&app.router, post_json("/setup", setup_body())).await;
        assert_eq!(status, StatusCode::OK);

        let limited_peer = [198, 51, 100, 40];
        for _ in 0..auth::rate_limit::LOGIN_FAILURE_LIMIT {
            let (status, body) = login_from(
                &app.router,
                "admin",
                "not-the-password",
                limited_peer,
                "RateLimitTest/1.0",
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{body:#}");
            assert_eq!(body["error"], "invalid credentials");
        }

        let response = send_response(
            &app.router,
            with_client_context(
                post_json(
                    "/auth/login",
                    serde_json::json!({
                        "username": "admin",
                        "password": "a-good-password",
                    }),
                ),
                limited_peer,
                "RateLimitTest/1.0",
            ),
        )
        .await;
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        let retry_after = response
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<u64>().ok())
            .expect("numeric Retry-After header");
        assert!((1..=15 * 60).contains(&retry_after));

        // Another direct peer is independent and can still sign in.
        let (status, body) = login_from(
            &app.router,
            "admin",
            "a-good-password",
            [198, 51, 100, 41],
            "FreshPeer/1.0",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:#}");

        // A success below the threshold removes earlier failures. Ten new
        // failures are allowed before the following request is limited.
        let reset_peer = [198, 51, 100, 42];
        for _ in 0..3 {
            let (status, _) = login_from(
                &app.router,
                "admin",
                "not-the-password",
                reset_peer,
                "ResetPeer/1.0",
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, body) = login_from(
            &app.router,
            "admin",
            "a-good-password",
            reset_peer,
            "ResetPeer/1.0",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:#}");
        for _ in 0..auth::rate_limit::LOGIN_FAILURE_LIMIT {
            let (status, _) = login_from(
                &app.router,
                "admin",
                "not-the-password",
                reset_peer,
                "ResetPeer/1.0",
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }
        let (status, _) = login_from(
            &app.router,
            "admin",
            "a-good-password",
            reset_peer,
            "ResetPeer/1.0",
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    }

    /* ---------------------------------------------------------------- */
    /* Connector types and instances                                     */
    /* ---------------------------------------------------------------- */

    /// Delegates every established capability to DebugConnector while using
    /// the trait's discovery defaults. This lets the HTTP unsupported branch
    /// be proven without adding a fake production registry type solely for a
    /// negative test.
    struct NonDiscoverableConnector(DebugConnector);

    #[async_trait::async_trait]
    impl Connector for NonDiscoverableConnector {
        async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
            self.0.status().await
        }

        async fn actions(&self) -> Vec<ConnectorAction> {
            self.0.actions().await
        }

        async fn execute_action(
            &self,
            action_id: &str,
            target_id: Option<&str>,
            params: Value,
        ) -> Result<ActionResult, ConnectorError> {
            self.0.execute_action(action_id, target_id, params).await
        }

        fn config_schema(&self) -> Value {
            self.0.config_schema()
        }

        fn metadata(&self) -> ConnectorMetadata {
            self.0.metadata()
        }

        fn display_fields(&self) -> Vec<DisplayField> {
            self.0.display_fields()
        }

        fn data_points(&self) -> Vec<DataPointDescriptor> {
            self.0.data_points()
        }

        fn default_layout(&self) -> WidgetLayout {
            self.0.default_layout()
        }
    }

    /// Creates a debug instance through the real endpoint and returns its id.
    async fn create_debug_instance(app: &Router, token: &str, name: &str) -> String {
        let (status, body) = send(
            app,
            post_json_auth(
                "/connector-instances",
                token,
                serde_json::json!({
                    "connectorType": "debug",
                    "name": name,
                    "config": {},
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "create failed: {body:#}");
        body["id"].as_str().expect("id").to_owned()
    }

    #[tokio::test]
    async fn the_type_catalog_lists_every_registered_type_with_its_schema() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (status, body) = send(
            &app.router,
            get_with_auth("/connector-types", &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let types = body.as_array().expect("array");
        assert_eq!(
            types.len(),
            5,
            "the catalog should contain Debug, Docker, TrueNAS, Pi-hole, and UniFi Network"
        );
        assert!(types.iter().all(|entry| {
            entry["typeId"] != "docker-host" && entry["typeId"] != "docker-container"
        }));
        let by_id = |type_id: &str| {
            types
                .iter()
                .find(|entry| entry["typeId"] == type_id)
                .unwrap_or_else(|| panic!("{type_id} is not in the catalog: {body:#}"))
        };

        // Every entry, whatever it is, has to be renderable: a name, and a
        // schema the add-connector form can actually be generated from.
        for entry in types {
            assert!(entry["displayName"]
                .as_str()
                .is_some_and(|name| !name.is_empty()));
            assert_eq!(entry["configSchema"]["type"], "object");
            assert!(entry["configSchema"]["properties"].is_object());
        }

        let debug = by_id("debug");
        assert_eq!(debug["discoverableType"], "debug");
        assert_eq!(debug["discoveryTargetField"], serde_json::Value::Null);
        let variants = debug["setupGuide"]["variants"]
            .as_array()
            .expect("debug setup-guide variants");
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0]["id"], "simple");
        assert_eq!(variants[0]["toggles"], serde_json::json!([]));
        assert_eq!(variants[1]["id"], "configurable");
        assert_eq!(variants[1]["toggles"].as_array().map(Vec::len), Some(2));
        assert!(variants[1]["template"]
            .as_str()
            .is_some_and(|template| template.contains("{{label}}")));

        // The Docker connector is catalogued **without a daemon anywhere in
        // this test**, which is the point: the type list and its form are
        // type-level data, so someone can open the add-connector dialog and
        // read what Docker needs before they have a working endpoint.
        let docker = by_id("docker");
        assert_eq!(docker["displayName"], "Docker");
        assert_eq!(docker["icon"], "brand:docker");
        assert!(docker["configSchema"]["properties"]["dockerHost"].is_object());
        assert!(docker["configSchema"]["properties"]
            .get("containerName")
            .is_none());
        assert_eq!(
            docker["configSchema"]["properties"]["dockerHost"]["default"],
            "unix:///var/run/docker.sock"
        );
        let variants = docker["setupGuide"]["variants"]
            .as_array()
            .expect("Docker setup-guide variants");
        assert_eq!(variants.len(), 2);
        assert_eq!(variants[0]["id"], "socket");
        assert_eq!(variants[1]["id"], "proxy");
        assert!(variants[1]["toggles"]
            .as_array()
            .is_some_and(|toggles| toggles
                .iter()
                .any(|toggle| toggle["envVar"] == "CONTAINERS")));
        assert_eq!(docker["discoverableType"], serde_json::Value::Null);
        assert_eq!(docker["discoveryTargetField"], serde_json::Value::Null);

        let truenas = by_id("truenas");
        assert_eq!(truenas["displayName"], "TrueNAS");
        assert_eq!(truenas["icon"], "brand:truenas");
        assert_eq!(
            truenas["configSchema"]["properties"]["apiKey"]["x-loom-sensitive"],
            true
        );
        assert!(truenas["configSchema"]["required"]
            .as_array()
            .is_some_and(|required| required.iter().any(|field| field == "username")));
        let variants = truenas["setupGuide"]["variants"]
            .as_array()
            .expect("TrueNAS should publish setup help");
        assert_eq!(variants.len(), 1);
        assert_eq!(variants[0]["id"], "api-key");
        assert_eq!(
            variants[0]["capabilityRequirements"]
                .as_array()
                .expect("TrueNAS guide should map capabilities")
                .len(),
            6
        );
        assert_eq!(truenas["discoverableType"], serde_json::Value::Null);

        let pihole = by_id("pihole");
        assert_eq!(pihole["displayName"], "Pi-hole");
        assert_eq!(pihole["icon"], "brand:pihole");
        assert_eq!(
            pihole["configSchema"]["properties"]["password"]["x-loom-sensitive"],
            true
        );
        assert_eq!(
            pihole["configSchema"]["properties"]["allowInsecureCert"]["default"],
            false
        );
        assert_eq!(
            pihole["setupGuide"]["variants"][0]["id"],
            "application-password"
        );
        assert_eq!(pihole["discoverableType"], serde_json::Value::Null);

        let unifi = by_id("unifi-network");
        assert_eq!(unifi["displayName"], "UniFi Network");
        assert_eq!(unifi["icon"], "brand:unifi");
        assert_eq!(
            unifi["configSchema"]["properties"]["apiKey"]["x-loom-sensitive"],
            true
        );
        assert_eq!(
            unifi["configSchema"]["properties"]["site"]["default"],
            "default"
        );
        assert_eq!(unifi["setupGuide"], serde_json::Value::Null);
        assert_eq!(unifi["discoverableType"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn discovery_is_instance_scoped_and_reports_unsupported_instances() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &access, "Discovery source").await;

        let (status, detail) = send(
            &app.router,
            get_with_auth(&format!("/connector-instances/{id}"), &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["discoverableType"], "debug");

        let (status, discovery) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/discover"),
                &access,
                serde_json::Value::Null,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "discovery failed: {discovery:#}");
        assert_eq!(discovery["discoveryTargetField"], serde_json::Value::Null);
        let resources = discovery["resources"].as_array().expect("resource array");
        assert_eq!(resources.len(), 3);
        assert!(resources.iter().all(|resource| {
            resource["targetConnectorType"] == "debug"
                && resource["suggestedName"]
                    .as_str()
                    .is_some_and(|name| name.starts_with("Discovered Debug Fixture"))
        }));

        // Keep the durable row and replace only its live implementation with a
        // connector that uses the trait's opt-out defaults.
        let uuid = uuid::Uuid::parse_str(&id).expect("instance uuid");
        app.connectors
            .insert(
                uuid,
                Arc::new(NonDiscoverableConnector(DebugConnector::default())),
            )
            .await;

        let (status, body) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/discover"),
                &access,
                serde_json::Value::Null,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        assert!(body["error"]
            .as_str()
            .is_some_and(|message| message.contains("not supported")));
    }

    #[tokio::test]
    async fn sub_targets_are_live_read_only_instance_metadata() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &access, "Target source").await;

        let (status, targets) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/sub-targets"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{targets:#}");
        assert_eq!(
            targets,
            serde_json::json!([
                // `kind` is the connector's own word for what sort of thing a
                // target is. A connector with nothing to distinguish leaves it
                // at the default, which is what a client sees here.
                { "id": "fixture-a", "label": "fixture-a", "kind": "target" },
                { "id": "fixture-b", "label": "fixture-b", "kind": "target" },
            ])
        );

        // Replace only the live implementation with one using the trait's
        // sub-target opt-out defaults. The durable row remains addressable.
        let uuid = uuid::Uuid::parse_str(&id).expect("instance uuid");
        app.connectors
            .insert(
                uuid,
                Arc::new(NonDiscoverableConnector(DebugConnector::default())),
            )
            .await;
        let (status, body) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/sub-targets"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        assert!(body["error"]
            .as_str()
            .is_some_and(|message| message.contains("does not support")));
    }

    #[tokio::test]
    async fn resource_kinds_and_rows_are_served_from_the_live_connector() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &access, "Browsable").await;

        let (status, kinds) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/resource-kinds"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{kinds:#}");
        let kinds = kinds.as_array().expect("a list of kinds").clone();
        assert_eq!(
            kinds
                .iter()
                .map(|kind| kind["kind"].as_str().expect("kind id"))
                .collect::<Vec<_>>(),
            // The first two are the fixture's own; the third is the
            // platform's, offered because this connector supports update
            // checking and served from Loom's action log rather than from the
            // connector.
            vec!["widgets", "gadgets", "recentlyUpdated"]
        );
        assert_eq!(kinds[0]["columns"][1]["valueType"], "bytes");
        assert_eq!(kinds[0]["rowActions"][0]["id"], "recycle");
        assert_eq!(kinds[0]["kindActions"][0]["id"], "cleanupAll");
        assert_eq!(
            kinds[1]["kindActions"],
            serde_json::json!([]),
            "the second fixture kind deliberately has no kind actions"
        );

        let (status, rows) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/resources/widgets"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{rows:#}");
        let rows = rows.as_array().expect("a list of rows");
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0]["id"], "widget-1");
        assert_eq!(rows[0]["fields"]["name"], "alpha-widget");
        assert_eq!(rows[0]["fields"]["size"], 1_048_576);

        // `targetId` is passed through rather than rejected: a browsable kind
        // may legitimately be scoped to one sub-target.
        let (status, scoped) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/resources/gadgets?targetId=fixture-a"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{scoped:#}");
        assert_eq!(scoped.as_array().expect("a list of rows").len(), 2);

        // A kind this connector never declared is a bad request, not an empty
        // table — the whole reason the route validates against the live
        // descriptors instead of calling through.
        let (status, body) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/resources/nonexistent-kind"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        assert!(body["error"]
            .as_str()
            .is_some_and(|message| message.contains("nonexistent-kind")));

        // A connector using the trait's defaults browses nothing, and asking
        // for one of the fixture's kinds through it is refused for exactly the
        // same reason.
        let uuid = uuid::Uuid::parse_str(&id).expect("instance uuid");
        app.connectors
            .insert(
                uuid,
                Arc::new(NonDiscoverableConnector(DebugConnector::default())),
            )
            .await;
        let (status, kinds) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/resource-kinds"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{kinds:#}");
        // A connector using the trait's defaults browses nothing and cannot
        // check for updates, so it gets no platform kind either.
        assert_eq!(kinds, serde_json::json!([]));

        let (status, body) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/resources/widgets"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
    }

    #[tokio::test]
    async fn resource_actions_run_through_the_ordinary_action_endpoint() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &access, "Browsable").await;

        // A row action, carrying the row it acts on in `params` — the whole
        // targeting convention, exercised end to end.
        let (status, body) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/recycle"),
                &access,
                serde_json::json!({ "resourceId": "widget-2" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:#}");
        assert_eq!(body["success"], true);
        assert_eq!(body["payload"]["recycled"], "widget-2");

        // The same action without a row is the connector's own refusal, not a
        // route-level one, and arrives as a 400 with the reason intact.
        let (status, body) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/recycle"),
                &access,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        assert!(body["error"]
            .as_str()
            .is_some_and(|message| message.contains("resourceId")));

        // A kind action addresses no row and needs no params.
        let (status, body) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/cleanupAll"),
                &access,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:#}");
        assert_eq!(body["success"], true);

        // Validation spans both universes but is not a free pass: an id
        // advertised in neither is still refused.
        let (status, body) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/not-an-action"),
                &access,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body:#}");
    }

    /// Browsing is a connector *action*, so it sits behind the same permission
    /// as any other action — no new tier, no exemption for the ones that
    /// happen to be advertised inside a table.
    #[tokio::test]
    async fn resource_browsing_uses_the_established_permission_tiers() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &admin, "Browsable").await;

        // Can look at connectors, cannot control them.
        let viewer = user_with_grants(
            &app.router,
            &admin,
            "viewer",
            serde_json::json!([{
                "key": "connectors.view",
                "resourceType": null,
                "resourceId": null,
            }]),
        )
        .await;

        for path in [
            format!("/connector-instances/{id}/resource-kinds"),
            format!("/connector-instances/{id}/resources/widgets"),
        ] {
            let (status, body) = send(&app.router, get_with_auth(&path, &bearer(&viewer))).await;
            assert_eq!(status, StatusCode::OK, "{path}: {body:#}");
        }

        for action in ["recycle", "cleanupAll"] {
            let (status, _) = send(
                &app.router,
                post_json_auth(
                    &format!("/connector-instances/{id}/actions/{action}"),
                    &viewer,
                    serde_json::json!({ "resourceId": "widget-1" }),
                ),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "{action} must need connectors.control"
            );
        }

        // And no grant at all cannot even read the tables.
        let nobody = user_with_grants(&app.router, &admin, "nobody", serde_json::json!([])).await;
        let (status, _) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/resource-kinds"),
                &bearer(&nobody),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // Anonymously, both browsing routes are 401 like every other one.
        for path in [
            format!("/connector-instances/{id}/resource-kinds"),
            format!("/connector-instances/{id}/resources/widgets"),
        ] {
            let (status, _) = send(&app.router, get(&path)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}");
        }
    }

    #[tokio::test]
    async fn type_scoped_discovery_uses_a_candidate_without_persisting_it() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM connector_instances")
            .fetch_one(&app.pool)
            .await
            .expect("count before discovery");

        let (status, discovery) = send(
            &app.router,
            post_json_auth(
                "/connector-types/debug/discover",
                &access,
                serde_json::json!({ "label": "candidate" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "discovery failed: {discovery:#}");
        assert_eq!(discovery["discoveryTargetField"], serde_json::Value::Null);
        assert_eq!(
            discovery["resources"]
                .as_array()
                .expect("resource array")
                .len(),
            3
        );

        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM connector_instances")
            .fetch_one(&app.pool)
            .await
            .expect("count after discovery");
        assert_eq!(after, before, "candidate discovery must never insert a row");

        let (status, body) = send(
            &app.router,
            post_json_auth(
                "/connector-types/debug/discover",
                &access,
                serde_json::json!({ "baseLoad": 101 }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        assert!(body["connectorError"].is_object());
    }

    #[tokio::test]
    async fn type_scoped_connection_test_is_ephemeral_and_reports_debug_capabilities() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM connector_instances")
            .fetch_one(&app.pool)
            .await
            .expect("count before connection tests");
        let live_before = app.connectors.len().await;

        let (status, healthy) = send(
            &app.router,
            post_json_auth(
                "/connector-types/debug/test-connection",
                &access,
                serde_json::json!({ "label": "candidate", "enabled": false }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "connection test failed: {healthy:#}"
        );
        assert_eq!(healthy["reachable"], true);
        assert_eq!(healthy["message"], serde_json::Value::Null);
        let capabilities = healthy["capabilities"].as_array().expect("capabilities");
        assert_eq!(capabilities.len(), 4);
        assert!(capabilities.iter().any(|capability| {
            capability["key"] == "read-status" && capability["available"] == true
        }));
        assert!(capabilities.iter().any(|capability| {
            capability["key"] == "perform-actions" && capability["available"] == false
        }));
        assert!(capabilities.iter().any(|capability| {
            capability["key"] == "view-gadgets" && capability["available"] == true
        }));

        let (status, failing) = send(
            &app.router,
            post_json_auth(
                "/connector-types/debug/test-connection",
                &access,
                serde_json::json!({ "failMode": "unreachable" }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "connection test failed: {failing:#}"
        );
        assert_eq!(failing["reachable"], false);
        assert!(failing["capabilities"]
            .as_array()
            .is_some_and(|items| items.iter().all(|item| item["available"] == false)));
        assert!(failing["message"]
            .as_str()
            .is_some_and(|message| message.contains("simulated outage")));

        // Docker's ordinary factory reads CONTAINERS before returning, while
        // this endpoint must be able to report that permission as unavailable.
        // The dedicated setup-test constructor therefore performs no I/O and
        // lets test_connection own the reachability result.
        let (status, docker) = send(
            &app.router,
            post_json_auth(
                "/connector-types/docker/test-connection",
                &access,
                serde_json::json!({ "dockerHost": "tcp://127.0.0.1:1" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "Docker check failed: {docker:#}");
        assert_eq!(docker["reachable"], false);
        assert!(docker["message"]
            .as_str()
            .is_some_and(|message| message.contains("127.0.0.1:1")));

        // Pi-hole likewise authenticates during normal construction. Its
        // setup-test factory defers that request so transport/auth failures are
        // returned in the standard connection-test result instead of as a 400.
        let (status, pihole) = send(
            &app.router,
            post_json_auth(
                "/connector-types/pihole/test-connection",
                &access,
                serde_json::json!({
                    "baseUrl": "http://127.0.0.1:1",
                    "password": "not-a-real-password"
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "Pi-hole check failed: {pihole:#}");
        assert_eq!(pihole["reachable"], false);
        assert!(pihole["message"]
            .as_str()
            .is_some_and(|message| message.contains("127.0.0.1:1")));

        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM connector_instances")
            .fetch_one(&app.pool)
            .await
            .expect("count after connection tests");
        assert_eq!(after, before, "candidate checks must never insert a row");
        assert_eq!(
            app.connectors.len().await,
            live_before,
            "candidate checks must never enter the runtime map"
        );

        let (status, invalid) = send(
            &app.router,
            post_json_auth(
                "/connector-types/debug/test-connection",
                &access,
                serde_json::json!({ "baseLoad": 101 }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid:#}");
        assert!(invalid["connectorError"].is_object());
    }

    #[tokio::test]
    async fn docker_type_discovery_is_unsupported_and_never_persists_a_candidate() {
        let app = test_app().await;
        let candidate = serde_json::json!({
            "dockerHost": loom_connector_docker::DEFAULT_DOCKER_HOST,
        });

        // Contributors are not required to run Docker. GitHub-hosted runners
        // do, so this path is exercised in CI; locally the skip remains loud
        // under `--nocapture` rather than turning workspace tests red.
        if let Err(error) = app.connectors.build("docker", candidate.clone()).await {
            eprintln!("SKIPPING docker type discovery endpoint test: {error:?}");
            return;
        }

        let (access, _) = setup_and_login(&app.router).await;
        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM connector_instances")
            .fetch_one(&app.pool)
            .await
            .expect("count before Docker discovery");

        let (status, discovery) = send(
            &app.router,
            post_json_auth("/connector-types/docker/discover", &access, candidate),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{discovery:#}");
        assert!(discovery["error"]
            .as_str()
            .is_some_and(|message| message.contains("not supported")));

        let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM connector_instances")
            .fetch_one(&app.pool)
            .await
            .expect("count after Docker discovery");
        assert_eq!(after, before, "candidate discovery must never insert a row");
        assert_eq!(
            app.connectors.len().await,
            0,
            "candidate must not be retained"
        );
    }

    #[tokio::test]
    async fn an_instance_can_be_created_read_updated_and_deleted() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        // Nothing to begin with: no connector is implicit any more.
        let (status, body) = send(
            &app.router,
            get_with_auth("/connector-instances", &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.as_array().expect("array").len(), 0);

        let (status, created) = send(
            &app.router,
            post_json_auth(
                "/connector-instances",
                &access,
                serde_json::json!({
                    "connectorType": "debug",
                    "name": "Fixture",
                    "config": { "baseLoad": 10 },
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "create failed: {created:#}");
        let id = created["id"].as_str().expect("id").to_owned();
        assert_eq!(created["name"], "Fixture");
        assert_eq!(created["connectorType"], "debug");
        assert_eq!(created["metadata"]["id"], "debug");
        assert_eq!(created["metadata"]["minSize"], serde_json::json!([2, 2]));
        assert_eq!(created["metadata"]["icon"], "lucide:bug");
        // A fresh instance inherits its type's icon; the override only exists
        // once someone sets one.
        assert_eq!(created["iconOverride"], serde_json::Value::Null);
        assert_eq!(created["tags"], serde_json::json!([]));
        assert_eq!(
            created["status"],
            serde_json::Value::Null,
            "creation must not wait for a potentially expensive remote poll"
        );
        assert!(!created["displayFields"]
            .as_array()
            .expect("array")
            .is_empty());
        assert_eq!(created["dataPoints"].as_array().expect("array").len(), 9);
        assert!(!created["defaultLayout"]["bindings"]
            .as_array()
            .expect("array")
            .is_empty());
        assert_eq!(created["discoverableType"], "debug");
        assert_eq!(created["supportsSubTargets"], true);

        // The poller fills status after the response rather than extending the
        // create request until every remote sub-target has been sampled.
        app.connectors.poll_once().await;

        // Detail carries what a placement UI needs.
        let (status, detail) = send(
            &app.router,
            get_with_auth(&format!("/connector-instances/{id}"), &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(detail["config"]["baseLoad"], 10);
        let action_ids: Vec<&str> = detail["actions"]
            .as_array()
            .expect("actions")
            .iter()
            .map(|action| action["id"].as_str().expect("id"))
            .collect();
        assert!(action_ids.contains(&"ping") && action_ids.contains(&"set-enabled"));

        // Update: both fields, and the live connector is rebuilt from the new
        // configuration rather than left as it was.
        let (status, updated) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{id}"),
                &access,
                serde_json::json!({ "name": "Renamed", "config": { "label": "after-update" } }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "update failed: {updated:#}");
        assert_eq!(updated["name"], "Renamed");
        assert_eq!(updated["config"]["label"], "after-update");
        assert!(updated["displayFields"]
            .as_array()
            .expect("array")
            .iter()
            .any(|field| field["value"] == "after-update"));

        let (status, _) = send(
            &app.router,
            delete_auth(&format!("/connector-instances/{id}"), &access),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, _) = send(
            &app.router,
            get_with_auth(&format!("/connector-instances/{id}"), &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // And the live runtime entry went with it.
        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/ping"),
                &access,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn sensitive_connector_config_is_encrypted_redacted_and_merged() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (status, created) = send(
            &app.router,
            post_json_auth(
                "/connector-instances",
                &access,
                serde_json::json!({
                    "connectorType": "debug",
                    "name": "Secret fixture",
                    "config": {
                        "label": "before",
                        "apiToken": "original-secret"
                    }
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "create failed: {created:#}");
        let id = created["id"].as_str().expect("id");
        assert!(created["config"].get("apiToken").is_none());
        assert_eq!(
            created["sensitiveFieldsSet"],
            serde_json::json!(["apiToken"])
        );

        let raw_before: String =
            sqlx::query_scalar("SELECT config FROM connector_instances WHERE id = ?")
                .bind(id)
                .fetch_one(&app.pool)
                .await
                .expect("stored config");
        assert!(!raw_before.contains("original-secret"));
        let stored_before: Value = serde_json::from_str(&raw_before).expect("stored JSON");
        let ciphertext_before = stored_before["apiToken"]
            .as_str()
            .expect("encrypted token")
            .to_owned();
        assert_eq!(
            connectors::config_secrets::decrypt_value(
                &ciphertext_before,
                &app.state.config_encryption_key,
            )
            .expect("token decrypts"),
            "original-secret"
        );
        let reloaded = connectors::ConnectorRuntime::load(
            &app.pool,
            connectors::builtin_registry(),
            &app.state.config_encryption_key,
        )
        .await
        .expect("runtime reload decrypts stored config");
        assert_eq!(reloaded.len().await, 1);

        let (status, listed) = send(
            &app.router,
            get_with_auth("/connector-instances", &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            listed[0]["sensitiveFieldsSet"],
            serde_json::json!(["apiToken"])
        );

        let (status, updated) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{id}"),
                &access,
                serde_json::json!({ "config": { "label": "after" } }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "update failed: {updated:#}");
        assert_eq!(updated["config"]["label"], "after");
        assert!(updated["config"].get("apiToken").is_none());

        let raw_after_omission: String =
            sqlx::query_scalar("SELECT config FROM connector_instances WHERE id = ?")
                .bind(id)
                .fetch_one(&app.pool)
                .await
                .expect("stored config after omission");
        let stored_after_omission: Value =
            serde_json::from_str(&raw_after_omission).expect("stored JSON");
        assert_eq!(stored_after_omission["apiToken"], ciphertext_before);

        let (status, replaced) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{id}"),
                &access,
                serde_json::json!({
                    "config": { "label": "after", "apiToken": "replacement-secret" }
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "replacement failed: {replaced:#}");
        assert!(replaced["config"].get("apiToken").is_none());
        assert_eq!(
            replaced["sensitiveFieldsSet"],
            serde_json::json!(["apiToken"])
        );

        let raw_after_replacement: String =
            sqlx::query_scalar("SELECT config FROM connector_instances WHERE id = ?")
                .bind(id)
                .fetch_one(&app.pool)
                .await
                .expect("stored config after replacement");
        let stored_after_replacement: Value =
            serde_json::from_str(&raw_after_replacement).expect("stored JSON");
        let replacement_blob = stored_after_replacement["apiToken"]
            .as_str()
            .expect("replacement ciphertext");
        assert_ne!(replacement_blob, ciphertext_before);
        assert_eq!(
            connectors::config_secrets::decrypt_value(
                replacement_blob,
                &app.state.config_encryption_key,
            )
            .expect("replacement token decrypts"),
            "replacement-secret"
        );
    }

    #[tokio::test]
    async fn connector_tags_are_replace_all_listed_and_cascade_with_the_instance() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let first = create_debug_instance(&app.router, &access, "First").await;
        let second = create_debug_instance(&app.router, &access, "Second").await;

        let (status, updated) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{first}"),
                &access,
                serde_json::json!({ "tags": [" test ", "production", "test"] }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "tagging failed: {updated:#}");
        assert_eq!(updated["tags"], serde_json::json!(["production", "test"]));

        let (status, second_updated) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{second}"),
                &access,
                serde_json::json!({ "tags": ["lab", "test"] }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "tagging failed: {second_updated:#}");

        let (status, tags) = send(
            &app.router,
            get_with_auth("/connector-instances/tags", &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(tags, serde_json::json!(["lab", "production", "test"]));

        let (status, replaced) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{first}"),
                &access,
                serde_json::json!({ "tags": ["staging"] }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "replacement failed: {replaced:#}");
        assert_eq!(replaced["tags"], serde_json::json!(["staging"]));

        let (status, renamed) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{first}"),
                &access,
                serde_json::json!({ "name": "First renamed" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "rename failed: {renamed:#}");
        assert_eq!(renamed["tags"], serde_json::json!(["staging"]));

        let (status, invalid) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{first}"),
                &access,
                serde_json::json!({ "tags": ["  "] }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "invalid tags accepted: {invalid:#}"
        );

        let (status, listed) = send(
            &app.router,
            get_with_auth("/connector-instances", &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let first_entry = listed
            .as_array()
            .expect("instances")
            .iter()
            .find(|entry| entry["id"] == first)
            .expect("first instance");
        assert_eq!(first_entry["tags"], serde_json::json!(["staging"]));

        let (status, _) = send(
            &app.router,
            delete_auth(&format!("/connector-instances/{first}"), &access),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM connector_instance_tags WHERE instance_id = ?",
        )
        .bind(&first)
        .fetch_one(&app.pool)
        .await
        .expect("count cascaded tags");
        assert_eq!(remaining, 0);
    }

    /// The factory is the validator: a configuration it refuses must never be
    /// written, or the row would be silently skipped at the next startup.
    #[tokio::test]
    async fn an_invalid_configuration_is_refused_and_nothing_is_stored() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (status, body) = send(
            &app.router,
            post_json_auth(
                "/connector-instances",
                &access,
                serde_json::json!({
                    "connectorType": "debug",
                    "name": "Bad",
                    "config": { "baseLoad": 900 },
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        // The connector's own objection, not a generic rejection.
        assert!(body["connectorError"]["invalidConfig"]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("baseLoad")));

        // An unknown key is caught too, rather than silently ignored.
        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/connector-instances",
                &access,
                serde_json::json!({
                    "connectorType": "debug",
                    "name": "Bad",
                    "config": { "notAField": 1 },
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // An unregistered type is a 400, not a 404: the instance was never the
        // thing that could not be found.
        let (status, body) = send(
            &app.router,
            post_json_auth(
                "/connector-instances",
                &access,
                serde_json::json!({
                    "connectorType": "not-a-type",
                    "name": "Bad",
                    "config": {},
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["error"]
            .as_str()
            .is_some_and(|error| error.contains("not-a-type")));

        let (_, list) = send(
            &app.router,
            get_with_auth("/connector-instances", &bearer(&access)),
        )
        .await;
        assert_eq!(list.as_array().expect("array").len(), 0);
    }

    /// `iconOverride` has three request states, and the one that is easy to get
    /// wrong is the difference between "leave it alone" and "clear it" — a flat
    /// `Option` collapses them and quietly makes a chosen icon permanent.
    #[tokio::test]
    async fn an_icon_override_can_be_set_left_alone_and_cleared() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &access, "Fixture").await;

        let (status, set) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{id}"),
                &access,
                serde_json::json!({ "iconOverride": "lucide:hard-drive" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "setting failed: {set:#}");
        assert_eq!(set["iconOverride"], "lucide:hard-drive");
        // The type's own icon is untouched: an override sits beside it rather
        // than replacing it, so "use default" has something to go back to.
        assert_eq!(set["metadata"]["icon"], "lucide:bug");

        // An unrelated PATCH must not disturb it.
        let (status, renamed) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{id}"),
                &access,
                serde_json::json!({ "name": "Renamed" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "rename failed: {renamed:#}");
        assert_eq!(renamed["iconOverride"], "lucide:hard-drive");

        // And it survives a round trip through the database, not just the
        // response this request happened to build.
        let (status, listed) = send(
            &app.router,
            get_with_auth("/connector-instances", &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(listed[0]["iconOverride"], "lucide:hard-drive");

        // Explicit null clears it.
        let (status, cleared) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{id}"),
                &access,
                serde_json::json!({ "iconOverride": null }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "clearing failed: {cleared:#}");
        assert_eq!(cleared["iconOverride"], serde_json::Value::Null);
    }

    /// The type picker draws an icon before any instance exists, so the catalog
    /// has to carry one.
    #[tokio::test]
    async fn the_type_catalog_carries_each_type_s_icon() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (status, types) = send(
            &app.router,
            get_with_auth("/connector-types", &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let debug = types
            .as_array()
            .expect("array")
            .iter()
            .find(|entry| entry["typeId"] == "debug")
            .expect("the debug type is always registered");
        assert_eq!(debug["icon"], "lucide:bug");
    }

    #[tokio::test]
    async fn an_update_with_an_invalid_configuration_leaves_the_instance_alone() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &access, "Fixture").await;

        let (status, _) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{id}"),
                &access,
                serde_json::json!({ "name": "Renamed", "config": { "baseLoad": 900 } }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let (_, detail) = send(
            &app.router,
            get_with_auth(&format!("/connector-instances/{id}"), &bearer(&access)),
        )
        .await;
        assert_eq!(detail["name"], "Fixture", "the rename must not have landed");
    }

    #[tokio::test]
    async fn actions_run_against_the_instance_that_was_named() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let first = create_debug_instance(&app.router, &access, "First").await;
        let second = create_debug_instance(&app.router, &access, "Second").await;

        let (status, body) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{first}/actions/set-label"),
                &access,
                serde_json::json!({ "label": "only-the-first" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "action failed: {body:#}");
        assert_eq!(body["success"], true);

        let (status, targeted) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{first}/actions/ping"),
                &access,
                serde_json::json!({ "targetId": "fixture-a", "params": {} }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "targeted action failed: {targeted:#}"
        );
        assert_eq!(targeted["success"], true);

        // Actions mutate the connector, while reads intentionally use the last
        // completed poll. Drive that boundary explicitly instead of relying on
        // wall-clock timing in the test.
        app.connectors.poll_once().await;

        // Instances are separate live connectors, not one shared fixture.
        let (_, first_detail) = send(
            &app.router,
            get_with_auth(&format!("/connector-instances/{first}"), &bearer(&access)),
        )
        .await;
        let (_, second_detail) = send(
            &app.router,
            get_with_auth(&format!("/connector-instances/{second}"), &bearer(&access)),
        )
        .await;
        assert_eq!(
            first_detail["status"]["details"][""]["label"],
            "only-the-first"
        );
        assert_eq!(
            second_detail["status"]["details"][""]["label"],
            "debug-fixture"
        );

        // Bad parameters are the connector's objection, reported as a 400.
        let (status, body) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{first}/actions/set-load"),
                &access,
                serde_json::json!({ "value": 900 }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["connectorError"]["invalidParams"].is_object());

        // An unknown action id is a 404 from the connector, not a 400.
        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{first}/actions/nope"),
                &access,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    /// Unauthenticated access must be 401, not 403: there is no identity yet.
    /// The overlay, end to end through the HTTP layer.
    ///
    /// A restart takes the service away and brings it back; a poll landing in
    /// that window reports a perfectly accurate outage that helps nobody. What
    /// a client needs instead is "Performing: Restart", and this checks it is
    /// actually there *while the action is running* — not merely that the
    /// marker type exists.
    #[tokio::test]
    async fn a_disruptive_action_is_reported_as_a_pending_operation_while_it_runs() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        // Slow enough to observe mid-flight, short enough not to pad the suite.
        let (status, created) = send(
            &app.router,
            post_json_auth(
                "/connector-instances",
                &access,
                serde_json::json!({
                    "connectorType": "debug",
                    "name": "Restartable",
                    "config": { "simulatedLatencyMs": 600 },
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created:#}");
        let id = created["id"].as_str().expect("id").to_owned();
        assert_eq!(
            created["pendingOperation"],
            serde_json::Value::Null,
            "nothing is running yet"
        );

        // `restart` and `recalibrate` are the fixture's disruptive actions;
        // `ping` is not. All are read from the connector's own descriptors,
        // never from a name this test or the route recognises.
        let actions = created["actions"].as_array().expect("actions");
        let disruptive: Vec<&str> = actions
            .iter()
            .filter(|action| action["isDisruptive"] == true && action["targetId"].is_null())
            .map(|action| action["id"].as_str().expect("id"))
            .collect();
        assert_eq!(disruptive, vec!["restart", "recalibrate"]);

        // Seed the underlying status explicitly; production's process poller
        // does this on its next tick.
        app.connectors.poll_once().await;

        // Subscribe before dispatch so the test can synchronize with the
        // operation marker instead of hoping a wall-clock poll lands inside a
        // short window. Detail responses call the fixture's latency-gated
        // `actions()` method, so repeated HTTP requests are not actually
        // sampled every 50 ms: each one takes roughly the same 600 ms as the
        // action and can miss the marker entirely on a loaded runner.
        let mut status_updates = app.connectors.subscribe_statuses();
        let router = app.router.clone();
        let token = access.clone();
        let restart_id = id.clone();
        let restart = tokio::spawn(async move {
            send(
                &router,
                post_json_auth(
                    &format!("/connector-instances/{restart_id}/actions/restart"),
                    &token,
                    serde_json::Value::Null,
                ),
            )
            .await
        });

        let instance_uuid = uuid::Uuid::parse_str(&id).expect("instance id");
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let update = status_updates.recv().await.expect("status update");
                if update.instance_id == instance_uuid
                    && update.snapshot.pending_operation.is_some()
                {
                    break;
                }
            }
        })
        .await
        .expect("the disruptive action must publish its pending marker");

        // The HTTP response captures the status snapshot before awaiting the
        // connector's latency-gated descriptors, so it remains a faithful
        // observation of the marker even if the action completes while the
        // rest of the response is being assembled.
        let (status, during) = send(
            &app.router,
            get_with_auth(&format!("/connector-instances/{id}"), &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            during["pendingOperation"]["actionLabel"], "Restart",
            "the overlay must be visible while the action runs: {during:#}"
        );
        assert!(during["pendingOperation"]["startedAt"]
            .as_str()
            .is_some_and(|started| !started.is_empty()));
        // The poll result underneath is untouched — the overlay adds context,
        // it does not rewrite what the connector said.
        assert!(during["status"]["health"].is_string());

        let (status, result) = restart.await.expect("the action task");
        assert_eq!(status, StatusCode::OK, "{result:#}");
        assert_eq!(result["success"], true);

        let (status, after) = send(
            &app.router,
            get_with_auth(&format!("/connector-instances/{id}"), &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            after["pendingOperation"],
            serde_json::Value::Null,
            "the marker must be cleared once the action returns: {after:#}"
        );
    }

    /// A non-disruptive action must not raise the overlay. An action wrongly
    /// marked disruptive would hide a real outage behind "Performing…", so the
    /// negative case is worth pinning too.
    #[tokio::test]
    async fn an_ordinary_action_raises_no_pending_operation() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &access, "Pingable").await;

        let (status, result) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/ping"),
                &access,
                serde_json::Value::Null,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{result:#}");

        let (status, after) = send(
            &app.router,
            get_with_auth(&format!("/connector-instances/{id}"), &bearer(&access)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(after["pendingOperation"], serde_json::Value::Null);
    }

    /// A Down instance whose connector names a network endpoint is explained,
    /// not merely reported.
    #[tokio::test]
    async fn a_down_instance_carries_a_network_diagnosis() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        // Loopback port 1 is reserved and nothing binds it, so the probe is
        // refused immediately — deterministic on any machine, no network.
        let (status, created) = send(
            &app.router,
            post_json_auth(
                "/connector-instances",
                &access,
                serde_json::json!({
                    "connectorType": "debug",
                    "name": "Offline",
                    "config": {
                        "simulatedHealth": "down",
                        "networkTarget": { "host": "127.0.0.1", "port": 1 },
                    },
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created:#}");

        let id = created["id"].as_str().expect("id");
        assert_eq!(created["status"], serde_json::Value::Null);
        app.connectors.poll_once().await;
        let (_, polled) = send(
            &app.router,
            get_with_auth(&format!("/connector-instances/{id}"), &bearer(&access)),
        )
        .await;
        assert_eq!(polled["status"]["health"], "down");
        let diagnosis = polled["diagnosis"]
            .as_str()
            .unwrap_or_else(|| panic!("a Down instance should carry a diagnosis: {polled:#}"));
        assert!(
            diagnosis.contains("unreachable on port `1`"),
            "the diagnosis should name the port that was tried: {diagnosis}"
        );
    }

    /// Reads an instance's action log through the real endpoint.
    async fn action_log(
        app: &TestApp,
        token: &str,
        id: &str,
        query: &str,
    ) -> Vec<serde_json::Value> {
        let (status, body) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/action-log{query}"),
                &bearer(token),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:#}");
        body.as_array().expect("a list of log entries").clone()
    }

    /// Reads the cross-instance audit log through the real endpoint.
    async fn global_audit_log(app: &TestApp, token: &str, query: &str) -> Vec<serde_json::Value> {
        let (status, body) = send(
            &app.router,
            get_with_auth(&format!("/audit-log{query}"), &bearer(token)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:#}");
        body.as_array().expect("a list of audit entries").clone()
    }

    #[tokio::test]
    async fn every_action_is_recorded_with_its_caller_params_and_outcome() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &access, "Audited").await;
        let admin_id = current_user_id(&app.router, &access).await;

        assert!(
            action_log(&app, &access, &id, "").await.is_empty(),
            "nothing has been invoked yet"
        );

        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/set-label"),
                &access,
                serde_json::json!({ "label": "audited" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let entries = action_log(&app, &access, &id, "").await;
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry["actionId"], "set-label");
        assert_eq!(entry["targetId"], serde_json::Value::Null);
        assert_eq!(entry["params"], serde_json::json!({ "label": "audited" }));
        assert_eq!(entry["invokedBy"]["id"], admin_id);
        // The username, not merely the id: a log nobody can read without a
        // second lookup is a log nobody reads.
        assert_eq!(entry["invokedBy"]["username"], "admin");
        assert_eq!(entry["success"], true);
        assert_eq!(entry["resultMessage"], "Simulated label updated.");
        assert!(entry["invokedAt"].is_string());
        assert!(entry["completedAt"].is_string());
        // No snapshot was declared for this action, and none was invented.
        assert_eq!(entry["snapshot"], serde_json::Value::Null);

        // A refusal is recorded as emphatically as a success. The fixture
        // rejects a bad parameter, and that is exactly the invocation someone
        // will later want to find.
        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/set-load"),
                &access,
                serde_json::json!({ "value": 900 }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let entries = action_log(&app, &access, &id, "").await;
        assert_eq!(entries.len(), 2, "newest first");
        assert_eq!(entries[0]["actionId"], "set-load");
        assert_eq!(entries[0]["success"], false);
        assert!(entries[0]["resultMessage"]
            .as_str()
            .is_some_and(|message| message.contains("between 0 and 100")));

        // A targeted action records which target it addressed.
        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/ping"),
                &access,
                serde_json::json!({ "targetId": "fixture-a", "params": {} }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            action_log(&app, &access, &id, "").await[0]["targetId"],
            "fixture-a"
        );
    }

    #[tokio::test]
    async fn a_snapshot_bearing_action_records_the_reading_it_was_about_to_destroy() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &access, "Snapshotted").await;

        // The snapshot comes from the poll cache, so there has to have been a
        // poll. Production's poller does this on its own tick.
        app.connectors.poll_once().await;
        let cached = app
            .connectors
            .cached_status(&uuid::Uuid::parse_str(&id).expect("uuid"))
            .await
            .and_then(|snapshot| snapshot.status)
            .expect("a cached reading");
        let load_before = cached
            .data_point_value("load")
            .cloned()
            .expect("the load reading");

        let (status, body) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/recalibrate"),
                &access,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:#}");

        let entry = action_log(&app, &access, &id, "").await.remove(0);
        assert_eq!(entry["actionId"], "recalibrate");
        assert_eq!(
            entry["snapshot"],
            serde_json::json!({ "load": load_before }),
            "the snapshot must hold the value from before the action ran"
        );
        // And the action really did overwrite it, which is what makes the
        // snapshot the only remaining record.
        assert_eq!(body["payload"]["previousLoad"], load_before);
    }

    #[tokio::test]
    async fn the_action_log_filters_by_action_and_target_and_caps_its_page() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &access, "Busy").await;

        for _ in 0..3 {
            let (status, _) = send(
                &app.router,
                post_json_auth(
                    &format!("/connector-instances/{id}/actions/ping"),
                    &access,
                    serde_json::json!({}),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }
        for target in ["fixture-a", "fixture-b"] {
            let (status, _) = send(
                &app.router,
                post_json_auth(
                    &format!("/connector-instances/{id}/actions/restart"),
                    &access,
                    serde_json::json!({ "targetId": target, "params": {} }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::OK);
        }

        assert_eq!(action_log(&app, &access, &id, "").await.len(), 5);
        assert_eq!(
            action_log(&app, &access, &id, "?actionId=ping").await.len(),
            3
        );
        let targeted = action_log(&app, &access, &id, "?targetId=fixture-b").await;
        assert_eq!(targeted.len(), 1);
        assert_eq!(targeted[0]["actionId"], "restart");
        assert_eq!(
            action_log(&app, &access, &id, "?actionId=restart&targetId=fixture-a")
                .await
                .len(),
            1
        );

        // `limit` narrows the page...
        assert_eq!(action_log(&app, &access, &id, "?limit=2").await.len(), 2);
        // ...and an absurd one is clamped rather than refused, because the
        // caller meant "as much as there is".
        assert_eq!(
            action_log(&app, &access, &id, "?limit=100000").await.len(),
            5
        );

        // A filter matching nothing is an empty list, not a 404: the instance
        // exists and has simply never been asked to do that.
        assert!(action_log(&app, &access, &id, "?actionId=never-invoked")
            .await
            .is_empty());

        // An unknown instance *is* a 404 — "nothing happened here" and "there
        // is no here" are different answers.
        let (status, _) = send(
            &app.router,
            get_with_auth(
                "/connector-instances/00000000-0000-0000-0000-000000000000/action-log",
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn the_global_audit_log_joins_context_filters_and_requires_manage() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let admin_id = current_user_id(&app.router, &admin).await;
        let first = create_debug_instance(&app.router, &admin, "First fixture").await;
        let second = create_debug_instance(&app.router, &admin, "Second fixture").await;

        let (status, body) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{first}/actions/ping"),
                &admin,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:#}");

        // A connector refusal is still an audit row and must be filterable as
        // a failure rather than disappearing from the global view.
        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{second}/actions/set-load"),
                &admin,
                serde_json::json!({ "value": 900 }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        // Fixed-width timestamps make the before/after assertions independent
        // of scheduler timing while exercising the same TEXT range query used
        // in production.
        sqlx::query(
            "UPDATE connector_action_log SET invoked_at = CASE action_id \
             WHEN 'ping' THEN '2026-08-30T12:00:10+00:00' \
             ELSE '2026-08-30T12:00:20+00:00' END",
        )
        .execute(&app.pool)
        .await
        .expect("stamp audit fixtures");

        let entries = global_audit_log(&app, &admin, "").await;
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["instanceId"], second);
        assert_eq!(entries[0]["instanceName"], "Second fixture");
        assert_eq!(entries[0]["connectorType"], "debug");
        assert_eq!(entries[0]["invokedBy"]["username"], "admin");
        assert_eq!(entries[0]["success"], false);
        assert_eq!(entries[1]["instanceId"], first);

        assert_eq!(
            global_audit_log(&app, &admin, &format!("?instanceId={first}"))
                .await
                .len(),
            1
        );
        assert_eq!(
            global_audit_log(&app, &admin, "?actionId=ping&success=true")
                .await
                .len(),
            1
        );
        assert_eq!(
            global_audit_log(&app, &admin, &format!("?userId={admin_id}"))
                .await
                .len(),
            2
        );
        assert_eq!(
            global_audit_log(&app, &admin, "?before=2026-08-30T12:00:15Z").await[0]["actionId"],
            "ping"
        );
        assert_eq!(
            global_audit_log(&app, &admin, "?after=2026-08-30T12:00:15Z").await[0]["actionId"],
            "set-load"
        );
        assert_eq!(global_audit_log(&app, &admin, "?limit=1").await.len(), 1);

        let (status, body) = send(
            &app.router,
            get_with_auth("/audit-log?before=not-a-timestamp", &bearer(&admin)),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");

        let viewer = user_with_grants(
            &app.router,
            &admin,
            "audit-viewer",
            serde_json::json!([{
                "key": "connectors.view",
                "resourceType": null,
                "resourceId": null,
            }]),
        )
        .await;
        let (status, _) = send(&app.router, get_with_auth("/audit-log", &bearer(&viewer))).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    /// Reading the history is looking, not doing — and the people who most need
    /// "what happened to this?" are the ones who could not have done it.
    #[tokio::test]
    async fn the_action_log_is_readable_with_view_and_not_without_it() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &admin, "Audited").await;
        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/ping"),
                &admin,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let viewer = user_with_grants(
            &app.router,
            &admin,
            "viewer",
            serde_json::json!([{
                "key": "connectors.view",
                "resourceType": null,
                "resourceId": null,
            }]),
        )
        .await;
        assert_eq!(action_log(&app, &viewer, &id, "").await.len(), 1);

        let nobody = user_with_grants(&app.router, &admin, "nobody", serde_json::json!([])).await;
        let (status, _) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/action-log"),
                &bearer(&nobody),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = send(
            &app.router,
            get(&format!("/connector-instances/{id}/action-log")),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// The log outlives the action but not the instance: a deleted connector
    /// takes its history with it, because the rows reference a row that is no
    /// longer there and a log about nothing is not evidence.
    #[tokio::test]
    async fn deleting_an_instance_takes_its_action_log_with_it() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &access, "Doomed").await;
        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/ping"),
                &access,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM connector_action_log WHERE instance_id = ?")
                .bind(&id)
                .fetch_one(&app.pool)
                .await
                .expect("counting log rows");
        assert_eq!(rows, 1);

        let (status, _) = send(
            &app.router,
            delete_auth(&format!("/connector-instances/{id}"), &access),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM connector_action_log WHERE instance_id = ?")
                .bind(&id)
                .fetch_one(&app.pool)
                .await
                .expect("counting log rows");
        assert_eq!(
            rows, 0,
            "the cascade must clear the history with the instance"
        );
    }

    /// The other half of that decision: a *user* who appears in the log cannot
    /// be deleted out from under it. Attribution a later delete can erase is
    /// not an audit trail, so the account is refused with an explanation rather
    /// than the database refusing it with a 500.
    #[tokio::test]
    async fn a_user_named_in_the_action_log_cannot_be_deleted() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &admin, "Audited").await;
        let operator = user_with_grants(
            &app.router,
            &admin,
            "operator",
            serde_json::json!([{
                "key": "connectors.control",
                "resourceType": null,
                "resourceId": null,
            }]),
        )
        .await;
        let operator_id = current_user_id(&app.router, &operator).await;

        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/ping"),
                &operator,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = send(
            &app.router,
            delete_auth(&format!("/users/{operator_id}"), &admin),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body:#}");
        assert!(body["error"]
            .as_str()
            .is_some_and(|message| message.contains("action log")));
    }

    #[tokio::test]
    async fn the_update_check_capability_reaches_the_instance_detail() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        // The fixture supports checking, and reports whatever its stored
        // configuration asked for — both answers reachable without a registry.
        let outdated = DebugConnector::from_config_value(serde_json::json!({
            "simulatedUpdateAvailable": true,
        }))
        .expect("a valid fixture configuration");
        assert!(outdated.supports_update_checking());
        assert_eq!(
            outdated
                .check_for_updates(None)
                .await
                .expect("the check should succeed")
                .latest_ref
                .as_deref(),
            Some("debug-fixture:2.0.0")
        );

        // And an instance built through the ordinary create endpoint carries
        // that configuration into its live connector.
        let (status, created) = send(
            &app.router,
            post_json_auth(
                "/connector-instances",
                &access,
                serde_json::json!({
                    "connectorType": "debug",
                    "name": "Outdated",
                    "config": { "simulatedUpdateAvailable": true },
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created:#}");
        let uuid = uuid::Uuid::parse_str(created["id"].as_str().expect("id")).expect("uuid");
        let connector = app.connectors.get(&uuid).await.expect("a live connector");
        assert!(connector.supports_update_checking());
        assert!(
            connector
                .check_for_updates(None)
                .await
                .expect("the check should succeed")
                .available
        );
    }

    /* ---------------------------------------------------------------- */
    /* Update scheduler                                                  */
    /* ---------------------------------------------------------------- */

    /// A connector that can be checked for updates and can apply one.
    ///
    /// Not `DebugConnector` extended: the fixture in Core deliberately denies
    /// unknown configuration keys, and what is under test here is the
    /// *platform's* scheduler — the settings convention, the interval, the
    /// stagger, the auto-apply path — against any connector that participates.
    /// A local fixture keeps that generality honest, and lets the test count
    /// exactly how many times a registry would have been asked.
    /// One recorded `check_for_updates` call: which target, and when.
    type CheckRecord = (Option<String>, std::time::Instant);

    #[derive(Clone)]
    struct UpdatableConnector {
        targets: Vec<String>,
        /// One entry per `check_for_updates` call: the target, and when.
        checks: Arc<std::sync::Mutex<Vec<CheckRecord>>>,
        /// One entry per `applyUpdate` invocation.
        applied: Arc<std::sync::Mutex<Vec<(String, String)>>>,
        available: Arc<std::sync::atomic::AtomicBool>,
    }

    impl UpdatableConnector {
        fn new(targets: &[&str]) -> Self {
            Self {
                targets: targets.iter().map(|target| (*target).to_owned()).collect(),
                checks: Arc::new(std::sync::Mutex::new(Vec::new())),
                applied: Arc::new(std::sync::Mutex::new(Vec::new())),
                available: Arc::new(std::sync::atomic::AtomicBool::new(true)),
            }
        }

        fn checks(&self) -> Vec<CheckRecord> {
            self.checks.lock().unwrap().clone()
        }

        fn applied(&self) -> Vec<(String, String)> {
            self.applied.lock().unwrap().clone()
        }
    }

    #[async_trait::async_trait]
    impl Connector for UpdatableConnector {
        async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
            let mut details = Value::Object(Default::default());
            for target in &self.targets {
                loom_core::connector::details::set_detail(
                    &mut details,
                    Some(target),
                    "currentImageRef",
                    serde_json::json!(format!("example/{target}:1.0")),
                );
            }
            Ok(ConnectorStatus::new(
                loom_core::connector::HealthState::Healthy,
                details,
            ))
        }

        async fn actions(&self) -> Vec<ConnectorAction> {
            self.targets
                .iter()
                .map(|target| {
                    ConnectorAction::simple("applyUpdate", "Apply update")
                        .disruptive()
                        .snapshotting(["currentImageRef"])
                        .for_target(target)
                })
                .collect()
        }

        async fn execute_action(
            &self,
            action_id: &str,
            target_id: Option<&str>,
            params: Value,
        ) -> Result<ActionResult, ConnectorError> {
            if action_id != "applyUpdate" {
                return Err(ConnectorError::invalid_action(action_id));
            }
            let target = target_id.unwrap_or_default().to_owned();
            let image = params
                .get("targetImageRef")
                .and_then(Value::as_str)
                .ok_or_else(|| ConnectorError::InvalidParams {
                    action_id: action_id.to_owned(),
                    reason: "expected `targetImageRef`".to_owned(),
                })?
                .to_owned();
            self.applied
                .lock()
                .unwrap()
                .push((target.clone(), image.clone()));
            Ok(ActionResult::ok(format!("{target} now running {image}")))
        }

        /// One kind for every view, plus a second that only a target has.
        ///
        /// Mirrors the shape the parameter exists for — Docker's stack members
        /// — without this test needing Docker: what matters here is that the
        /// route reaches the connector with the target the caller named.
        fn resource_kinds(
            &self,
            target_id: Option<&str>,
        ) -> Vec<loom_core::connector::ResourceKindDescriptor> {
            let mut kinds = vec![loom_core::connector::ResourceKindDescriptor::new(
                "everywhere",
                "Everywhere",
                Vec::new(),
            )];
            if let Some(target) = target_id {
                kinds.push(loom_core::connector::ResourceKindDescriptor::new(
                    "targetOnly",
                    format!("Only {target}"),
                    Vec::new(),
                ));
            }
            kinds
        }

        fn supports_sub_targets(&self) -> bool {
            true
        }

        async fn list_sub_targets(
            &self,
        ) -> Result<Vec<loom_core::connector::SubTarget>, ConnectorError> {
            Ok(self
                .targets
                .iter()
                .map(|id| loom_core::connector::SubTarget::new(id.clone(), id.clone()))
                .collect())
        }

        fn supports_update_checking(&self) -> bool {
            true
        }

        async fn check_for_updates(
            &self,
            target_id: Option<&str>,
        ) -> Result<loom_core::connector::UpdateCheckResult, ConnectorError> {
            self.checks
                .lock()
                .unwrap()
                .push((target_id.map(str::to_owned), std::time::Instant::now()));
            Ok(
                if self.available.load(std::sync::atomic::Ordering::SeqCst) {
                    loom_core::connector::UpdateCheckResult::available(format!(
                        "example/{}@sha256:newer",
                        target_id.unwrap_or("host")
                    ))
                } else {
                    loom_core::connector::UpdateCheckResult::up_to_date()
                },
            )
        }

        fn config_schema(&self) -> Value {
            serde_json::json!({ "type": "object" })
        }

        fn metadata(&self) -> ConnectorMetadata {
            ConnectorMetadata {
                id: "debug".to_owned(),
                name: "Updatable fixture".to_owned(),
                icon: None,
                version: "1.0.0".to_owned(),
                min_size: (1, 1),
            }
        }

        fn display_fields(&self) -> Vec<DisplayField> {
            Vec::new()
        }

        fn data_points(&self) -> Vec<DataPointDescriptor> {
            self.targets
                .iter()
                .map(|target| {
                    DataPointDescriptor::new(
                        "currentImageRef",
                        "Image",
                        loom_core::connector::DataPointValueType::String,
                    )
                    .for_target(target)
                })
                .collect()
        }

        fn default_layout(&self) -> WidgetLayout {
            WidgetLayout::default()
        }
    }

    /// Installs an updatable connector over a real instance row and gives that
    /// row the update settings under test.
    ///
    /// The settings go into the stored configuration directly rather than
    /// through the create endpoint, because the settings convention is the
    /// *platform's* and the fixture connector's schema is not the thing being
    /// tested.
    async fn updatable_instance(
        app: &TestApp,
        token: &str,
        name: &str,
        targets: &[&str],
        settings: serde_json::Value,
    ) -> (String, UpdatableConnector) {
        let id = create_debug_instance(&app.router, token, name).await;
        sqlx::query("UPDATE connector_instances SET config = ? WHERE id = ?")
            .bind(settings.to_string())
            .bind(&id)
            .execute(&app.pool)
            .await
            .expect("storing the update settings");

        let connector = UpdatableConnector::new(targets);
        app.connectors
            .insert(
                uuid::Uuid::parse_str(&id).expect("uuid"),
                Arc::new(connector.clone()),
            )
            .await;
        (id, connector)
    }

    /// Part 2's whole job: the route reaches the connector with the target the
    /// caller named, so a kind that only one sort of target has is *absent*
    /// elsewhere rather than merely empty.
    #[tokio::test]
    async fn the_resource_kinds_route_passes_its_target_to_the_connector() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let (id, _) =
            updatable_instance(&app, &access, "Targeted", &["web"], serde_json::json!({})).await;

        let kinds = |query: &str| {
            let router = app.router.clone();
            let token = bearer(&access);
            let path = format!("/connector-instances/{id}/resource-kinds{query}");
            async move {
                let (status, body) = send(&router, get_with_auth(&path, &token)).await;
                assert_eq!(status, StatusCode::OK, "{body:#}");
                body.as_array()
                    .expect("an array of descriptors")
                    .iter()
                    .map(|kind| kind["kind"].as_str().unwrap_or_default().to_owned())
                    .collect::<Vec<_>>()
            }
        };

        // No target: only the kind that exists everywhere, plus whatever the
        // platform contributes for an update-checking connector.
        let host = kinds("").await;
        assert!(host.contains(&"everywhere".to_owned()));
        assert!(!host.contains(&"targetOnly".to_owned()));

        // With one: the target-conditional kind appears, labelled for it.
        let targeted = kinds("?targetId=web").await;
        assert!(targeted.contains(&"everywhere".to_owned()));
        assert!(targeted.contains(&"targetOnly".to_owned()));

        // A blank parameter is an absent one — an empty form field must not
        // become a third case every connector has to think about.
        assert_eq!(kinds("?targetId=").await, host);

        // And a listing is validated against *that target's* kinds: a kind only
        // a target has is as much "no such kind" for the host as one nothing
        // publishes at all.
        let (status, _) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/resources/targetOnly"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        let (status, _) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/resources/targetOnly?targetId=web"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn the_scheduler_checks_only_what_is_configured_and_only_when_due() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (checked_id, checked) = updatable_instance(
            &app,
            &access,
            "Checked",
            &["web"],
            serde_json::json!({ "checkForUpdates": true, "checkIntervalMinutes": 60 }),
        )
        .await;
        // Same connector, same capability — but its configuration never asked
        // for checking, so it is never contacted.
        let (_, unchecked) = updatable_instance(
            &app,
            &access,
            "Unchecked",
            &["web"],
            serde_json::json!({ "checkForUpdates": false }),
        )
        .await;

        let start = chrono::Utc::now();
        let state = app.state.clone();
        connectors::updates::run_tick(&state, start, Duration::ZERO).await;
        assert_eq!(checked.checks().len(), 1);
        assert!(
            unchecked.checks().is_empty(),
            "opting out must mean opting out"
        );

        // A tick a minute later is not due: the interval is an hour.
        connectors::updates::run_tick(&state, start + chrono::Duration::minutes(1), Duration::ZERO)
            .await;
        assert_eq!(checked.checks().len(), 1, "the interval must be respected");

        // An hour later it is.
        connectors::updates::run_tick(
            &state,
            start + chrono::Duration::minutes(61),
            Duration::ZERO,
        )
        .await;
        assert_eq!(checked.checks().len(), 2);

        // And the reading reaches the client, per target, with its own age.
        let (status, detail) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{checked_id}"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{detail:#}");
        assert_eq!(detail["supportsUpdateChecking"], true);
        assert_eq!(detail["updateStatus"]["web"]["available"], true);
        assert_eq!(
            detail["updateStatus"]["web"]["latestRef"],
            "example/web@sha256:newer"
        );
        assert!(detail["updateStatus"]["web"]["lastChecked"].is_string());
    }

    /// Registries rate-limit by source address. Thirty containers checked in
    /// the same instant is the fastest way to be told to go away.
    #[tokio::test]
    async fn the_scheduler_staggers_its_checks_within_one_instance() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let (_, connector) = updatable_instance(
            &app,
            &access,
            "Busy",
            &["one", "two", "three"],
            serde_json::json!({ "checkForUpdates": true }),
        )
        .await;

        let stagger = Duration::from_millis(60);
        connectors::updates::run_tick(&app.state, chrono::Utc::now(), stagger).await;

        let checks = connector.checks();
        assert_eq!(checks.len(), 3, "every target is checked");
        assert_eq!(
            checks
                .iter()
                .map(|(target, _)| target.as_deref().unwrap_or(""))
                .collect::<Vec<_>>(),
            vec!["one", "two", "three"],
            "sequentially, in enumeration order"
        );
        for pair in checks.windows(2) {
            assert!(
                pair[1].1.duration_since(pair[0].1) >= stagger,
                "consecutive checks must be spaced out"
            );
        }
    }

    #[tokio::test]
    async fn an_automatic_update_runs_through_the_ordinary_action_path() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let (id, connector) = updatable_instance(
            &app,
            &access,
            "Auto",
            &["web"],
            serde_json::json!({
                "checkForUpdates": true,
                "autoApplyUpdates": true,
            }),
        )
        .await;

        // The snapshot is read from the poll cache, so there has to have been a
        // poll — production's poller does this on its own tick.
        app.connectors.poll_once().await;

        connectors::updates::run_tick(&app.state, chrono::Utc::now(), Duration::ZERO).await;

        assert_eq!(
            connector.applied(),
            vec![("web".to_owned(), "example/web@sha256:newer".to_owned())],
            "an available update with no window is applied at once"
        );

        // The point of routing it through `invoke_action`: it is in the audit
        // log, attributed to Loom rather than to a person, with the pre-action
        // image recorded by the snapshot mechanism.
        let entries = action_log(&app, &access, &id, "").await;
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry["actionId"], "applyUpdate");
        assert_eq!(entry["targetId"], "web");
        assert_eq!(entry["invokedBy"]["system"], true);
        assert_eq!(entry["invokedBy"]["id"], serde_json::Value::Null);
        assert_eq!(entry["invokedBy"]["username"], serde_json::Value::Null);
        assert_eq!(entry["success"], true);
        assert_eq!(
            entry["snapshot"],
            serde_json::json!({ "currentImageRef": "example/web:1.0" }),
            "the reference it was running before is what a rollback needs"
        );
        assert_eq!(
            entry["params"],
            serde_json::json!({ "targetImageRef": "example/web@sha256:newer" })
        );

        // Applied once, not once per tick: the cached availability is cleared
        // when the update succeeds.
        connectors::updates::run_tick(&app.state, chrono::Utc::now(), Duration::ZERO).await;
        assert_eq!(connector.applied().len(), 1);
    }

    #[tokio::test]
    async fn an_excluded_instance_is_checked_but_never_applied() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let (_, connector) = updatable_instance(
            &app,
            &access,
            "Hands off",
            &["web"],
            serde_json::json!({
                "checkForUpdates": true,
                "autoApplyUpdates": true,
                "excludeFromAutoUpdate": true,
            }),
        )
        .await;

        connectors::updates::run_tick(&app.state, chrono::Utc::now(), Duration::ZERO).await;
        assert_eq!(connector.checks().len(), 1, "it is still reported on");
        assert!(
            connector.applied().is_empty(),
            "the exclusion is what makes 'tell me, do not touch it' expressible"
        );
    }

    /// A maintenance window is a promise that the service will not vanish
    /// during the day.
    #[tokio::test]
    async fn a_maintenance_window_holds_an_update_until_its_hour() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        // A fixed local afternoon rather than "now plus two minutes": a
        // relative window computed near midnight would wrap to the next day
        // and the test would fail once a day for a two-minute band. The
        // scheduler is told what time it is, so the test picks one.
        let midday = chrono::Local
            .with_ymd_and_hms(2026, 1, 15, 12, 0, 0)
            .single()
            .expect("a real local time");
        let window = "12:30";
        let (_, connector) = updatable_instance(
            &app,
            &access,
            "Windowed",
            &["web"],
            serde_json::json!({
                "checkForUpdates": true,
                "autoApplyUpdates": true,
                "autoApplyAtTime": window,
            }),
        )
        .await;

        connectors::updates::run_tick(
            &app.state,
            midday.with_timezone(&chrono::Utc),
            Duration::ZERO,
        )
        .await;
        assert_eq!(connector.checks().len(), 1);
        assert!(
            connector.applied().is_empty(),
            "before the window, an available update waits"
        );

        // A tick after the window opens applies it.
        connectors::updates::run_tick(
            &app.state,
            (midday + chrono::Duration::minutes(31)).with_timezone(&chrono::Utc),
            Duration::ZERO,
        )
        .await;
        assert_eq!(connector.applied().len(), 1, "inside the window, it runs");

        // And not again the same day, however many ticks follow — a container
        // whose update keeps failing must not be recreated every minute from
        // 03:00 until somebody notices.
        connectors::updates::run_tick(
            &app.state,
            (midday + chrono::Duration::minutes(45)).with_timezone(&chrono::Utc),
            Duration::ZERO,
        )
        .await;
        assert_eq!(connector.applied().len(), 1, "one window, one attempt");
    }

    #[tokio::test]
    async fn the_recently_updated_table_is_the_action_log_and_its_rows_roll_back() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;
        let (id, _connector) = updatable_instance(
            &app,
            &access,
            "History",
            &["web"],
            serde_json::json!({ "checkForUpdates": true }),
        )
        .await;
        app.connectors.poll_once().await;

        // A person applying an update by hand, through the ordinary endpoint.
        let (status, body) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/applyUpdate"),
                &access,
                serde_json::json!({
                    "targetId": "web",
                    "params": { "targetImageRef": "example/web:2.0" },
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body:#}");

        let (status, kinds) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/resource-kinds"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{kinds:#}");
        let recently = kinds
            .as_array()
            .expect("kinds")
            .iter()
            .find(|kind| kind["kind"] == "recentlyUpdated")
            .expect("the platform kind is offered for an update-checking connector")
            .clone();
        assert_eq!(
            recently["columns"]
                .as_array()
                .expect("columns")
                .iter()
                .map(|column| column["key"].as_str().expect("key"))
                .collect::<Vec<_>>(),
            vec![
                "targetId",
                "targetImageRef",
                "newRef",
                "appliedAt",
                "appliedBy"
            ]
        );

        let (status, rows) = send(
            &app.router,
            get_with_auth(
                &format!("/connector-instances/{id}/resources/recentlyUpdated"),
                &bearer(&access),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{rows:#}");
        let rows = rows.as_array().expect("rows");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["fields"]["targetId"], "web");
        assert_eq!(rows[0]["fields"]["newRef"], "example/web:2.0");
        // The rollback target, recorded by the snapshot mechanism rather than
        // by anything this feature stores.
        assert_eq!(rows[0]["fields"]["targetImageRef"], "example/web:1.0");
        assert_eq!(rows[0]["fields"]["appliedBy"], "admin");
        assert!(rows[0]["fields"]["appliedAt"].is_string());
    }

    #[tokio::test]
    async fn connector_routes_reject_an_anonymous_caller() {
        let app = test_app().await;
        setup_and_login(&app.router).await;

        for request in [
            get("/connector-types"),
            get("/connector-instances"),
            get("/connector-instances/tags"),
            get("/connector-instances/whatever"),
        ] {
            let (status, _) = send(&app.router, request).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }

        let (status, _) = send(
            &app.router,
            post_json(
                "/connector-instances/whatever/actions/ping",
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, _) = send(
            &app.router,
            post_json("/connector-types/debug/discover", serde_json::Value::Null),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);

        let (status, _) = send(
            &app.router,
            post_json(
                "/connector-types/debug/test-connection",
                serde_json::Value::Null,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }

    /// `connectors.manage` is a real split, not a synonym: viewing instances
    /// and deciding which exist are separately granted.
    #[tokio::test]
    async fn managing_instances_needs_more_than_viewing_them() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &admin, "Fixture").await;

        let viewer = user_with_grants(
            &app.router,
            &admin,
            "viewer",
            serde_json::json!([{
                "key": "connectors.view",
                "resourceType": null,
                "resourceId": null,
            }]),
        )
        .await;

        // View works.
        let (status, _) = send(
            &app.router,
            get_with_auth("/connector-instances", &bearer(&viewer)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(
            &app.router,
            get_with_auth("/connector-instances/tags", &bearer(&viewer)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(
            &app.router,
            get_with_auth(&format!("/connector-instances/{id}"), &bearer(&viewer)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Everything that changes the instance list does not.
        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/connector-instances",
                &viewer,
                serde_json::json!({ "connectorType": "debug", "name": "Nope", "config": {} }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/connector-types/debug/discover",
                &viewer,
                serde_json::Value::Null,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/connector-types/debug/test-connection",
                &viewer,
                serde_json::Value::Null,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/discover"),
                &viewer,
                serde_json::Value::Null,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = send(
            &app.router,
            patch_json_auth(
                &format!("/connector-instances/{id}"),
                &viewer,
                serde_json::json!({ "name": "Nope" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        let (status, _) = send(
            &app.router,
            delete_auth(&format!("/connector-instances/{id}"), &viewer),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // And the type catalog is part of adding one.
        let (status, _) = send(
            &app.router,
            get_with_auth("/connector-types", &bearer(&viewer)),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // A manager who cannot view still manages.
        let manager = user_with_grants(
            &app.router,
            &admin,
            "manager",
            serde_json::json!([{
                "key": "connectors.manage",
                "resourceType": null,
                "resourceId": null,
            }]),
        )
        .await;
        let (status, _) = send(
            &app.router,
            get_with_auth("/connector-types", &bearer(&manager)),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/connector-types/debug/test-connection",
                &manager,
                serde_json::Value::Null,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = send(
            &app.router,
            get_with_auth("/connector-instances", &bearer(&manager)),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = send(
            &app.router,
            get_with_auth("/connector-instances/tags", &bearer(&manager)),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    /* ---------------------------------------------------------------- */
    /* Dashboard ownership and sharing                                   */
    /* ---------------------------------------------------------------- */

    #[tokio::test]
    async fn dashboard_roles_sharing_placements_and_cascades_are_enforced() {
        let app = test_app().await;
        let (owner, _) = setup_and_login(&app.router).await;
        let connector_id = create_debug_instance(&app.router, &owner, "Dashboard fixture").await;

        let (status, created) = send(
            &app.router,
            post_json_auth(
                "/dashboards",
                &owner,
                serde_json::json!({ "name": "Operations" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created:#}");
        assert_eq!(created["role"], "owner");
        let dashboard_id = created["id"].as_str().expect("dashboard id").to_owned();

        // Ownership is sufficient even though dashboards have no RBAC key.
        let (status, renamed) = send(
            &app.router,
            patch_json_auth(
                &format!("/dashboards/{dashboard_id}"),
                &owner,
                serde_json::json!({ "name": "Renamed operations" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{renamed:#}");
        assert_eq!(renamed["name"], "Renamed operations");

        let editor = user_with_grants(&app.router, &owner, "editor", serde_json::json!([])).await;
        let editor_id = current_user_id(&app.router, &editor).await;
        let viewer = user_with_grants(&app.router, &owner, "viewer", serde_json::json!([])).await;
        let viewer_id = current_user_id(&app.router, &viewer).await;
        let outsider =
            user_with_grants(&app.router, &owner, "outsider", serde_json::json!([])).await;

        for (target_id, role) in [(&editor_id, "edit"), (&viewer_id, "view")] {
            let (status, share) = send(
                &app.router,
                post_json_auth(
                    &format!("/dashboards/{dashboard_id}/shares"),
                    &owner,
                    serde_json::json!({
                        "targetType": "user",
                        "targetId": target_id,
                        "role": role,
                    }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{share:#}");
        }

        // A weaker group share does not downgrade the editor's stronger direct
        // share; role resolution must take the highest applicable role.
        let editor_group_id = group_id_named(&app.router, &owner, "editor-group").await;
        let (status, weaker_share) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/shares"),
                &owner,
                serde_json::json!({
                    "targetType": "group",
                    "targetId": editor_group_id,
                    "role": "view",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{weaker_share:#}");

        let (status, duplicate) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/shares"),
                &owner,
                serde_json::json!({
                    "targetType": "user",
                    "targetId": editor_id,
                    "role": "view",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{duplicate:#}");

        let (status, invalid_target) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/shares"),
                &owner,
                serde_json::json!({
                    "targetType": "user",
                    "targetId": "00000000-0000-4000-8000-00000000ffff",
                    "role": "view",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{invalid_target:#}");

        // An editor can place despite having no connector RBAC grants. The
        // dashboard ACL authorizes the layout mutation; it does not authorize
        // connector actions.
        let (status, placement) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/placements"),
                &editor,
                serde_json::json!({
                    "connectorInstanceId": connector_id,
                    "positionX": 1,
                    "positionY": 2,
                    "width": 2,
                    "height": 2,
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{placement:#}");
        assert_eq!(placement["connector"]["id"], connector_id);
        assert!(placement["widgetBindings"]
            .as_array()
            .is_some_and(|bindings| !bindings.is_empty()));
        let placement_id = placement["id"].as_str().expect("placement id").to_owned();

        let (status, updated_placement) = send(
            &app.router,
            patch_json_auth(
                &format!("/dashboards/{dashboard_id}/placements/{placement_id}"),
                &editor,
                serde_json::json!({ "positionX": 4, "width": 3 }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{updated_placement:#}");
        assert_eq!(updated_placement["positionX"], 4);
        assert_eq!(updated_placement["width"], 3);

        // Editors mutate placements but cannot rename, delete, or share.
        for request in [
            patch_json_auth(
                &format!("/dashboards/{dashboard_id}"),
                &editor,
                serde_json::json!({ "name": "Not allowed" }),
            ),
            delete_auth(&format!("/dashboards/{dashboard_id}"), &editor),
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/shares"),
                &editor,
                serde_json::json!({
                    "targetType": "user",
                    "targetId": viewer_id,
                    "role": "edit",
                }),
            ),
        ] {
            let (status, _) = send(&app.router, request).await;
            assert_eq!(status, StatusCode::FORBIDDEN);
        }

        let (status, viewer_detail) = send(
            &app.router,
            get_with_auth(&format!("/dashboards/{dashboard_id}"), &bearer(&viewer)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{viewer_detail:#}");
        assert_eq!(viewer_detail["role"], "viewer");
        assert_eq!(
            viewer_detail["placements"][0]["connector"]["id"],
            connector_id
        );

        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/pin"),
                &viewer,
                serde_json::Value::Null,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, viewer_dashboards) =
            send(&app.router, get_with_auth("/dashboards", &bearer(&viewer))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(viewer_dashboards[0]["pinned"], true);

        let (status, _) = send(
            &app.router,
            delete_auth(&format!("/dashboards/{dashboard_id}/pin"), &viewer),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, viewer_dashboards) =
            send(&app.router, get_with_auth("/dashboards", &bearer(&viewer))).await;
        assert_eq!(viewer_dashboards[0]["pinned"], false);

        // Re-pin so dashboard deletion has a real pin row to cascade.
        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/pin"),
                &viewer,
                serde_json::Value::Null,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, owner_dashboards) =
            send(&app.router, get_with_auth("/dashboards", &bearer(&owner))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(owner_dashboards[0]["pinned"], false);

        let (status, _) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/placements"),
                &viewer,
                serde_json::json!({
                    "connectorInstanceId": connector_id,
                    "positionX": 0,
                    "positionY": 0,
                    "width": 2,
                    "height": 2,
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // The owner can grant and revoke an individual share. Revocation takes
        // effect immediately because roles are read from the dashboard ACL,
        // not cached in the access token.
        let outsider_id = current_user_id(&app.router, &outsider).await;
        let (status, outsider_dashboard) = send(
            &app.router,
            post_json_auth(
                "/dashboards",
                &outsider,
                serde_json::json!({ "name": "Owned content" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{outsider_dashboard:#}");
        let (status, ownership_conflict) = send(
            &app.router,
            delete_auth(&format!("/users/{outsider_id}"), &owner),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{ownership_conflict:#}");

        let (status, temporary_share) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/shares"),
                &owner,
                serde_json::json!({
                    "targetType": "user",
                    "targetId": outsider_id,
                    "role": "view",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{temporary_share:#}");
        let temporary_share_id = temporary_share["id"].as_str().expect("share id");
        let (status, _) = send(
            &app.router,
            delete_auth(
                &format!("/dashboards/{dashboard_id}/shares/{temporary_share_id}"),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = send(
            &app.router,
            get_with_auth(&format!("/dashboards/{dashboard_id}"), &bearer(&outsider)),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // The live connector declares a 2x2 minimum.
        let (status, too_small) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/placements"),
                &owner,
                serde_json::json!({
                    "connectorInstanceId": connector_id,
                    "positionX": 0,
                    "positionY": 0,
                    "width": 1,
                    "height": 2,
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{too_small:#}");

        // Each binding kind is checked against its own namespace, so a bogus
        // display binding and a bogus action binding fail for different stated
        // reasons rather than one catch-all.
        let placement_body = |bindings: serde_json::Value| {
            serde_json::json!({
                "connectorInstanceId": connector_id,
                "positionX": 0,
                "positionY": 0,
                "width": 2,
                "height": 2,
                "widgetBindings": bindings,
            })
        };

        let (status, bad_binding) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/placements"),
                &owner,
                placement_body(serde_json::json!([{
                    "display": {
                        "dataPointId": "does-not-exist",
                        "widgetType": "statTile",
                        "config": {},
                    }
                }])),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_binding:#}");
        let message = bad_binding["error"].as_str().unwrap_or_default();
        assert!(
            message.contains("unknown data points") && message.contains("does-not-exist"),
            "{message}"
        );

        let (status, bad_action) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/placements"),
                &owner,
                placement_body(serde_json::json!([{
                    "action": {
                        "actionId": "not-an-action",
                        "widgetType": "button",
                        "config": {},
                    }
                }])),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad_action:#}");
        let message = bad_action["error"].as_str().unwrap_or_default();
        assert!(
            message.contains("unknown actions") && message.contains("not-an-action"),
            "{message}"
        );
        // An action id is not a data point id: the error has to send the reader
        // to the right half of the connector.
        assert!(!message.contains("unknown data points"), "{message}");

        // A target is checked live, and its binding namespace is coupled to
        // that exact target rather than merely to the connector instance.
        let target_placement = |target_id: &str, data_point_id: &str| {
            serde_json::json!({
                "connectorInstanceId": connector_id,
                "targetId": target_id,
                "positionX": 0,
                "positionY": 0,
                "width": 2,
                "height": 2,
                "widgetBindings": [{
                    "display": {
                        "dataPointId": data_point_id,
                        "widgetType": "statTile",
                        "config": {},
                    }
                }],
            })
        };
        let (status, wrong_target_binding) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/placements"),
                &owner,
                target_placement("fixture-a", loom_core::connector::debug::DATA_POINT_LABEL),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{wrong_target_binding:#}");
        assert!(wrong_target_binding["error"]
            .as_str()
            .is_some_and(|message| message.contains("unknown data points")));

        let (status, targeted) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/placements"),
                &owner,
                target_placement("fixture-b", loom_core::connector::debug::DATA_POINT_LABEL),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{targeted:#}");
        assert_eq!(targeted["targetId"], "fixture-b");

        let (status, missing_target) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/placements"),
                &owner,
                target_placement(
                    "fixture-missing",
                    loom_core::connector::debug::DATA_POINT_LABEL,
                ),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{missing_target:#}");
        assert!(missing_target["error"]
            .as_str()
            .is_some_and(|message| message.contains("no such connector sub-target")));

        // ...and a layout mixing both kinds, all of them real, is accepted.
        let mixed_bindings = serde_json::json!([
            {
                "display": {
                    "dataPointId": loom_core::connector::debug::DATA_POINT_LOAD,
                    "widgetType": "gauge",
                    "config": { "min": 0, "max": 100 },
                }
            },
            {
                "action": {
                    "actionId": loom_core::connector::debug::ACTION_SET_ENABLED,
                    "widgetType": "toggle",
                    "config": {},
                }
            },
        ]);
        let (status, mixed) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/placements"),
                &owner,
                placement_body(mixed_bindings.clone()),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{mixed:#}");
        assert_eq!(mixed["widgetBindings"], mixed_bindings);

        let mixed_placement_id = mixed["id"].as_str().expect("a placement id").to_string();
        let (status, _) = send(
            &app.router,
            delete_auth(
                &format!("/dashboards/{dashboard_id}/placements/{mixed_placement_id}"),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        // A group share applies to every current member, not only the user who
        // happened to identify the group for the test.
        let group_member =
            user_with_grants(&app.router, &owner, "group-member", serde_json::json!([])).await;
        let group_id = group_id_named(&app.router, &owner, "group-member-group").await;
        let (_second_member_id, second_member) =
            create_user_in_group(&app.router, &owner, "second-member", &group_id).await;
        let (status, group_share) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/shares"),
                &owner,
                serde_json::json!({
                    "targetType": "group",
                    "targetId": group_id,
                    "role": "view",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{group_share:#}");
        assert_eq!(group_share["resolvedName"], "group-member-group");
        for member in [&group_member, &second_member] {
            let (status, detail) = send(
                &app.router,
                get_with_auth(&format!("/dashboards/{dashboard_id}"), &bearer(member)),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{detail:#}");
            assert_eq!(detail["role"], "viewer");
        }

        let (status, shares) = send(
            &app.router,
            get_with_auth(
                &format!("/dashboards/{dashboard_id}/shares"),
                &bearer(&owner),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{shares:#}");
        assert_eq!(shares.as_array().expect("shares").len(), 4);

        // The owner can delete, and every dependent row must go with it.
        let (status, _) = send(
            &app.router,
            delete_auth(&format!("/dashboards/{dashboard_id}"), &owner),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        for table in ["dashboard_shares", "dashboard_pins", "dashboard_placements"] {
            let query = format!("SELECT COUNT(*) FROM {table} WHERE dashboard_id = ?");
            let (count,): (i64,) = sqlx::query_as(&query)
                .bind(&dashboard_id)
                .fetch_one(&app.pool)
                .await
                .expect("cascade count");
            assert_eq!(count, 0, "{table} rows did not cascade");
        }
    }

    /* ---------------------------------------------------------------- */
    /* Dashboard tile grouping                                           */
    /* ---------------------------------------------------------------- */

    /// A dashboard with `count` debug-connector placements on it, and the ids
    /// needed to talk about them.
    ///
    /// Each placement gets a distinct position and size, because the whole
    /// claim of the grouping model is that a member's own geometry survives
    /// being grouped — identical boxes would let a bug that resets them pass.
    async fn dashboard_with_placements(
        app: &TestApp,
        token: &str,
        count: usize,
    ) -> (String, Vec<String>) {
        let (status, created) = send(
            &app.router,
            post_json_auth(
                "/dashboards",
                token,
                serde_json::json!({ "name": "Grouping" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created:#}");
        let dashboard_id = created["id"].as_str().expect("dashboard id").to_owned();

        let mut placement_ids = Vec::with_capacity(count);
        for index in 0..count {
            // A separate connector instance each time: grouping must not care
            // what a member is connected to, and reusing one instance would
            // leave "works across connector types" untested.
            let connector_id =
                create_debug_instance(&app.router, token, &format!("Fixture {index}")).await;
            let (status, placement) = send(
                &app.router,
                post_json_auth(
                    &format!("/dashboards/{dashboard_id}/placements"),
                    token,
                    serde_json::json!({
                        "connectorInstanceId": connector_id,
                        "positionX": index as i64,
                        "positionY": index as i64 * 2,
                        "width": 2 + index as i64,
                        "height": 2,
                    }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{placement:#}");
            assert_eq!(placement["groupId"], serde_json::Value::Null);
            placement_ids.push(placement["id"].as_str().expect("placement id").to_owned());
        }

        (dashboard_id, placement_ids)
    }

    async fn dashboard_detail(app: &TestApp, token: &str, dashboard_id: &str) -> serde_json::Value {
        let (status, detail) = send(
            &app.router,
            get_with_auth(&format!("/dashboards/{dashboard_id}"), &bearer(token)),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{detail:#}");
        detail
    }

    /// The ids in `detail.placements`, i.e. everything currently standalone.
    fn standalone_ids(detail: &serde_json::Value) -> Vec<String> {
        detail["placements"]
            .as_array()
            .expect("placements array")
            .iter()
            .map(|placement| placement["id"].as_str().expect("id").to_owned())
            .collect()
    }

    fn member_ids_of(group: &serde_json::Value) -> Vec<String> {
        group["members"]
            .as_array()
            .expect("members array")
            .iter()
            .map(|member| member["id"].as_str().expect("id").to_owned())
            .collect()
    }

    #[tokio::test]
    async fn placements_can_be_grouped_reordered_and_split_apart() {
        let app = test_app().await;
        let (owner, _) = setup_and_login(&app.router).await;
        let (dashboard_id, placements) = dashboard_with_placements(&app, &owner, 3).await;

        // Geometry before grouping, so the round trip can be checked exactly.
        let before = dashboard_detail(&app, &owner, &dashboard_id).await;
        let original: std::collections::HashMap<String, serde_json::Value> = before["placements"]
            .as_array()
            .expect("placements")
            .iter()
            .map(|placement| {
                (
                    placement["id"].as_str().expect("id").to_owned(),
                    serde_json::json!([
                        placement["positionX"],
                        placement["positionY"],
                        placement["width"],
                        placement["height"],
                    ]),
                )
            })
            .collect();
        assert_eq!(original.len(), 3);

        // Three members, not two: nothing in the model may assume a pair.
        let (status, group) = send(
            &app.router,
            post_json_auth(
                &format!("/dashboards/{dashboard_id}/placement-groups"),
                &owner,
                serde_json::json!({
                    "placementIds": [&placements[2], &placements[0], &placements[1]],
                    "name": "Infrastructure",
                    "icon": "lucide:network",
                    "positionX": 0,
                    "positionY": 0,
                    "width": 6,
                    "height": 3,
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{group:#}");
        let group_id = group["id"].as_str().expect("group id").to_owned();
        assert_eq!(group["name"], "Infrastructure");
        assert_eq!(group["icon"], "lucide:network");
        // Member order is the order the request listed, not creation order.
        assert_eq!(
            member_ids_of(&group),
            vec![
                placements[2].clone(),
                placements[0].clone(),
                placements[1].clone()
            ]
        );
        assert_eq!(group["width"], 6);

        // The detail response separates the two kinds of tile.
        let detail = dashboard_detail(&app, &owner, &dashboard_id).await;
        assert!(
            standalone_ids(&detail).is_empty(),
            "every placement is grouped, so none should be listed standalone: {detail:#}"
        );
        let groups = detail["placementGroups"].as_array().expect("groups");
        assert_eq!(groups.len(), 1);
        assert_eq!(
            member_ids_of(&groups[0]),
            vec![
                placements[2].clone(),
                placements[0].clone(),
                placements[1].clone()
            ]
        );
        // Each member still carries its own untouched geometry, and says which
        // group it is in.
        for member in groups[0]["members"].as_array().expect("members") {
            let id = member["id"].as_str().expect("id");
            assert_eq!(member["groupId"], group_id.as_str());
            assert_eq!(
                serde_json::json!([
                    member["positionX"],
                    member["positionY"],
                    member["width"],
                    member["height"],
                ]),
                original[id],
                "grouping must not disturb a member's own geometry"
            );
        }

        // Reorder, and move the tile in the same request.
        let (status, reordered) = send(
            &app.router,
            patch_json_auth(
                &format!("/dashboards/{dashboard_id}/placement-groups/{group_id}"),
                &owner,
                serde_json::json!({
                    "name": "Core services",
                    "icon": null,
                    "positionX": 2,
                    "height": 4,
                    "memberOrder": [&placements[1], &placements[2], &placements[0]],
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{reordered:#}");
        assert_eq!(reordered["positionX"], 2);
        assert_eq!(reordered["name"], "Core services");
        assert_eq!(reordered["icon"], serde_json::Value::Null);
        assert_eq!(reordered["height"], 4);
        assert_eq!(reordered["width"], 6, "an absent field must be left alone");
        assert_eq!(
            member_ids_of(&reordered),
            vec![
                placements[1].clone(),
                placements[2].clone(),
                placements[0].clone()
            ]
        );

        // The reorder is persisted, not just reflected in the response body.
        let detail = dashboard_detail(&app, &owner, &dashboard_id).await;
        assert_eq!(
            member_ids_of(&detail["placementGroups"][0]),
            vec![
                placements[1].clone(),
                placements[2].clone(),
                placements[0].clone()
            ]
        );

        // Explicit split: every member returns to standalone at once.
        let (status, _) = send(
            &app.router,
            delete_auth(
                &format!("/dashboards/{dashboard_id}/placement-groups/{group_id}"),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let detail = dashboard_detail(&app, &owner, &dashboard_id).await;
        assert!(detail["placementGroups"]
            .as_array()
            .expect("groups")
            .is_empty());
        let mut restored = standalone_ids(&detail);
        restored.sort();
        let mut expected = placements.clone();
        expected.sort();
        assert_eq!(restored, expected, "no placement may be lost by ungrouping");

        // Lossless: every placement is back exactly where it was.
        for placement in detail["placements"].as_array().expect("placements") {
            let id = placement["id"].as_str().expect("id");
            assert_eq!(placement["groupId"], serde_json::Value::Null);
            assert_eq!(
                serde_json::json!([
                    placement["positionX"],
                    placement["positionY"],
                    placement["width"],
                    placement["height"],
                ]),
                original[id],
                "ungrouping must restore the preserved geometry exactly"
            );
        }
    }

    #[tokio::test]
    async fn a_group_refuses_membership_it_cannot_honour() {
        let app = test_app().await;
        let (owner, _) = setup_and_login(&app.router).await;
        let (dashboard_id, placements) = dashboard_with_placements(&app, &owner, 3).await;
        let groups_url = format!("/dashboards/{dashboard_id}/placement-groups");
        let box_fields =
            serde_json::json!({ "positionX": 0, "positionY": 0, "width": 4, "height": 2 });
        let with_ids = |ids: serde_json::Value| {
            let mut body = box_fields.clone();
            body["placementIds"] = ids;
            body
        };

        // A group of one is the placement it contains.
        let (status, body) = send(
            &app.router,
            post_json_auth(
                &groups_url,
                &owner,
                with_ids(serde_json::json!([&placements[0]])),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        assert!(body["error"]
            .as_str()
            .is_some_and(|error| error.contains("at least 2")));

        // ...and neither is a group of one placement listed twice.
        let (status, body) = send(
            &app.router,
            post_json_auth(
                &groups_url,
                &owner,
                with_ids(serde_json::json!([&placements[0], &placements[0]])),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        assert!(body["error"]
            .as_str()
            .is_some_and(|error| error.contains("repeats")));

        // An id that is not a placement on this dashboard.
        let (status, body) = send(
            &app.router,
            post_json_auth(
                &groups_url,
                &owner,
                with_ids(serde_json::json!([&placements[0], "not-a-placement"])),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        assert!(body["error"]
            .as_str()
            .is_some_and(|error| error.contains("not-a-placement")));

        // A degenerate box.
        let (status, body) = send(
            &app.router,
            post_json_auth(
                &groups_url,
                &owner,
                serde_json::json!({
                    "placementIds": [&placements[0], &placements[1]],
                    "positionX": 0,
                    "positionY": 0,
                    "width": 0,
                    "height": 2,
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");

        // Now make a real group, and try to take a member of it into another.
        let (status, first) = send(
            &app.router,
            post_json_auth(
                &groups_url,
                &owner,
                with_ids(serde_json::json!([&placements[0], &placements[1]])),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{first:#}");
        let first_group = first["id"].as_str().expect("group id").to_owned();

        let (status, body) = send(
            &app.router,
            post_json_auth(
                &groups_url,
                &owner,
                with_ids(serde_json::json!([&placements[1], &placements[2]])),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        let error = body["error"].as_str().expect("error");
        assert!(
            error.contains(&placements[1]) && error.contains("already in a group"),
            "the refusal must name the offending placement: {error}"
        );
        assert!(
            !error.contains(&placements[2]),
            "an innocent id must not be named: {error}"
        );

        // The same rule on the add-member endpoint: a placement in another
        // group must be ungrouped first, never silently moved.
        let (status, second) = send(
            &app.router,
            post_json_auth(
                &groups_url,
                &owner,
                serde_json::json!({
                    "placementIds": [&placements[2], &placements[0]],
                    "positionX": 0,
                    "positionY": 4,
                    "width": 4,
                    "height": 2,
                }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "placements[0] is spoken for: {second:#}"
        );

        let (status, body) = send(
            &app.router,
            post_json_auth(
                &format!("{groups_url}/{first_group}/members"),
                &owner,
                serde_json::json!({ "placementId": &placements[1] }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");

        // memberOrder must name exactly the current membership.
        for bad_order in [
            serde_json::json!([&placements[0]]),
            serde_json::json!([&placements[0], &placements[0]]),
            serde_json::json!([&placements[0], &placements[2]]),
        ] {
            let (status, body) = send(
                &app.router,
                patch_json_auth(
                    &format!("{groups_url}/{first_group}"),
                    &owner,
                    serde_json::json!({ "memberOrder": bad_order }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        }

        // An unknown group id is a 404 on every group-scoped route.
        let missing = format!("{groups_url}/00000000-0000-4000-8000-0000000000ff");
        for request in [
            patch_json_auth(&missing, &owner, serde_json::json!({ "width": 3 })),
            post_json_auth(
                &format!("{missing}/members"),
                &owner,
                serde_json::json!({ "placementId": &placements[2] }),
            ),
            delete_auth(&missing, &owner),
            delete_auth(&format!("{missing}/members/{}", placements[2]), &owner),
        ] {
            let (status, body) = send(&app.router, request).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{body:#}");
        }
    }

    /// The auto-dissolve cascade, which is the least obvious behaviour here:
    /// removing one member of a pair destroys the tile and un-groups the
    /// placement that was *not* named in the request.
    #[tokio::test]
    async fn removing_a_member_from_a_pair_dissolves_the_whole_group() {
        let app = test_app().await;
        let (owner, _) = setup_and_login(&app.router).await;
        let (dashboard_id, placements) = dashboard_with_placements(&app, &owner, 2).await;
        let groups_url = format!("/dashboards/{dashboard_id}/placement-groups");

        let before = dashboard_detail(&app, &owner, &dashboard_id).await;
        let original = before["placements"].clone();

        let (status, group) = send(
            &app.router,
            post_json_auth(
                &groups_url,
                &owner,
                serde_json::json!({
                    "placementIds": [&placements[0], &placements[1]],
                    "positionX": 1,
                    "positionY": 1,
                    "width": 4,
                    "height": 2,
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{group:#}");
        let group_id = group["id"].as_str().expect("group id").to_owned();

        // Remove one. The other was not mentioned and is un-grouped anyway.
        let (status, _) = send(
            &app.router,
            delete_auth(
                &format!("{groups_url}/{group_id}/members/{}", placements[0]),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let detail = dashboard_detail(&app, &owner, &dashboard_id).await;
        assert!(
            detail["placementGroups"]
                .as_array()
                .expect("groups")
                .is_empty(),
            "a group of one must not survive: {detail:#}"
        );
        let mut restored = standalone_ids(&detail);
        restored.sort();
        let mut expected = placements.clone();
        expected.sort();
        assert_eq!(restored, expected);

        // Both are back exactly as they were, including the one that stayed
        // behind — its geometry was preserved through a membership it never
        // asked to leave.
        // Compared as whole objects, not field by field: anything the round
        // trip disturbed shows up, including a field this test does not know
        // to look at.
        fn by_id(list: &mut serde_json::Value) {
            list.as_array_mut()
                .expect("array")
                .sort_by(|left, right| left["id"].as_str().cmp(&right["id"].as_str()));
        }
        let mut after = detail["placements"].clone();
        let mut before_sorted = original;
        by_id(&mut after);
        by_id(&mut before_sorted);
        assert_eq!(after, before_sorted);

        // The group row itself is gone, not merely emptied.
        let (groups_left,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM dashboard_placement_groups WHERE dashboard_id = ?",
        )
        .bind(&dashboard_id)
        .fetch_one(&app.pool)
        .await
        .expect("group count");
        assert_eq!(groups_left, 0);
    }

    #[tokio::test]
    async fn removing_a_member_from_a_trio_leaves_the_group_standing() {
        let app = test_app().await;
        let (owner, _) = setup_and_login(&app.router).await;
        let (dashboard_id, placements) = dashboard_with_placements(&app, &owner, 4).await;
        let groups_url = format!("/dashboards/{dashboard_id}/placement-groups");

        let (status, group) = send(
            &app.router,
            post_json_auth(
                &groups_url,
                &owner,
                serde_json::json!({
                    "placementIds": [&placements[0], &placements[1], &placements[2]],
                    "positionX": 0,
                    "positionY": 0,
                    "width": 6,
                    "height": 2,
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{group:#}");
        let group_id = group["id"].as_str().expect("group id").to_owned();

        // Three down to two: still a group.
        let (status, _) = send(
            &app.router,
            delete_auth(
                &format!("{groups_url}/{group_id}/members/{}", placements[1]),
                &owner,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let detail = dashboard_detail(&app, &owner, &dashboard_id).await;
        let groups = detail["placementGroups"].as_array().expect("groups");
        assert_eq!(groups.len(), 1, "{detail:#}");
        assert_eq!(
            member_ids_of(&groups[0]),
            vec![placements[0].clone(), placements[2].clone()],
            "the survivors keep their relative order"
        );
        let mut standalone = standalone_ids(&detail);
        standalone.sort();
        let mut expected = vec![placements[1].clone(), placements[3].clone()];
        expected.sort();
        assert_eq!(standalone, expected);

        // Add the removed one back — appended last, not restored to the middle.
        let (status, grown) = send(
            &app.router,
            post_json_auth(
                &format!("{groups_url}/{group_id}/members"),
                &owner,
                serde_json::json!({ "placementId": &placements[1] }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{grown:#}");
        assert_eq!(
            member_ids_of(&grown),
            vec![
                placements[0].clone(),
                placements[2].clone(),
                placements[1].clone()
            ],
            "a removal leaves a gap in group_order; appending must clear it"
        );

        // Two down to one *by the same route* does dissolve.
        for placement in [&placements[1], &placements[2]] {
            let (status, _) = send(
                &app.router,
                delete_auth(
                    &format!("{groups_url}/{group_id}/members/{placement}"),
                    &owner,
                ),
            )
            .await;
            assert_eq!(status, StatusCode::NO_CONTENT);
        }
        let detail = dashboard_detail(&app, &owner, &dashboard_id).await;
        assert!(detail["placementGroups"]
            .as_array()
            .expect("groups")
            .is_empty());
        assert_eq!(standalone_ids(&detail).len(), 4);
    }

    /// A group can lose a member without anyone touching a group endpoint. The
    /// below-two rule has to hold on those routes too, or one-member groups
    /// accumulate on real dashboards.
    #[tokio::test]
    async fn deleting_a_placement_or_its_connector_also_dissolves_an_undersized_group() {
        let app = test_app().await;
        let (owner, _) = setup_and_login(&app.router).await;

        for route in ["placement", "connector"] {
            let (dashboard_id, placements) = dashboard_with_placements(&app, &owner, 2).await;
            let (status, group) = send(
                &app.router,
                post_json_auth(
                    &format!("/dashboards/{dashboard_id}/placement-groups"),
                    &owner,
                    serde_json::json!({
                        "placementIds": [&placements[0], &placements[1]],
                        "positionX": 0,
                        "positionY": 0,
                        "width": 4,
                        "height": 2,
                    }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{group:#}");

            let detail = dashboard_detail(&app, &owner, &dashboard_id).await;
            let connector_id = detail["placementGroups"][0]["members"][0]["connector"]["id"]
                .as_str()
                .expect("connector id")
                .to_owned();

            let request = if route == "placement" {
                delete_auth(
                    &format!("/dashboards/{dashboard_id}/placements/{}", placements[0]),
                    &owner,
                )
            } else {
                delete_auth(&format!("/connector-instances/{connector_id}"), &owner)
            };
            let (status, _) = send(&app.router, request).await;
            assert_eq!(status, StatusCode::NO_CONTENT, "deleting via {route}");

            let detail = dashboard_detail(&app, &owner, &dashboard_id).await;
            assert!(
                detail["placementGroups"]
                    .as_array()
                    .expect("groups")
                    .is_empty(),
                "deleting via {route} left an undersized group: {detail:#}"
            );
            assert_eq!(
                standalone_ids(&detail).len(),
                1,
                "the surviving placement must be standalone again: {detail:#}"
            );
        }
    }

    #[tokio::test]
    async fn placement_group_endpoints_need_the_editor_role() {
        let app = test_app().await;
        let (owner, _) = setup_and_login(&app.router).await;
        let (dashboard_id, placements) = dashboard_with_placements(&app, &owner, 3).await;
        let groups_url = format!("/dashboards/{dashboard_id}/placement-groups");

        let editor = user_with_grants(&app.router, &owner, "editor", serde_json::json!([])).await;
        let viewer = user_with_grants(&app.router, &owner, "viewer", serde_json::json!([])).await;
        for (token, role) in [(&editor, "edit"), (&viewer, "view")] {
            let target_id = current_user_id(&app.router, token).await;
            let (status, share) = send(
                &app.router,
                post_json_auth(
                    &format!("/dashboards/{dashboard_id}/shares"),
                    &owner,
                    serde_json::json!({
                        "targetType": "user",
                        "targetId": target_id,
                        "role": role,
                    }),
                ),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{share:#}");
        }

        // The editor can do all of it. Dashboard grouping is an ACL question,
        // not an RBAC one: this user holds no `connectors.*` grant at all.
        let (status, group) = send(
            &app.router,
            post_json_auth(
                &groups_url,
                &editor,
                serde_json::json!({
                    "placementIds": [&placements[0], &placements[1]],
                    "positionX": 0,
                    "positionY": 0,
                    "width": 4,
                    "height": 2,
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{group:#}");
        let group_id = group["id"].as_str().expect("group id").to_owned();

        // The viewer can do none of it, and the refusal is 403 rather than 404
        // — they can see this group, they simply may not change it.
        for request in [
            post_json_auth(
                &groups_url,
                &viewer,
                serde_json::json!({
                    "placementIds": [&placements[0], &placements[2]],
                    "positionX": 0,
                    "positionY": 0,
                    "width": 4,
                    "height": 2,
                }),
            ),
            patch_json_auth(
                &format!("{groups_url}/{group_id}"),
                &viewer,
                serde_json::json!({ "width": 5 }),
            ),
            post_json_auth(
                &format!("{groups_url}/{group_id}/members"),
                &viewer,
                serde_json::json!({ "placementId": &placements[2] }),
            ),
            delete_auth(
                &format!("{groups_url}/{group_id}/members/{}", placements[0]),
                &viewer,
            ),
            delete_auth(&format!("{groups_url}/{group_id}"), &viewer),
        ] {
            let (status, body) = send(&app.router, request).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{body:#}");
        }

        // A viewer still *reads* the grouping, and nothing above changed it.
        let detail = dashboard_detail(&app, &viewer, &dashboard_id).await;
        let groups = detail["placementGroups"].as_array().expect("groups");
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["width"], 4);
        assert_eq!(
            member_ids_of(&groups[0]),
            vec![placements[0].clone(), placements[1].clone()]
        );

        // The editor finishes what the viewer could not start.
        let (status, _) = send(
            &app.router,
            delete_auth(&format!("{groups_url}/{group_id}"), &editor),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
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
        let id = create_debug_instance(&app.router, &admin, "Fixture").await;
        let nobody = user_with_grants(&app.router, &admin, "nobody", serde_json::json!([])).await;

        for request in [
            get_with_auth("/connector-types", &bearer(&nobody)),
            get_with_auth("/connector-instances", &bearer(&nobody)),
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
                &format!("/connector-instances/{id}/actions/ping"),
                &nobody,
                serde_json::json!({}),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    /// The three cases the resource-scoped check has to get right — now scoped
    /// to a real instance id rather than one hardcoded connector.
    #[tokio::test]
    async fn connector_action_respects_scoped_global_and_absent_grants() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &admin, "Fixture").await;
        let other = create_debug_instance(&app.router, &admin, "Other").await;

        // 1. A grant naming exactly this instance.
        let scoped = user_with_grants(
            &app.router,
            &admin,
            "scoped",
            serde_json::json!([{
                "key": "connectors.control",
                "resourceType": "connector",
                "resourceId": id,
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

        let act = |instance: &str, token: &str| {
            post_json_auth(
                &format!("/connector-instances/{instance}/actions/ping"),
                token,
                serde_json::json!({}),
            )
        };

        let (status, body) = send(&app.router, act(&id, &scoped)).await;
        assert_eq!(status, StatusCode::OK, "scoped grant must work: {body:#}");

        let (status, body) = send(&app.router, act(&id, &global)).await;
        assert_eq!(status, StatusCode::OK, "global grant must work: {body:#}");

        // The whole point of scoping: a grant for one instance authorizes
        // nothing on another, even of the same type.
        let (status, _) = send(&app.router, act(&other, &scoped)).await;
        assert_eq!(status, StatusCode::FORBIDDEN);

        // And an instance-scoped grant is not authority over connectors at
        // large, so the global-only list endpoint still refuses it.
        let (status, _) = send(
            &app.router,
            get_with_auth("/connector-instances", &bearer(&scoped)),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
    }

    /// A 403 must not depend on whether the resource exists, or the endpoint
    /// becomes a way to enumerate configured connectors.
    #[tokio::test]
    async fn an_unauthorized_action_does_not_reveal_whether_the_connector_exists() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let id = create_debug_instance(&app.router, &admin, "Fixture").await;
        let nobody = user_with_grants(&app.router, &admin, "nobody", serde_json::json!([])).await;

        let (real, real_body) = send(
            &app.router,
            post_json_auth(
                &format!("/connector-instances/{id}/actions/ping"),
                &nobody,
                serde_json::json!({}),
            ),
        )
        .await;
        let (fake, fake_body) = send(
            &app.router,
            post_json_auth(
                "/connector-instances/00000000-0000-4000-8000-00000000ffff/actions/ping",
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
                "connectors.manage",
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

    /* ---------------------------------------------------------------- */
    /* Self-service account                                              */
    /* ---------------------------------------------------------------- */

    /// Boundary used by the multipart helpers. Fixed rather than generated:
    /// these bodies are built and parsed in the same process, so there is
    /// nothing to collide with.
    const BOUNDARY: &str = "loomtestboundary";

    /// A multipart request carrying one file field named `file`.
    ///
    /// `content_type` is deliberately a parameter: several tests assert that it
    /// is *not* what the server decides by.
    fn upload_request(
        uri: &str,
        token: &str,
        filename: &str,
        content_type: &str,
        data: &[u8],
    ) -> Request<Body> {
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{BOUNDARY}\r\n\
                 Content-Disposition: form-data; name=\"file\"; filename=\"{filename}\"\r\n\
                 Content-Type: {content_type}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{BOUNDARY}--\r\n").as_bytes());

        Request::builder()
            .method("POST")
            .uri(uri)
            .header(
                "content-type",
                format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .header("authorization", format!("Bearer {token}"))
            .body(Body::from(body))
            .expect("valid request")
    }

    /// A real, valid, tiny PNG — encoded rather than pasted in as a blob, so it
    /// is obvious what it is and it cannot rot into something unreadable.
    fn tiny_png() -> Vec<u8> {
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::new(4, 4))
            .write_to(&mut bytes, image::ImageFormat::Png)
            .expect("encoding a 4x4 PNG must succeed");
        bytes.into_inner()
    }

    #[tokio::test]
    async fn account_returns_the_callers_own_profile_with_groups() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (status, body) = send(&app.router, get_with_auth("/account", &bearer(&access))).await;

        assert_eq!(status, StatusCode::OK, "{body:#}");
        assert_eq!(body["username"], "admin");
        assert_eq!(body["displayName"], serde_json::Value::Null);
        assert_eq!(body["avatarUrl"], serde_json::Value::Null);
        assert!(body["createdAt"].is_string());

        // The seeded Administrators group, named rather than just identified.
        let groups = body["groups"].as_array().expect("groups array");
        assert_eq!(groups.len(), 1, "{body:#}");
        assert_eq!(groups[0]["name"], "Administrators");
        assert!(groups[0]["id"].is_string());

        // The hash must not be anywhere in this response, under any key.
        assert!(
            !body.to_string().contains("$argon2"),
            "a password hash reached the account response: {body:#}"
        );
    }

    #[tokio::test]
    async fn account_routes_reject_an_anonymous_caller() {
        let app = test_app().await;
        setup_and_login(&app.router).await;

        for request in [
            get("/account"),
            Request::builder()
                .method("PATCH")
                .uri("/account")
                .header("content-type", "application/json")
                .body(Body::from("{}"))
                .expect("valid request"),
            post_json("/account/password", serde_json::json!({})),
            Request::builder()
                .method("DELETE")
                .uri("/account/avatar")
                .body(Body::empty())
                .expect("valid request"),
        ] {
            let uri = request.uri().to_string();
            let (status, _) = send(&app.router, request).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri} allowed no token");
        }
    }

    #[tokio::test]
    async fn a_user_can_rename_themselves_and_set_a_display_name() {
        let app = test_app().await;
        let (access, refresh) = setup_and_login(&app.router).await;

        let (status, body) = send(
            &app.router,
            patch_json_auth(
                "/account",
                &access,
                serde_json::json!({ "username": "  renamed  ", "displayName": "The Admin" }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body:#}");
        // Trimmed on the way in, like every other username in the API.
        assert_eq!(body["username"], "renamed");
        assert_eq!(body["displayName"], "The Admin");

        // The old name is genuinely free now, and the new one genuinely works.
        let (status, _) = send(
            &app.router,
            post_json(
                "/auth/login",
                serde_json::json!({ "username": "renamed", "password": "a-good-password" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "the new username must authenticate");

        // The already-issued token keeps its stale `username` claim and still
        // works, because handlers key off `sub`. This is the documented
        // trade-off in `update_account`, asserted so a change to it is visible.
        let (status, body) = send(&app.router, get_with_auth("/account", &bearer(&access))).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["username"], "renamed", "the row is what is read");

        // And a refresh picks the new name up.
        let (status, tokens) = send(
            &app.router,
            post_json(
                "/auth/refresh",
                serde_json::json!({ "refreshToken": refresh }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{tokens:#}");
        let (status, session) = send(
            &app.router,
            get_with_auth(
                "/auth/session",
                &bearer(tokens["accessToken"].as_str().expect("accessToken")),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(session["username"], "renamed");
    }

    #[tokio::test]
    async fn renaming_yourself_to_a_taken_username_conflicts() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        let other = user_with_grants(&app.router, &admin, "housemate", serde_json::json!([])).await;

        let (status, body) = send(
            &app.router,
            patch_json_auth(
                "/account",
                &other,
                serde_json::json!({ "username": "admin" }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::CONFLICT, "{body:#}");
        assert!(
            body["error"].as_str().expect("error").contains("admin"),
            "the message should name the taken username: {body:#}"
        );

        // Refused means unchanged, not partially applied.
        let (_, account) = send(&app.router, get_with_auth("/account", &bearer(&other))).await;
        assert_eq!(account["username"], "housemate");
    }

    #[tokio::test]
    async fn keeping_your_own_username_is_not_a_conflict() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        // The uniqueness check excludes self; without that exclusion, sending
        // the current username back — which any "save profile" form does —
        // would collide with the caller's own row.
        let (status, body) = send(
            &app.router,
            patch_json_auth(
                "/account",
                &access,
                serde_json::json!({ "username": "admin", "displayName": "Still Admin" }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body:#}");
        assert_eq!(body["displayName"], "Still Admin");
    }

    #[tokio::test]
    async fn a_display_name_can_be_cleared_and_whitespace_does_not_count() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (_, body) = send(
            &app.router,
            patch_json_auth(
                "/account",
                &access,
                serde_json::json!({ "displayName": "Named" }),
            ),
        )
        .await;
        assert_eq!(body["displayName"], "Named");

        // Explicit null clears it.
        let (status, body) = send(
            &app.router,
            patch_json_auth(
                "/account",
                &access,
                serde_json::json!({ "displayName": null }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["displayName"], serde_json::Value::Null);

        // So does whitespace, which is not a name.
        send(
            &app.router,
            patch_json_auth(
                "/account",
                &access,
                serde_json::json!({ "displayName": "Named" }),
            ),
        )
        .await;
        let (_, body) = send(
            &app.router,
            patch_json_auth(
                "/account",
                &access,
                serde_json::json!({ "displayName": "   " }),
            ),
        )
        .await;
        assert_eq!(body["displayName"], serde_json::Value::Null);

        // An absent field leaves it alone, rather than clearing it.
        send(
            &app.router,
            patch_json_auth(
                "/account",
                &access,
                serde_json::json!({ "displayName": "Kept" }),
            ),
        )
        .await;
        let (_, body) = send(
            &app.router,
            patch_json_auth(
                "/account",
                &access,
                serde_json::json!({ "username": "admin" }),
            ),
        )
        .await;
        assert_eq!(body["displayName"], "Kept");
    }

    #[tokio::test]
    async fn a_rename_cannot_be_pointed_at_another_account() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let other = user_with_grants(&app.router, &admin, "housemate", serde_json::json!([])).await;

        // There is no id parameter to supply, so the only way to try is to send
        // one in the body and hope it is honoured. It must be ignored: the
        // subject comes from the token.
        let (status, body) = send(
            &app.router,
            patch_json_auth(
                "/account",
                &other,
                serde_json::json!({ "id": "00000000-0000-4000-8000-000000000009", "username": "hijacked" }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body:#}");
        assert_eq!(body["username"], "hijacked");

        // The admin is untouched.
        let (_, admin_account) =
            send(&app.router, get_with_auth("/account", &bearer(&admin))).await;
        assert_eq!(admin_account["username"], "admin");
    }

    #[tokio::test]
    async fn changing_a_password_requires_the_current_one() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (status, body) = send(
            &app.router,
            post_json_auth(
                "/account/password",
                &access,
                serde_json::json!({
                    "currentPassword": "not-the-password",
                    "newPassword": "a-better-password",
                }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body:#}");
        // A distinct message from the login 401, so a client can say which
        // field is wrong rather than "sign in failed".
        assert_eq!(body["error"], "current password is incorrect");

        // Nothing changed: the original password still works.
        let (status, _) = send(
            &app.router,
            post_json(
                "/auth/login",
                serde_json::json!({ "username": "admin", "password": "a-good-password" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_correct_current_password_changes_it() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (status, _) = send(
            &app.router,
            post_json_auth(
                "/account/password",
                &access,
                serde_json::json!({
                    "currentPassword": "a-good-password",
                    "newPassword": "an-even-better-password",
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, _) = send(
            &app.router,
            post_json(
                "/auth/login",
                serde_json::json!({ "username": "admin", "password": "an-even-better-password" }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "the new password must authenticate");

        let (status, _) = send(
            &app.router,
            post_json(
                "/auth/login",
                serde_json::json!({ "username": "admin", "password": "a-good-password" }),
            ),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "the old password must stop working"
        );
    }

    #[tokio::test]
    async fn a_new_password_faces_the_same_floor_as_setup() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (status, body) = send(
            &app.router,
            post_json_auth(
                "/account/password",
                &access,
                serde_json::json!({ "currentPassword": "a-good-password", "newPassword": "short" }),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        assert!(body["error"]
            .as_str()
            .expect("error")
            .contains(&auth::password::MIN_PASSWORD_LENGTH.to_string()));
    }

    #[tokio::test]
    async fn an_avatar_upload_stores_a_file_and_reports_its_url() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        assert!(app.avatar_files().is_empty(), "starts with no avatars");

        let (status, body) = send(
            &app.router,
            upload_request(
                "/account/avatar",
                &access,
                "me.png",
                "image/png",
                &tiny_png(),
            ),
        )
        .await;

        assert_eq!(status, StatusCode::OK, "{body:#}");
        let url = body["avatarUrl"].as_str().expect("avatarUrl").to_owned();
        assert!(url.starts_with("/avatars/"), "{url}");
        assert!(url.ends_with(".png"), "{url}");
        // Never derived from the upload's own filename.
        assert!(!url.contains("me.png"), "{url}");

        let files = app.avatar_files();
        assert_eq!(files.len(), 1, "{files:?}");
        assert!(url.ends_with(&files[0]), "url {url} vs file {}", files[0]);

        // And it is reachable through the static service, which is the whole
        // point of storing it.
        let (status, _) = send(&app.router, get(&url)).await;
        assert_eq!(status, StatusCode::OK, "the avatar must be served at {url}");

        let (_, account) = send(&app.router, get_with_auth("/account", &bearer(&access))).await;
        assert_eq!(account["avatarUrl"], url);
    }

    #[tokio::test]
    async fn a_replacement_avatar_deletes_the_previous_file() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (_, first) = send(
            &app.router,
            upload_request(
                "/account/avatar",
                &access,
                "a.png",
                "image/png",
                &tiny_png(),
            ),
        )
        .await;
        let first_url = first["avatarUrl"].as_str().expect("avatarUrl").to_owned();
        let first_files = app.avatar_files();
        assert_eq!(first_files.len(), 1);

        let (status, second) = send(
            &app.router,
            upload_request(
                "/account/avatar",
                &access,
                "b.png",
                "image/png",
                &tiny_png(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{second:#}");
        let second_url = second["avatarUrl"].as_str().expect("avatarUrl").to_owned();

        // A fresh name every time, so a cached URL cannot show the old picture.
        assert_ne!(first_url, second_url);

        // Exactly one file: the replaced one was removed rather than orphaned.
        let files = app.avatar_files();
        assert_eq!(files.len(), 1, "orphaned avatar files: {files:?}");
        assert!(second_url.ends_with(&files[0]));

        // The old URL now 404s, which is what "deleted from disk" means to a
        // client.
        let (status, _) = send(&app.router, get(&first_url)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn deleting_an_avatar_removes_the_file_and_clears_the_field() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let (_, uploaded) = send(
            &app.router,
            upload_request(
                "/account/avatar",
                &access,
                "a.png",
                "image/png",
                &tiny_png(),
            ),
        )
        .await;
        let url = uploaded["avatarUrl"]
            .as_str()
            .expect("avatarUrl")
            .to_owned();
        assert_eq!(app.avatar_files().len(), 1);

        let (status, body) = send(&app.router, delete_auth("/account/avatar", &access)).await;

        assert_eq!(status, StatusCode::OK, "{body:#}");
        // Returns the updated profile, not just an acknowledgement.
        assert_eq!(body["avatarUrl"], serde_json::Value::Null);
        assert_eq!(body["username"], "admin");

        assert!(
            app.avatar_files().is_empty(),
            "the file must be gone: {:?}",
            app.avatar_files()
        );

        let (status, _) = send(&app.router, get(&url)).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // Deleting again is not an error — the end state is the one asked for.
        let (status, _) = send(&app.router, delete_auth("/account/avatar", &access)).await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test]
    async fn a_non_image_upload_is_rejected_whatever_it_claims_to_be() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        // Announced as a PNG, and it is not one. The content-type header is
        // caller-supplied, so only the bytes get a vote.
        let (status, body) = send(
            &app.router,
            upload_request(
                "/account/avatar",
                &access,
                "payload.png",
                "image/png",
                b"#!/bin/sh\necho not an image\n",
            ),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        assert!(app.avatar_files().is_empty(), "nothing may be written");

        let (_, account) = send(&app.router, get_with_auth("/account", &bearer(&access))).await;
        assert_eq!(account["avatarUrl"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn a_truncated_image_is_rejected() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        // A real PNG header followed by nothing usable. This is what a header
        // sniff alone would wave through, and why the file is decoded in full.
        let png = tiny_png();
        let truncated = &png[..png.len() / 2];

        let (status, body) = send(
            &app.router,
            upload_request(
                "/account/avatar",
                &access,
                "half.png",
                "image/png",
                truncated,
            ),
        )
        .await;

        assert_eq!(status, StatusCode::BAD_REQUEST, "{body:#}");
        assert!(app.avatar_files().is_empty());
    }

    #[tokio::test]
    async fn an_oversized_avatar_is_rejected() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let too_big = vec![0u8; routes::account::MAX_AVATAR_BYTES + 1];

        let (status, body) = send(
            &app.router,
            upload_request("/account/avatar", &access, "big.png", "image/png", &too_big),
        )
        .await;

        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{body:#}");
        assert!(app.avatar_files().is_empty(), "nothing may be written");
    }

    #[tokio::test]
    async fn an_upload_with_no_file_field_is_rejected() {
        let app = test_app().await;
        let (access, _) = setup_and_login(&app.router).await;

        let body = format!(
            "--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"note\"\r\n\r\nhello\r\n--{BOUNDARY}--\r\n"
        );
        let request = Request::builder()
            .method("POST")
            .uri("/account/avatar")
            .header(
                "content-type",
                format!("multipart/form-data; boundary={BOUNDARY}"),
            )
            .header("authorization", bearer(&access))
            .body(Body::from(body))
            .expect("valid request");

        let (status, _) = send(&app.router, request).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn the_avatar_service_does_not_escape_its_directory() {
        let app = test_app().await;
        setup_and_login(&app.router).await;

        // The database sits one level up from the avatar directory. A traversal
        // that worked would hand out password hashes.
        for uri in [
            "/avatars/../loom.db",
            "/avatars/%2e%2e/loom.db",
            "/avatars/..%2floom.db",
        ] {
            let (status, _) = send(&app.router, get(uri)).await;
            assert_ne!(status, StatusCode::OK, "{uri} escaped the avatar directory");
        }
    }

    #[tokio::test]
    async fn one_users_avatar_change_does_not_touch_anothers() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;
        let other = user_with_grants(&app.router, &admin, "housemate", serde_json::json!([])).await;

        let (_, admin_avatar) = send(
            &app.router,
            upload_request("/account/avatar", &admin, "a.png", "image/png", &tiny_png()),
        )
        .await;
        let (_, other_avatar) = send(
            &app.router,
            upload_request("/account/avatar", &other, "b.png", "image/png", &tiny_png()),
        )
        .await;

        // Two accounts, two files: the second upload must not have been treated
        // as a replacement of the first.
        assert_eq!(app.avatar_files().len(), 2);
        assert_ne!(admin_avatar["avatarUrl"], other_avatar["avatarUrl"]);

        // Deleting one leaves the other alone.
        send(&app.router, delete_auth("/account/avatar", &other)).await;

        let (_, admin_account) =
            send(&app.router, get_with_auth("/account", &bearer(&admin))).await;
        assert_eq!(admin_account["avatarUrl"], admin_avatar["avatarUrl"]);
        assert_eq!(app.avatar_files().len(), 1);
    }

    #[tokio::test]
    async fn a_group_description_can_be_cleared_with_an_explicit_null() {
        let app = test_app().await;
        let (admin, _) = setup_and_login(&app.router).await;

        let (status, group) = send(
            &app.router,
            post_json_auth(
                "/groups",
                &admin,
                serde_json::json!({
                    "name": "Viewers",
                    "description": "Read-only access.",
                    "permissions": [],
                }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{group:#}");
        let id = group["id"].as_str().expect("group id").to_owned();

        // Present-and-null must clear. This distinguishes "the caller sent
        // null" from "the caller omitted the field", which a bare
        // `Option<Option<String>>` cannot do — see `routes::present_option`.
        let (status, updated) = send(
            &app.router,
            patch_json_auth(
                &format!("/groups/{id}"),
                &admin,
                serde_json::json!({ "description": null }),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{updated:#}");
        assert_eq!(updated["description"], serde_json::Value::Null);

        // An omitted field still leaves the value alone.
        send(
            &app.router,
            patch_json_auth(
                &format!("/groups/{id}"),
                &admin,
                serde_json::json!({ "description": "Restored." }),
            ),
        )
        .await;
        let (_, updated) = send(
            &app.router,
            patch_json_auth(
                &format!("/groups/{id}"),
                &admin,
                serde_json::json!({ "name": "Viewers" }),
            ),
        )
        .await;
        assert_eq!(updated["description"], "Restored.");
    }
}
