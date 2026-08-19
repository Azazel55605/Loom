//! A configurable fake connector, kept in the tree on purpose.
//!
//! **This is a permanent development and testing fixture, not example code.**
//! It is not scaffolding to be deleted once "real" connectors exist, and it must
//! not be removed on the grounds that it talks to nothing. Its whole job is to
//! talk to nothing: it lets the clients — web-frontend today, the desktop and
//! mobile apps later — be built and tested against realistic loading, error,
//! and success states without anyone needing live infrastructure. A contributor
//! with a laptop and no homelab can still work on Loom's UI, and Loom's tests
//! can assert on connector behaviour without depending on a service being up.
//!
//! Everything interesting is set at construction through
//! [`MockConnectorConfig`]:
//!
//! - [`MockConnectorConfig::simulated_status`] — the reading `status()` returns,
//!   so every [`HealthState`](super::HealthState) can be rendered and reviewed.
//! - [`MockConnectorConfig::simulated_latency_ms`] — an artificial delay before
//!   every response, so spinners and skeletons are exercised instead of being
//!   skipped by an instant local answer.
//! - [`MockConnectorConfig::fail_mode`] — a stored [`ConnectorError`] returned
//!   in place of success, so error paths are reachable on demand.
//!
//! ```
//! use loom_core::connector::mock::{MockConnector, MockConnectorConfig};
//! use loom_core::connector::{Connector, ConnectorError};
//!
//! # async fn example() {
//! let flaky = MockConnector::new(MockConnectorConfig {
//!     simulated_latency_ms: 250,
//!     fail_mode: Some(ConnectorError::unreachable("simulated outage")),
//!     ..MockConnectorConfig::default()
//! });
//! assert!(flaky.status().await.is_err());
//! # }
//! ```

use async_trait::async_trait;
use serde_json::{json, Value};

use super::{
    ActionResult, Connector, ConnectorAction, ConnectorError, ConnectorMetadata, ConnectorStatus,
};

/// The action id for the mock's simulated restart.
pub const ACTION_RESTART: &str = "restart";

/// The action id for the mock's simulated reachability check.
pub const ACTION_PING: &str = "ping";

/// How a [`MockConnector`] should behave.
///
/// A plain struct rather than a builder: every field is independent and has a
/// sensible default, so `..MockConnectorConfig::default()` in a struct literal
/// is already the ergonomic form and a builder would only add indirection.
/// The default is the boring happy path — healthy, instant, never failing —
/// which is what most tests want.
#[derive(Debug, Clone, PartialEq)]
pub struct MockConnectorConfig {
    /// The status [`Connector::status`] hands back. Defaults to
    /// [`ConnectorStatus::healthy`]; set it to a degraded or down reading to
    /// see how a client renders one.
    pub simulated_status: ConnectorStatus,

    /// Milliseconds to wait before answering any call. Defaults to `0`.
    ///
    /// The point is to make asynchronicity visible: with a real service the
    /// network supplies the delay, and without one a loading state can appear
    /// to work while actually never being rendered.
    pub simulated_latency_ms: u64,

    /// When `Some`, every call fails with this error instead of succeeding.
    /// Defaults to `None`.
    ///
    /// The error is cloned out on each call rather than consumed, so one
    /// configured connector can be asked repeatedly and behave the same way —
    /// which is why [`ConnectorError`] is `Clone`.
    pub fail_mode: Option<ConnectorError>,
}

impl Default for MockConnectorConfig {
    fn default() -> Self {
        Self {
            simulated_status: ConnectorStatus::healthy(),
            simulated_latency_ms: 0,
            fail_mode: None,
        }
    }
}

/// A [`Connector`] that simulates a service instead of contacting one.
///
/// See the [module documentation](self) for why this exists permanently. Cheap
/// to construct and to clone, holds no resources, and starts no tasks.
#[derive(Debug, Clone, Default)]
pub struct MockConnector {
    config: MockConnectorConfig,
}

impl MockConnector {
    /// Builds a mock with the given behaviour.
    pub fn new(config: MockConnectorConfig) -> Self {
        Self { config }
    }

    /// Builds a mock that always fails with `error`.
    ///
    /// The shorthand for the most common non-default case: pointing a client at
    /// something broken to check its error handling.
    pub fn failing(error: ConnectorError) -> Self {
        Self::new(MockConnectorConfig {
            fail_mode: Some(error),
            ..MockConnectorConfig::default()
        })
    }

    /// Builds a healthy mock that takes `ms` milliseconds to answer, for
    /// exercising loading states.
    pub fn with_latency(ms: u64) -> Self {
        Self::new(MockConnectorConfig {
            simulated_latency_ms: ms,
            ..MockConnectorConfig::default()
        })
    }

    /// Read-only view of the configured behaviour, so tests can assert against
    /// what they asked for without keeping a second copy.
    pub fn config(&self) -> &MockConnectorConfig {
        &self.config
    }

    /// Applies the configured artificial delay.
    ///
    /// `tokio::time::sleep` is the only reason core depends on tokio: it needs
    /// a timer driven by the caller's existing runtime. Nothing here spawns a
    /// task or builds a runtime, so the library stays passive.
    async fn simulate_latency(&self) {
        if self.config.simulated_latency_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(
                self.config.simulated_latency_ms,
            ))
            .await;
        }
    }

    /// Delays, then short-circuits with the configured failure if there is one.
    async fn gate(&self) -> Result<(), ConnectorError> {
        self.simulate_latency().await;
        match &self.config.fail_mode {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }
}

#[async_trait]
impl Connector for MockConnector {
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
        self.gate().await?;
        Ok(self.config.simulated_status.clone())
    }

    /// Returns the two canned actions, or an empty list in fail mode.
    ///
    /// [`Connector::actions`] has no error channel, and the honest translation
    /// of "this connector is currently broken" into a `Vec` is "it can do
    /// nothing right now" — which is also exactly what a client should render
    /// for an unreachable service. Reporting actions that would immediately
    /// fail, or panicking, would both be worse.
    async fn actions(&self) -> Vec<ConnectorAction> {
        if self.gate().await.is_err() {
            return Vec::new();
        }

        vec![
            ConnectorAction::simple(ACTION_RESTART, "Restart")
                .with_description("Pretends to restart the simulated service."),
            ConnectorAction::simple(ACTION_PING, "Ping")
                .with_description("Pretends to check that the simulated service answers."),
        ]
    }

    async fn execute_action(
        &self,
        action_id: &str,
        params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        self.gate().await?;

        match action_id {
            ACTION_RESTART => Ok(ActionResult::ok("Simulated service restarted.")
                .with_payload(json!({ "restarted": true, "params": params }))),
            ACTION_PING => Ok(ActionResult::ok("Simulated service answered the ping.")
                .with_payload(json!({ "pong": true }))),
            unknown => Err(ConnectorError::invalid_action(unknown)),
        }
    }

    fn config_schema(&self) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "Mock connector configuration",
            "description": "The mock connector contacts nothing, so its configuration only \
                            controls how it pretends to behave.",
            "type": "object",
            "properties": {
                "simulatedLatencyMs": {
                    "type": "integer",
                    "minimum": 0,
                    "default": 0,
                    "description": "Artificial delay before each response, in milliseconds."
                },
                "simulatedHealth": {
                    "type": "string",
                    "enum": ["healthy", "degraded", "down", "unknown"],
                    "default": "healthy",
                    "description": "The health state the mock reports."
                }
            },
            "additionalProperties": false
        })
    }

    fn metadata(&self) -> ConnectorMetadata {
        ConnectorMetadata {
            id: "mock".to_string(),
            name: "Mock Service".to_string(),
            icon: Some("beaker".to_string()),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::HealthState;
    use super::*;
    use std::time::Instant;

    #[tokio::test]
    async fn default_mock_is_healthy_and_lists_both_actions() {
        let connector = MockConnector::default();

        let status = connector.status().await.expect("default must succeed");
        assert_eq!(status.health, HealthState::Healthy);

        let ids: Vec<String> = connector
            .actions()
            .await
            .into_iter()
            .map(|action| action.id)
            .collect();
        assert_eq!(ids, vec![ACTION_RESTART, ACTION_PING]);

        assert_eq!(connector.metadata().id, "mock");
        assert!(connector.config_schema().is_object());
    }

    #[tokio::test]
    async fn simulated_status_is_returned_verbatim() {
        let simulated = ConnectorStatus::new(HealthState::Degraded, json!({ "queueDepth": 7 }));
        let connector = MockConnector::new(MockConnectorConfig {
            simulated_status: simulated.clone(),
            ..MockConnectorConfig::default()
        });

        assert_eq!(connector.status().await.unwrap(), simulated);
    }

    #[tokio::test]
    async fn canned_actions_succeed_and_echo_their_params() {
        let connector = MockConnector::default();

        let restart = connector
            .execute_action(ACTION_RESTART, json!({ "force": true }))
            .await
            .unwrap();
        assert!(restart.success);
        assert_eq!(restart.payload.unwrap()["params"], json!({ "force": true }));

        let ping = connector
            .execute_action(ACTION_PING, Value::Null)
            .await
            .unwrap();
        assert!(ping.success);
    }

    #[tokio::test]
    async fn unknown_action_id_is_rejected() {
        let error = MockConnector::default()
            .execute_action("not-a-real-action", Value::Null)
            .await
            .expect_err("unknown ids must not silently succeed");

        assert_eq!(error, ConnectorError::invalid_action("not-a-real-action"));
    }

    #[tokio::test]
    async fn latency_is_respected() {
        // Asserted loosely: the timer guarantees a lower bound, not an exact
        // duration, and a busy CI machine can always take longer.
        let connector = MockConnector::with_latency(50);

        let started = Instant::now();
        connector.status().await.unwrap();
        assert!(
            started.elapsed().as_millis() >= 50,
            "status returned after {:?}, expected at least 50ms",
            started.elapsed()
        );

        let started = Instant::now();
        connector.actions().await;
        assert!(started.elapsed().as_millis() >= 50);

        let started = Instant::now();
        connector
            .execute_action(ACTION_PING, Value::Null)
            .await
            .unwrap();
        assert!(started.elapsed().as_millis() >= 50);
    }

    #[tokio::test]
    async fn fail_mode_applies_to_every_entry_point() {
        let configured = ConnectorError::AuthFailed {
            reason: "simulated bad token".to_string(),
        };
        let connector = MockConnector::failing(configured.clone());

        assert_eq!(connector.status().await.unwrap_err(), configured);
        assert_eq!(
            connector
                .execute_action(ACTION_RESTART, Value::Null)
                .await
                .unwrap_err(),
            configured
        );
        // Even a *valid* action id fails, and an invalid one reports the fail
        // mode rather than `InvalidAction` — the connector never got far enough
        // to look the id up.
        assert_eq!(
            connector
                .execute_action("not-a-real-action", Value::Null)
                .await
                .unwrap_err(),
            configured
        );
        // `actions()` cannot return an error, so it degrades to "nothing is
        // available"; see the impl for why.
        assert!(connector.actions().await.is_empty());
    }

    #[tokio::test]
    async fn fail_mode_is_reusable_across_calls() {
        let connector = MockConnector::failing(ConnectorError::unreachable("simulated outage"));

        for _ in 0..3 {
            assert_eq!(
                connector.status().await.unwrap_err(),
                ConnectorError::unreachable("simulated outage")
            );
        }
    }

    #[test]
    fn config_default_is_the_happy_path() {
        let config = MockConnectorConfig::default();
        assert_eq!(config.simulated_status.health, HealthState::Healthy);
        assert_eq!(config.simulated_latency_ms, 0);
        assert_eq!(config.fail_mode, None);
        // Compared field-by-field: `simulated_status.last_checked` is stamped
        // with the construction time, so two defaults are never `==`.
        let from_connector = MockConnector::default();
        assert_eq!(
            from_connector.config().simulated_status.health,
            config.simulated_status.health
        );
        assert_eq!(from_connector.config().simulated_latency_ms, 0);
        assert_eq!(from_connector.config().fail_mode, None);
    }
}
