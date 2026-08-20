//! Shared application state.

use std::path::PathBuf;
use std::sync::Arc;

use loom_core::connector::{mock::MockConnector, Connector};
use sqlx::SqlitePool;

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
    /// Directory holding uploaded avatar files.
    ///
    /// Carried on the state rather than re-resolved per request so a handler
    /// cannot disagree with the static file service about where avatars live —
    /// they are handed the same path at startup.
    pub avatars_dir: Arc<PathBuf>,
    /// The connector registry.
    ///
    /// Heterogeneous and plural from the start — `Vec<Arc<dyn Connector>>`
    /// rather than one concrete connector — so registering real connectors is
    /// an insertion rather than a reshape of this type and every handler that
    /// reads it.
    pub connectors: Arc<Vec<Arc<dyn Connector>>>,
}

impl AppState {
    /// Builds state around an already-migrated pool.
    ///
    /// The connector registry still holds only `MockConnector`: real connector
    /// loading depends on the contract question left open in
    /// `docs/adr/0002-connector-contract-tbd.md`, and the mock is a permanent
    /// fixture regardless — see `crates/core/src/connector/mock.rs`.
    pub fn new(pool: SqlitePool, jwt_secret: String, avatars_dir: PathBuf) -> Self {
        Self {
            pool,
            jwt_secret: Arc::new(jwt_secret),
            avatars_dir: Arc::new(avatars_dir),
            connectors: Arc::new(vec![Arc::new(MockConnector::default())]),
        }
    }

    /// Finds a registered connector by its metadata id.
    pub fn connector(&self, id: &str) -> Option<&Arc<dyn Connector>> {
        self.connectors
            .iter()
            .find(|connector| connector.metadata().id == id)
    }
}
