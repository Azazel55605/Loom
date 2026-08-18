//! The Loom server.
//!
//! This is the single long-running process in the system. Every client
//! (web frontend, desktop, mobile) talks to it over HTTP; it in turn depends on
//! `loom-core` for connector and business logic.
//!
//! Right now it serves exactly one route, `/health`, which is enough to prove
//! the `core -> web-backend` wiring works at runtime rather than only at
//! compile time.

use axum::{http::HeaderValue, routing::get, Json, Router};
use serde::Serialize;
use tower_http::cors::{Any, CorsLayer};
use tracing::info;
use tracing_subscriber::EnvFilter;

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_env("LOOM_LOG_LEVEL").unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    let bind_addr =
        std::env::var("LOOM_BIND_ADDR").unwrap_or_else(|_| DEFAULT_BIND_ADDR.to_owned());

    let app = Router::new()
        .route("/health", get(health))
        .layer(cors_layer());

    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    info!(
        addr = %listener.local_addr()?,
        core_version = loom_core::version(),
        "loom web-backend listening"
    );

    axum::serve(listener, app).await?;
    Ok(())
}
