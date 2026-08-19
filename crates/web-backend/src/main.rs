//! The Loom server.
//!
//! This is the single long-running process in the system. Every client
//! (web frontend, desktop, mobile) talks to it over HTTP; it in turn depends on
//! `loom-core` for connector and business logic.
//!
//! Right now it serves exactly one route, `/health`, which is enough to prove
//! the `core -> web-backend` wiring works at runtime rather than only at
//! compile time.
//!
//! Under the non-default `dev-stub-auth` feature it additionally serves the stub
//! auth and connector routes from [`dev_stub_auth`] — see that module and
//! `crates/web-backend/Cargo.toml` for why that must never be a shipped build.

use axum::{http::HeaderValue, routing::get, Json, Router};
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[cfg(feature = "dev-stub-auth")]
mod dev_stub_auth;

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

/// Builds the application router.
///
/// Split out of [`main`] so tests can drive the real routing table through
/// `tower::ServiceExt::oneshot` instead of binding a port — including the tests
/// that assert the `dev-stub-auth` routes are *absent* from a default build.
fn app() -> Router {
    let router = Router::new().route("/health", get(health));

    #[cfg(feature = "dev-stub-auth")]
    let router = router.merge(dev_stub_auth::routes());

    router.layer(cors_layer())
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

    #[cfg(feature = "dev-stub-auth")]
    tracing::warn!(
        "dev-stub-auth is COMPILED IN: /auth/login accepts ANY username and \
         password, and the connector routes require no authentication at all. \
         This build is for local development only and must not be exposed to any \
         network you do not fully control. See docs/API_CONTRACT.md."
    );

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    info!(
        addr = %listener.local_addr()?,
        core_version = loom_core::version(),
        "loom web-backend listening"
    );

    axum::serve(listener, app()).await?;
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

    /// Sends one request through the real router and returns status plus body.
    async fn call(request: Request<Body>) -> (StatusCode, serde_json::Value) {
        let response = app()
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

    fn post_json(uri: &str, body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .expect("valid request")
    }

    #[tokio::test]
    async fn health_reports_ok_and_the_core_version() {
        let (status, body) = call(get("/health")).await;

        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "ok");
        assert_eq!(body["core_version"], loom_core::version());
    }

    /// The load-bearing test for the feature gate: a default build must not
    /// answer the stub routes at all, not even with a 401.
    #[cfg(not(feature = "dev-stub-auth"))]
    mod stub_absent {
        use super::*;

        #[tokio::test]
        async fn login_route_does_not_exist() {
            let (status, _) = call(post_json(
                "/auth/login",
                serde_json::json!({ "username": "anyone", "password": "anything" }),
            ))
            .await;

            assert_eq!(status, StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn session_route_does_not_exist() {
            let (status, _) = call(get("/auth/session")).await;

            assert_eq!(status, StatusCode::NOT_FOUND);
        }

        #[tokio::test]
        async fn connector_routes_do_not_exist() {
            let (list, _) = call(get("/connectors")).await;
            assert_eq!(list, StatusCode::NOT_FOUND);

            let (action, _) = call(post_json(
                "/connectors/mock/actions/restart",
                serde_json::json!({}),
            ))
            .await;
            assert_eq!(action, StatusCode::NOT_FOUND);
        }
    }

    #[cfg(feature = "dev-stub-auth")]
    mod stub_present {
        use super::*;
        use crate::dev_stub_auth::{DEV_STUB_TOKEN, DEV_STUB_USER};

        fn get_with_auth(uri: &str, header: &str) -> Request<Body> {
            Request::builder()
                .uri(uri)
                .header("authorization", header)
                .body(Body::empty())
                .expect("valid request")
        }

        #[tokio::test]
        async fn login_accepts_arbitrary_credentials() {
            let before = chrono::Utc::now();
            let (status, body) = call(post_json(
                "/auth/login",
                serde_json::json!({ "username": "", "password": "hunter2" }),
            ))
            .await;

            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["token"], DEV_STUB_TOKEN);

            let expires_at: chrono::DateTime<chrono::Utc> = body["expiresAt"]
                .as_str()
                .expect("expiresAt must be a string")
                .parse()
                .expect("expiresAt must be RFC 3339");
            assert!(
                expires_at > before,
                "expiresAt {expires_at} must be in the future"
            );

            println!("POST /auth/login -> {status}\n{body:#}");
        }

        #[tokio::test]
        async fn login_rejects_a_body_that_is_not_credentials() {
            let (status, _) = call(post_json("/auth/login", serde_json::json!({ "x": 1 }))).await;

            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        }

        #[tokio::test]
        async fn session_accepts_the_stub_token() {
            let (status, body) = call(get_with_auth(
                "/auth/session",
                &format!("Bearer {DEV_STUB_TOKEN}"),
            ))
            .await;

            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["authenticated"], true);
            assert_eq!(body["user"], DEV_STUB_USER);

            println!("GET /auth/session (valid) -> {status}\n{body:#}");
        }

        #[tokio::test]
        async fn session_rejects_a_wrong_token() {
            let (status, body) = call(get_with_auth("/auth/session", "Bearer not-the-token")).await;

            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert!(body["error"].is_string());

            println!("GET /auth/session (wrong token) -> {status}\n{body:#}");
        }

        #[tokio::test]
        async fn session_rejects_a_missing_header() {
            let (status, body) = call(get("/auth/session")).await;

            assert_eq!(status, StatusCode::UNAUTHORIZED);
            assert!(body["error"].is_string());
        }

        #[tokio::test]
        async fn session_rejects_a_non_bearer_scheme() {
            let (status, _) = call(get_with_auth(
                "/auth/session",
                &format!("Basic {DEV_STUB_TOKEN}"),
            ))
            .await;

            assert_eq!(status, StatusCode::UNAUTHORIZED);
        }

        #[tokio::test]
        async fn connector_list_is_an_array_with_the_mock() {
            let (status, body) = call(get("/connectors")).await;

            assert_eq!(status, StatusCode::OK);
            let entries = body.as_array().expect("the list must be a JSON array");
            assert_eq!(entries.len(), 1);
            assert_eq!(entries[0]["metadata"]["id"], "mock");
            assert_eq!(entries[0]["status"]["health"], "healthy");
            assert!(
                entries[0].get("statusError").is_none(),
                "a healthy connector must not carry statusError"
            );

            // The dashboard renders one button per action, so the list has to
            // carry them; without this a client would have to hardcode ids.
            let actions = entries[0]["actions"]
                .as_array()
                .expect("every entry must carry an actions array");
            let ids: Vec<&str> = actions
                .iter()
                .map(|action| action["id"].as_str().expect("action ids are strings"))
                .collect();
            assert!(ids.contains(&"restart"), "actions were {ids:?}");
            assert!(ids.contains(&"ping"), "actions were {ids:?}");
            assert!(actions[0].get("label").is_some());
            assert!(actions[0].get("paramsSchema").is_some());

            println!("GET /connectors -> {status}\n{body:#}");
        }

        #[tokio::test]
        async fn restart_action_succeeds_and_echoes_its_params() {
            let (status, body) = call(post_json(
                "/connectors/mock/actions/restart",
                serde_json::json!({ "force": true }),
            ))
            .await;

            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["success"], true);
            assert_eq!(
                body["payload"]["params"],
                serde_json::json!({"force": true})
            );

            println!("POST /connectors/mock/actions/restart -> {status}\n{body:#}");
        }

        #[tokio::test]
        async fn ping_action_succeeds_without_a_body() {
            let request = Request::builder()
                .method("POST")
                .uri("/connectors/mock/actions/ping")
                .body(Body::empty())
                .expect("valid request");
            let (status, body) = call(request).await;

            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["success"], true);

            println!("POST /connectors/mock/actions/ping (no body) -> {status}\n{body:#}");
        }

        #[tokio::test]
        async fn unknown_connector_id_is_not_found() {
            let (status, body) = call(post_json(
                "/connectors/nope/actions/ping",
                serde_json::json!({}),
            ))
            .await;

            assert_eq!(status, StatusCode::NOT_FOUND);
            assert!(body["error"].is_string());
            assert!(
                body.get("connectorError").is_none(),
                "an unknown connector never produced a ConnectorError"
            );

            println!("POST /connectors/nope/actions/ping -> {status}\n{body:#}");
        }

        #[tokio::test]
        async fn unknown_action_id_is_not_found() {
            let (status, body) = call(post_json(
                "/connectors/mock/actions/self-destruct",
                serde_json::json!({}),
            ))
            .await;

            assert_eq!(status, StatusCode::NOT_FOUND);
            assert_eq!(
                body["connectorError"],
                serde_json::json!({ "invalidAction": { "actionId": "self-destruct" } })
            );

            println!("POST /connectors/mock/actions/self-destruct -> {status}\n{body:#}");
        }

        #[tokio::test]
        async fn a_malformed_body_is_a_bad_request() {
            let request = Request::builder()
                .method("POST")
                .uri("/connectors/mock/actions/ping")
                .header("content-type", "application/json")
                .body(Body::from("{not json"))
                .expect("valid request");
            let (status, body) = call(request).await;

            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(body["error"].is_string());

            println!("POST .../ping (malformed body) -> {status}\n{body:#}");
        }
    }
}
