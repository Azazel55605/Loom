//! The connector contract: how Loom talks to a service it manages.
//!
//! A *connector* is the adapter between Loom and one thing running in a
//! homelab — a media server, a reverse proxy, a hypervisor. It answers two
//! questions and nothing else: *is this service alright?* ([`Connector::status`])
//! and *what can I ask it to do?* ([`Connector::actions`] /
//! [`Connector::execute_action`]). That split is deliberate: Loom is a
//! management platform, so a connector that can only report uptime is only half
//! a connector.
//!
//! Everything in this module is a wire type. The web backend hands these values
//! straight to its HTTP layer and the TypeScript clients deserialize them
//! as-is, so field names are `camelCase` throughout and the shapes are treated
//! as public API rather than internal detail. See
//! `docs/adr/0002-connector-contract-tbd.md` — the *transport* for connector
//! definitions (declarative manifests, sandboxed WASM, or both) is still open,
//! but both candidates need a Rust-side trait to be driven through, and this is
//! it.
//!
//! Note what is absent: no credentials, no permission checks, no notion of
//! *who* is asking. A connector executes what it is told. Deciding whether a
//! caller is allowed to tell it that belongs to `web-backend`, because this
//! crate also gets linked into clients running on the user's own machine.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod mock;

/// An adapter for one manageable service.
///
/// Implementors are held behind `Box<dyn Connector>` in a registry keyed by
/// [`ConnectorMetadata::id`], so the trait is deliberately dyn-compatible:
/// no generic methods, no `Self`-returning methods, and `#[async_trait]`
/// instead of native `async fn` (which is stable on the pinned toolchain but
/// not object-safe). `Send + Sync` is required because the backend keeps that
/// registry in shared state across request tasks.
///
/// Implementations should be cheap to construct and hold their own
/// configuration; the trait has no `configure` step. Instead,
/// [`Connector::config_schema`] describes what configuration the connector
/// expects so that the loader can validate it and the clients can render a
/// form for it without shipping per-connector UI code.
#[async_trait]
pub trait Connector: Send + Sync {
    /// Checks in on the service and reports how it is doing right now.
    ///
    /// This is expected to be called repeatedly on a polling interval by
    /// whoever owns the schedule (never by the connector itself — core starts
    /// no background work). Implementations should be quick and side-effect
    /// free. A service that answers "I am broken" is a successful call
    /// returning [`HealthState::Degraded`] or [`HealthState::Down`]; the `Err`
    /// arm is for when the check itself could not be carried out.
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError>;

    /// Lists the operations this connector is willing to perform.
    ///
    /// Returned as data rather than compiled in so clients can build their
    /// controls from it dynamically. The list may legitimately be empty — a
    /// read-only connector that only reports health is valid — and it may vary
    /// over the connector's lifetime if the remote service's capabilities
    /// depend on its configuration or state.
    async fn actions(&self) -> Vec<ConnectorAction>;

    /// Performs the action named by `action_id`, passing `params` through.
    ///
    /// `params` is validated against the matching
    /// [`ConnectorAction::params_schema`]; an implementation that receives
    /// something it cannot use should say so with
    /// [`ConnectorError::InvalidParams`] rather than guessing. An `action_id`
    /// that is not in [`Connector::actions`] must produce
    /// [`ConnectorError::InvalidAction`].
    ///
    /// A returned [`ActionResult`] with `success: false` means the service
    /// understood the request and declined or failed it — which is different
    /// from an `Err`, meaning Loom could not get the request across at all.
    async fn execute_action(
        &self,
        action_id: &str,
        params: Value,
    ) -> Result<ActionResult, ConnectorError>;

    /// The JSON Schema for the configuration this connector needs.
    ///
    /// Two consumers rely on it: manifest loading, which validates a stored
    /// configuration before a connector is instantiated, and the frontends,
    /// which generate the setup form from it. Publishing a schema is what keeps
    /// "add a connector" from requiring a matching UI change in three clients.
    /// A connector that needs no configuration should return an empty schema
    /// object rather than `null`.
    fn config_schema(&self) -> Value;

    /// Identifying information for this connector: id, label, icon, version.
    ///
    /// Cheap and synchronous on purpose — it is used for registry keys, list
    /// rendering, and log lines, none of which should have to await a service.
    fn metadata(&self) -> ConnectorMetadata;
}

/// How a service is doing, as of a particular moment.
///
/// Pairs a coarse machine-comparable verdict ([`HealthState`], which is what
/// dashboards colour and alerts fire on) with an open `details` payload for
/// whatever that specific service considers interesting. The timestamp is part
/// of the value rather than the response envelope so a cached or polled status
/// stays honest about its own age.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorStatus {
    /// The coarse verdict clients act on.
    pub health: HealthState,
    /// Connector-specific extras — version strings, queue depths, disk usage.
    ///
    /// Intentionally unstructured: forcing every service's telemetry into one
    /// Rust struct would either bloat it or lose information. Clients that do
    /// not recognise a connector simply ignore this.
    pub details: Value,
    /// When this reading was actually taken, so stale data is visible as stale.
    pub last_checked: DateTime<Utc>,
}

impl ConnectorStatus {
    /// A "nothing to report" status stamped with the current time.
    ///
    /// The common case for a connector whose check succeeded and which has no
    /// extra telemetry to attach.
    pub fn healthy() -> Self {
        Self::new(HealthState::Healthy, Value::Object(Default::default()))
    }

    /// A status with the given verdict and details, stamped with the current
    /// time.
    ///
    /// Use this rather than filling in `last_checked` by hand so that "now"
    /// always means the moment the reading was constructed.
    pub fn new(health: HealthState, details: Value) -> Self {
        Self {
            health,
            details,
            last_checked: Utc::now(),
        }
    }
}

/// The coarse health verdict for a service.
///
/// Kept to four cases on purpose: clients need to sort, colour, and alert on
/// this, which a free-form string cannot support. Serializes lowercase
/// (`"healthy"`, `"degraded"`, `"down"`, `"unknown"`) to match the TypeScript
/// union on the other side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HealthState {
    /// Reachable and behaving as expected.
    Healthy,
    /// Reachable but impaired — slow, partially failing, or self-reporting a
    /// problem. Worth surfacing, not worth waking someone up for.
    Degraded,
    /// Reachable-but-refusing or not answering at all. Something is broken.
    Down,
    /// No usable reading: never polled yet, or the service exposes nothing we
    /// can interpret. Distinct from [`HealthState::Down`] so a dashboard does
    /// not report an outage it has not actually observed.
    Unknown,
}

/// One operation a connector exposes, described well enough for a client to
/// render a control for it without knowing what the connector is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorAction {
    /// Stable identifier passed back to [`Connector::execute_action`].
    ///
    /// Machine-facing: it belongs in URLs and stored automations, so it should
    /// not change when the label does.
    pub id: String,
    /// Short human-facing name for the button or menu entry.
    pub label: String,
    /// Optional longer explanation, for tooltips and confirmation prompts —
    /// the place to warn that an action is disruptive.
    pub description: Option<String>,
    /// JSON Schema for this action's parameters, driving both client-side form
    /// generation and server-side validation.
    ///
    /// An action that takes no parameters uses an empty object rather than
    /// `null`, so consumers can always treat this as a schema.
    pub params_schema: Value,
}

impl ConnectorAction {
    /// A parameterless action with an id and a label.
    ///
    /// Covers the majority of real actions ("restart", "refresh") without
    /// making every call site spell out an empty schema.
    pub fn simple(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            params_schema: Value::Object(Default::default()),
        }
    }

    /// Attaches a description, for chaining onto [`ConnectorAction::simple`].
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// The outcome of an attempted action.
///
/// Distinct from `Result`: this type describes what the *service* did once the
/// request reached it. `success: false` is a normal, well-formed answer — the
/// service was asked and declined — whereas a [`ConnectorError`] means Loom
/// never got a verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    /// Whether the service carried the action out.
    pub success: bool,
    /// Human-readable summary, shown to the user verbatim. Should be
    /// meaningful on its own, since clients are not expected to map it.
    pub message: String,
    /// Optional structured result — a job id, the new state, a listing — for
    /// clients that want more than the message.
    pub payload: Option<Value>,
}

impl ActionResult {
    /// A successful result carrying only a message.
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            payload: None,
        }
    }

    /// A failed-but-answered result: the service was reached and said no.
    ///
    /// Reach for this instead of [`ConnectorError`] when the failure is the
    /// service's decision rather than a transport problem.
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            payload: None,
        }
    }

    /// Attaches a structured payload, for chaining onto the constructors above.
    #[must_use]
    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = Some(payload);
        self
    }
}

/// Identity and presentation for a connector.
///
/// This is what a client needs in order to list a connector before it has
/// talked to anything: what to call it, what to draw next to it, and which
/// revision of the connector produced the data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorMetadata {
    /// Stable machine identifier — the registry key and the URL segment.
    /// Lowercase kebab-case by convention (`"mock"`, `"reverse-proxy"`).
    pub id: String,
    /// Display name shown in the UI.
    pub name: String,
    /// Icon *identifier*, not image data — a name the clients resolve against
    /// their own icon set, so core never ships assets or assumes a renderer.
    /// `None` means "use the generic fallback".
    pub icon: Option<String>,
    /// Version of the connector implementation itself, independent of the Loom
    /// release, so a connector can be revised without a platform bump.
    pub version: String,
}

/// Why a connector could not answer.
///
/// Reserved for failures of the *interaction*, not of the managed service: a
/// service reporting its own bad state is a successful [`ConnectorStatus`], and
/// a service refusing an action is an [`ActionResult`] with `success: false`.
/// Mixing those into this enum would make it impossible for a client to tell
/// "Loom is misconfigured" from "your server is unhappy".
///
/// Three properties are load-bearing beyond `Error`:
///
/// - `Clone`, so a stored error (notably [`mock::MockConnector`]'s fail mode)
///   can be handed out repeatedly.
/// - `Serialize`/`Deserialize`, so the backend can forward the discriminant to
///   the clients instead of flattening everything to a string. This works
///   because every variant payload is a plain `String`; the derived
///   representation is externally tagged, giving objects like
///   `{"invalidAction":{"actionId":"nope"}}` and `{"internal":"…"}` (variant tags
///   and their fields both `camelCase`). Any future
///   variant that needs a non-serializable payload (an I/O error, a
///   `Box<dyn Error>`) must be reduced to a string here rather than breaking
///   this property.
/// - No `#[source]` anywhere, for the same reason — a chained cause would not
///   survive the wire.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConnectorError {
    /// The service could not be contacted: refused, timed out, DNS failure.
    /// The user's likely fix is at the infrastructure level, not in Loom.
    #[error("service is unreachable: {reason}")]
    Unreachable {
        /// What went wrong, in terms a user can act on.
        reason: String,
    },

    /// The service was reached but rejected our credentials, so the stored
    /// configuration needs attention. Kept separate from `Unreachable` because
    /// the remedy is completely different.
    #[error("authentication with the service failed: {reason}")]
    AuthFailed {
        /// What the service objected to, with secrets left out.
        reason: String,
    },

    /// The requested action id is not one this connector exposes — usually a
    /// stale client or an automation referring to a removed action.
    #[error("unknown action id: {action_id}")]
    InvalidAction {
        /// The id that was asked for, echoed back for the log line.
        action_id: String,
    },

    /// The action exists but the parameters do not satisfy its schema.
    #[error("invalid parameters for action {action_id}: {reason}")]
    InvalidParams {
        /// The action whose parameters were rejected.
        action_id: String,
        /// Which constraint failed, so the client can point at the field.
        reason: String,
    },

    /// Anything else went wrong inside the connector — a bug, an unexpected
    /// response shape, a failed parse. The catch-all exists so implementors are
    /// never tempted to panic; it should not be the answer to a foreseeable
    /// condition.
    #[error("connector failed internally: {0}")]
    Internal(String),
}

impl ConnectorError {
    /// Shorthand for [`ConnectorError::Unreachable`].
    pub fn unreachable(reason: impl Into<String>) -> Self {
        Self::Unreachable {
            reason: reason.into(),
        }
    }

    /// Shorthand for [`ConnectorError::InvalidAction`].
    pub fn invalid_action(action_id: impl Into<String>) -> Self {
        Self::InvalidAction {
            action_id: action_id.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    /// A minimal in-test implementation, separate from [`mock::MockConnector`]
    /// on purpose: it proves the trait alone is enough to write a connector,
    /// with no help from the fixture's machinery.
    struct StubConnector {
        id: &'static str,
        health: HealthState,
    }

    #[async_trait]
    impl Connector for StubConnector {
        async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
            Ok(ConnectorStatus::new(self.health, json!({ "stub": true })))
        }

        async fn actions(&self) -> Vec<ConnectorAction> {
            vec![ConnectorAction::simple("noop", "Do nothing")]
        }

        async fn execute_action(
            &self,
            action_id: &str,
            _params: Value,
        ) -> Result<ActionResult, ConnectorError> {
            if action_id == "noop" {
                Ok(ActionResult::ok("did nothing"))
            } else {
                Err(ConnectorError::invalid_action(action_id))
            }
        }

        fn config_schema(&self) -> Value {
            json!({ "type": "object", "properties": {} })
        }

        fn metadata(&self) -> ConnectorMetadata {
            ConnectorMetadata {
                id: self.id.to_string(),
                name: "Stub".to_string(),
                icon: None,
                version: "0.0.1".to_string(),
            }
        }
    }

    #[tokio::test]
    async fn trait_is_object_safe_and_works_in_a_heterogeneous_collection() {
        let connectors: Vec<Box<dyn Connector>> = vec![
            Box::new(StubConnector {
                id: "stub-a",
                health: HealthState::Healthy,
            }),
            Box::new(StubConnector {
                id: "stub-b",
                health: HealthState::Degraded,
            }),
            Box::new(mock::MockConnector::default()),
        ];

        assert_eq!(connectors.len(), 3);

        let ids: Vec<String> = connectors.iter().map(|c| c.metadata().id).collect();
        assert_eq!(ids, vec!["stub-a", "stub-b", "mock"]);

        for connector in &connectors {
            let status = connector.status().await.expect("status should succeed");
            assert_ne!(status.health, HealthState::Unknown);
            assert!(connector.config_schema().is_object());
            assert!(!connector.actions().await.is_empty());
        }

        assert_eq!(
            connectors[0]
                .execute_action("noop", Value::Null)
                .await
                .expect("noop should succeed")
                .message,
            "did nothing"
        );
        assert_eq!(
            connectors[1]
                .execute_action("nope", Value::Null)
                .await
                .expect_err("unknown action should fail"),
            ConnectorError::invalid_action("nope")
        );
    }

    #[test]
    fn connector_status_serializes_with_camel_case_keys() {
        let status = ConnectorStatus {
            health: HealthState::Degraded,
            details: json!({ "queueDepth": 12 }),
            last_checked: Utc.with_ymd_and_hms(2026, 8, 19, 12, 0, 0).unwrap(),
        };

        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(
            value,
            json!({
                "health": "degraded",
                "details": { "queueDepth": 12 },
                "lastChecked": "2026-08-19T12:00:00Z"
            })
        );
        assert_eq!(
            serde_json::from_value::<ConnectorStatus>(value).unwrap(),
            status
        );
    }

    #[test]
    fn health_state_variants_serialize_lowercase() {
        for (state, expected) in [
            (HealthState::Healthy, "healthy"),
            (HealthState::Degraded, "degraded"),
            (HealthState::Down, "down"),
            (HealthState::Unknown, "unknown"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), json!(expected));
        }
    }

    #[test]
    fn connector_action_serializes_with_camel_case_keys() {
        let action = ConnectorAction::simple("restart", "Restart").with_description("Restarts it.");

        let value = serde_json::to_value(&action).unwrap();
        assert_eq!(
            value,
            json!({
                "id": "restart",
                "label": "Restart",
                "description": "Restarts it.",
                "paramsSchema": {}
            })
        );
        assert_eq!(
            serde_json::from_value::<ConnectorAction>(value).unwrap(),
            action
        );
    }

    #[test]
    fn action_result_serializes_with_camel_case_keys() {
        let result = ActionResult::ok("restarted").with_payload(json!({ "jobId": "abc" }));

        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(
            value,
            json!({
                "success": true,
                "message": "restarted",
                "payload": { "jobId": "abc" }
            })
        );
        assert_eq!(
            serde_json::from_value::<ActionResult>(value).unwrap(),
            result
        );
    }

    #[test]
    fn connector_metadata_serializes_with_camel_case_keys() {
        let metadata = ConnectorMetadata {
            id: "mock".to_string(),
            name: "Mock Service".to_string(),
            icon: Some("beaker".to_string()),
            version: "1.0.0".to_string(),
        };

        let value = serde_json::to_value(&metadata).unwrap();
        assert_eq!(
            value,
            json!({
                "id": "mock",
                "name": "Mock Service",
                "icon": "beaker",
                "version": "1.0.0"
            })
        );
        assert_eq!(
            serde_json::from_value::<ConnectorMetadata>(value).unwrap(),
            metadata
        );
    }

    #[test]
    fn connector_error_serializes_with_camel_case_tags_and_round_trips() {
        let cases = [
            (
                ConnectorError::unreachable("connection refused"),
                json!({ "unreachable": { "reason": "connection refused" } }),
            ),
            (
                ConnectorError::AuthFailed {
                    reason: "token rejected".to_string(),
                },
                json!({ "authFailed": { "reason": "token rejected" } }),
            ),
            (
                ConnectorError::invalid_action("nope"),
                json!({ "invalidAction": { "actionId": "nope" } }),
            ),
            (
                ConnectorError::InvalidParams {
                    action_id: "restart".to_string(),
                    reason: "missing field `force`".to_string(),
                },
                json!({ "invalidParams": {
                    "actionId": "restart",
                    "reason": "missing field `force`"
                } }),
            ),
            (
                ConnectorError::Internal("unexpected response shape".to_string()),
                json!({ "internal": "unexpected response shape" }),
            ),
        ];

        for (error, expected) in cases {
            let value = serde_json::to_value(&error).unwrap();
            assert_eq!(value, expected, "unexpected shape for {error:?}");
            assert_eq!(
                serde_json::from_value::<ConnectorError>(value).unwrap(),
                error
            );
        }
    }

    #[test]
    fn connector_error_display_messages_are_useful() {
        assert_eq!(
            ConnectorError::invalid_action("nope").to_string(),
            "unknown action id: nope"
        );
        assert_eq!(
            ConnectorError::Internal("boom".to_string()).to_string(),
            "connector failed internally: boom"
        );
    }

    /// Prints the canonical wire shapes. Not an assertion — it exists so the
    /// exact JSON the clients see can be read off a test run rather than
    /// reconstructed from the type definitions. Run with
    /// `cargo test -p loom-core -- --nocapture print_wire_shapes`.
    #[test]
    fn print_wire_shapes() {
        let last_checked = Utc.with_ymd_and_hms(2026, 8, 19, 12, 0, 0).unwrap();
        let status = ConnectorStatus {
            health: HealthState::Degraded,
            details: json!({ "version": "1.2.3", "queueDepth": 12 }),
            last_checked,
        };
        println!(
            "ConnectorStatus =\n{}",
            serde_json::to_string_pretty(&status).unwrap()
        );
        println!(
            "ConnectorAction =\n{}",
            serde_json::to_string_pretty(
                &ConnectorAction::simple("restart", "Restart")
                    .with_description("Restarts the service.")
            )
            .unwrap()
        );
        println!(
            "ActionResult =\n{}",
            serde_json::to_string_pretty(
                &ActionResult::ok("restart requested").with_payload(json!({ "jobId": "abc123" }))
            )
            .unwrap()
        );
        println!(
            "ConnectorMetadata =\n{}",
            serde_json::to_string_pretty(&ConnectorMetadata {
                id: "mock".to_string(),
                name: "Mock Service".to_string(),
                icon: Some("beaker".to_string()),
                version: "1.0.0".to_string(),
            })
            .unwrap()
        );
        for error in [
            ConnectorError::unreachable("connection refused"),
            ConnectorError::AuthFailed {
                reason: "token rejected".to_string(),
            },
            ConnectorError::invalid_action("nope"),
            ConnectorError::InvalidParams {
                action_id: "restart".to_string(),
                reason: "missing field `force`".to_string(),
            },
            ConnectorError::Internal("unexpected response shape".to_string()),
        ] {
            println!(
                "ConnectorError =\n{}",
                serde_json::to_string_pretty(&error).unwrap()
            );
        }
    }
}
