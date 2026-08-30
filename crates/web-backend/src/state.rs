//! Shared application state.

use std::path::PathBuf;
use std::sync::Arc;

use sqlx::SqlitePool;

use crate::auth::rate_limit::LoginRateLimiter;
use crate::connectors::config_secrets::ConfigEncryptionKey;
use crate::connectors::{ConnectorRuntime, UpdateCache};

/// Everything a handler needs, cloned per request.
///
/// `SqlitePool` and `Arc` are both cheap to clone — the pool is itself a handle
/// — so this is passed by value rather than behind another layer of sharing.
#[derive(Clone)]
pub struct AppState {
    /// The database. Migrations have already run by the time this exists.
    pub pool: SqlitePool,
    /// HS256 signing secret, read once at startup so no request pays for it.
    pub jwt_secret: Arc<String>,
    /// Per-direct-peer failed-login windows. Intentionally in memory: this is
    /// abuse throttling, not durable account state.
    pub login_rate_limiter: LoginRateLimiter,
    /// Independent AES-256 key for schema-marked connector config fields.
    pub config_encryption_key: Arc<ConfigEncryptionKey>,
    /// Directory holding uploaded avatar files.
    ///
    /// Carried on the state rather than re-resolved per request so a handler
    /// cannot disagree with the static file service about where avatars live —
    /// they are handed the same path at startup.
    pub avatars_dir: Arc<PathBuf>,
    /// The connector types this build registers, and the live connector
    /// instances built from the `connector_instances` table.
    ///
    /// Both halves live here because a handler that creates an instance needs
    /// the registry to validate it and the runtime to install it, in one
    /// request. See `crates/web-backend/src/connectors/mod.rs`.
    pub connectors: ConnectorRuntime,
    /// What the update scheduler last found, per instance and target.
    ///
    /// Beside the connector runtime rather than inside it: the runtime's cache
    /// is a *status* cache refreshed every few seconds from a local daemon,
    /// and this is refreshed every few hours from a third party. Folding them
    /// together would make one lock serve two cadences and invite an update
    /// check onto the status path. See `connectors::updates`.
    pub updates: UpdateCache,
}

impl AppState {
    /// Builds state around an already-migrated pool and a loaded runtime.
    ///
    /// The runtime is passed in rather than constructed here because loading it
    /// reads the database and can fail, and state construction should not be a
    /// fallible async operation buried inside a struct literal.
    pub fn new(
        pool: SqlitePool,
        jwt_secret: String,
        config_encryption_key: ConfigEncryptionKey,
        avatars_dir: PathBuf,
        connectors: ConnectorRuntime,
    ) -> Self {
        Self {
            pool,
            jwt_secret: Arc::new(jwt_secret),
            login_rate_limiter: LoginRateLimiter::default(),
            config_encryption_key: Arc::new(config_encryption_key),
            avatars_dir: Arc::new(avatars_dir),
            connectors,
            updates: UpdateCache::new(),
        }
    }
}
