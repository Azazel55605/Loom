//! The live connectors this instance currently has.
//!
//! One [`Connector`] object per row in `connector_instances`, constructed at
//! startup and kept for the process's lifetime. The map exists because a
//! connector is not a value that can be rebuilt per request: a real one will
//! hold a client, a connection pool, a token cache, and rebuilding it on every
//! poll would throw all of that away. The database row is the durable record;
//! this map is the running thing the row describes.
//!
//! Writes go through here rather than straight to the map so the two can never
//! disagree: creating an instance persists *and* inserts, deleting removes
//! from both, and updating replaces the live entry with one built from the new
//! configuration. Nothing else may hold a long-lived reference to a connector,
//! or an update would leave a stale one in use.

use std::collections::HashMap;
use std::sync::Arc;

use loom_core::connector::{Connector, ConnectorError};
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::sync::RwLock;
use uuid::Uuid;

use super::registry::{ConnectorTypeRegistration, ConnectorTypeRegistry};

/// Why an instance could not be constructed from a type id and a configuration.
#[derive(Debug)]
pub enum BuildError {
    /// No such type is registered in this build.
    UnknownType(String),
    /// The type is registered and refused the configuration. Carries the
    /// connector's own objection, so the caller can be told what is wrong with
    /// their input rather than that "something" is.
    Rejected(ConnectorError),
}

/// The live connectors, plus the registry they were built from.
///
/// Cloned per request as part of [`crate::state::AppState`]; both fields are
/// `Arc`, so a clone is two pointer bumps.
#[derive(Clone)]
pub struct ConnectorRuntime {
    types: ConnectorTypeRegistry,
    /// `Arc<dyn Connector>` rather than `Box`, because a handler needs to hold
    /// a connector across an `await` (`status()` and `execute_action()` are
    /// both async) and must not hold the map's lock while doing so. Cloning the
    /// `Arc` out and releasing the guard is what keeps one slow connector from
    /// blocking every other request.
    instances: Arc<RwLock<HashMap<Uuid, Arc<dyn Connector>>>>,
}

impl ConnectorRuntime {
    /// An empty runtime over `types`.
    pub fn new(types: ConnectorTypeRegistry) -> Self {
        Self {
            types,
            instances: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Builds a runtime and populates it from `connector_instances`.
    ///
    /// A row that cannot be turned into a live connector — unknown type,
    /// unparseable id, configuration the factory rejects — is **logged and
    /// skipped**, not fatal. The alternative is a server that refuses to start
    /// because of one bad connector, which would take authentication and every
    /// other connector down with it; the row survives on disk and can be fixed
    /// or deleted through the API. See `docs/adr/0004-zero-config-startup.md`
    /// for why startup fails as rarely as possible.
    pub async fn load(
        pool: &SqlitePool,
        types: ConnectorTypeRegistry,
    ) -> Result<Self, sqlx::Error> {
        let runtime = Self::new(types);

        let rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, connector_type, config FROM connector_instances",
        )
        .fetch_all(pool)
        .await?;

        let mut live = runtime.instances.write().await;
        for (id, connector_type, config) in rows {
            let Ok(uuid) = Uuid::parse_str(&id) else {
                tracing::warn!(instance = %id, "skipping connector instance with an unparseable id");
                continue;
            };

            let config: Value = match serde_json::from_str(&config) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(
                        instance = %id,
                        %error,
                        "skipping connector instance whose stored config is not valid JSON"
                    );
                    continue;
                }
            };

            match runtime.build(&connector_type, config) {
                Ok(connector) => {
                    live.insert(uuid, connector);
                }
                Err(BuildError::UnknownType(type_id)) => tracing::warn!(
                    instance = %id,
                    connector_type = %type_id,
                    "skipping connector instance of a type this build does not register"
                ),
                Err(BuildError::Rejected(error)) => tracing::warn!(
                    instance = %id,
                    %error,
                    "skipping connector instance the connector refused to be built from"
                ),
            }
        }
        drop(live);

        tracing::info!(count = runtime.len().await, "loaded connector instances");

        Ok(runtime)
    }

    /// The registered connector types.
    pub fn types(&self) -> &ConnectorTypeRegistry {
        &self.types
    }

    /// The registration for `type_id`, if this build has one.
    pub fn registration(&self, type_id: &str) -> Option<&ConnectorTypeRegistration> {
        self.types.get(type_id)
    }

    /// Constructs a connector from a type id and a configuration, without
    /// touching the map.
    ///
    /// Separated from insertion so create and update can validate *before*
    /// they write: a configuration that the connector refuses must never reach
    /// the database, or the next startup would skip the row it created.
    pub fn build(&self, type_id: &str, config: Value) -> Result<Arc<dyn Connector>, BuildError> {
        let registration = self
            .registration(type_id)
            .ok_or_else(|| BuildError::UnknownType(type_id.to_owned()))?;

        (registration.factory)(config)
            .map(Arc::from)
            .map_err(BuildError::Rejected)
    }

    /// Inserts or replaces the live connector for `id`.
    pub async fn insert(&self, id: Uuid, connector: Arc<dyn Connector>) {
        self.instances.write().await.insert(id, connector);
    }

    /// Drops the live connector for `id`.
    pub async fn remove(&self, id: &Uuid) {
        self.instances.write().await.remove(id);
    }

    /// The live connector for `id`, if there is one.
    ///
    /// Returns a clone of the `Arc` and releases the lock, so the caller can
    /// await on it freely.
    pub async fn get(&self, id: &Uuid) -> Option<Arc<dyn Connector>> {
        self.instances.read().await.get(id).cloned()
    }

    /// How many live connectors there are.
    ///
    /// Used for the startup log line and by tests; listing goes through the
    /// database, which is the ordering authority.
    pub async fn len(&self) -> usize {
        self.instances.read().await.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::registry::builtin_registry;
    use loom_core::connector::debug::TYPE_ID as DEBUG_TYPE_ID;
    use serde_json::json;

    #[tokio::test]
    async fn building_reports_an_unknown_type_separately_from_a_refused_config() {
        let runtime = ConnectorRuntime::new(builtin_registry());

        assert!(matches!(
            runtime.build("not-a-type", json!({})),
            Err(BuildError::UnknownType(type_id)) if type_id == "not-a-type"
        ));

        assert!(matches!(
            runtime.build(DEBUG_TYPE_ID, json!({ "baseLoad": 900 })),
            Err(BuildError::Rejected(ConnectorError::InvalidConfig { .. }))
        ));

        assert!(runtime.build(DEBUG_TYPE_ID, json!({})).is_ok());
    }

    #[tokio::test]
    async fn instances_can_be_inserted_replaced_and_removed() {
        let runtime = ConnectorRuntime::new(builtin_registry());
        let id = Uuid::new_v4();

        assert!(runtime.get(&id).await.is_none());

        runtime
            .insert(id, runtime.build(DEBUG_TYPE_ID, json!({})).unwrap())
            .await;
        assert!(runtime.get(&id).await.is_some());
        assert_eq!(runtime.len().await, 1);

        // Replacing must not leave the old connector reachable.
        runtime
            .insert(
                id,
                runtime
                    .build(DEBUG_TYPE_ID, json!({ "label": "replaced" }))
                    .unwrap(),
            )
            .await;
        assert_eq!(runtime.len().await, 1);
        assert!(runtime
            .get(&id)
            .await
            .unwrap()
            .display_fields()
            .iter()
            .any(|field| field.value == "replaced"));

        runtime.remove(&id).await;
        assert!(runtime.get(&id).await.is_none());
        assert_eq!(runtime.len().await, 0);
    }
}
