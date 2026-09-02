//! Which connector types this build knows how to construct.
//!
//! A compile-time table, not a database one. A registration carries executable
//! code — a factory that turns stored JSON into a live connector — and code
//! cannot come out of a row, so the set of *types* is fixed at build time even
//! though the set of *instances* is not. That is why `connector_instances.
//! connector_type` has no foreign key: there is no table for it to point at.
//! An instance whose type is no longer registered is therefore possible (a
//! build that dropped a connector), and is handled by skipping it at load with
//! a warning rather than by refusing to start.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use loom_connector_docker::DockerConnector;
use loom_connector_truenas::TrueNasConnector;
use loom_core::connector::debug::DebugConnector;
use loom_core::connector::{Connector, ConnectorError, SetupGuide};
use serde_json::Value;

/// Builds a live connector from a stored configuration, or explains why it
/// cannot.
///
/// **Asynchronous, because construction can involve I/O.** A connector to a
/// real service validates its configuration by *using* it: the Docker
/// connector opens its endpoint and inspects the named container, which is what
/// lets "no such container" be a different error from "no such host" at the
/// moment someone still has the form open to fix. A synchronous factory could
/// only reach that by blocking a worker thread on a runtime of its own, which
/// is a worse answer to a question the signature can simply be honest about.
///
/// Still a plain `fn` pointer rather than a boxed closure: every registration
/// is a free function, and a `fn` stays trivially `Send + Sync`. The return is
/// boxed because each factory's future is a distinct anonymous type. Swap to
/// `Arc<dyn Fn…>` only when a connector type genuinely needs to capture
/// something.
pub type ConnectorFactory =
    fn(Value) -> Pin<Box<dyn Future<Output = Result<Box<dyn Connector>, ConnectorError>> + Send>>;

/// One connector type this build can create instances of.
pub struct ConnectorTypeRegistration {
    /// Stable machine identifier, stored in `connector_instances.connector_type`
    /// and used as the registry key. Lowercase kebab-case by convention.
    pub type_id: &'static str,
    /// Human-facing name for the type picker.
    pub display_name: &'static str,
    /// The type's icon reference, in the `ConnectorMetadata::icon` convention
    /// (`"brand:<key>"` or `"lucide:<name>"`).
    ///
    /// Snapshotted from the same default instance as the schema and other type
    /// descriptors, because the type picker draws an icon before a configured
    /// instance exists.
    pub icon: Option<String>,
    /// Turns stored configuration into a live connector.
    pub factory: ConnectorFactory,
    /// Optional constructor for the ephemeral setup connection check.
    ///
    /// Most connectors validate and test through the same factory. A connector
    /// whose ordinary factory requires more permission than its capability
    /// check is trying to measure can provide a no-I/O constructor here.
    pub connection_test_factory: Option<ConnectorFactory>,
    /// The configuration this type accepts, as JSON Schema.
    pub schema: Value,
    /// Optional descriptive setup content published with the schema.
    pub setup_guide: Option<SetupGuide>,
    /// Type id this connector can discover through a configured instance.
    pub discoverable_type: Option<String>,
    /// Candidate configuration field that discovery can fill directly.
    pub discovery_target_field: Option<String>,
}

/// Every registered type, keyed by [`ConnectorTypeRegistration::type_id`].
///
/// Behind an `Arc` because it is built once at startup and then read by every
/// request; it never changes after construction, so there is nothing to lock.
pub type ConnectorTypeRegistry = Arc<HashMap<&'static str, ConnectorTypeRegistration>>;

/// The types compiled into this build.
///
/// Three today: the debug fixture, Docker, and TrueNAS. Further integrations
/// (a reverse proxy, a hypervisor) register here alongside them,
/// and nothing else in the backend has to change when they do — that is the
/// point of the indirection.
pub fn builtin_registry() -> ConnectorTypeRegistry {
    let mut types = HashMap::new();
    // Connector descriptors are instance methods because plugins may derive
    // them from their implementation. Construct the cheap default exactly
    // once and snapshot every type-level descriptor together, so schema,
    // setup help, discovery capability, and icon cannot come from different
    // throwaway instances or duplicated constants.
    let debug = DebugConnector::default();
    let debug_metadata = debug.metadata();

    types.insert(
        loom_core::connector::debug::TYPE_ID,
        ConnectorTypeRegistration {
            type_id: loom_core::connector::debug::TYPE_ID,
            display_name: "Debug Connector",
            icon: debug_metadata.icon,
            // Synchronous work behind an async signature: the fixture contacts
            // nothing, so there is nothing to await and the future is ready
            // immediately rather than pretending it might not be.
            factory: |config| {
                Box::pin(async move {
                    DebugConnector::from_config_value(config)
                        .map(|connector| Box::new(connector) as Box<dyn Connector>)
                })
            },
            connection_test_factory: None,
            schema: debug.config_schema(),
            setup_guide: debug.setup_guide(),
            discoverable_type: debug.discoverable_type(),
            discovery_target_field: debug.discovery_target_field(),
        },
    );

    // The Docker connector is the exception to the snapshot-a-default-instance
    // pattern above, and it has to be: constructing one requires a reachable
    // Docker endpoint, so there is no default instance to ask. Its type-level descriptors are `const`s in
    // the connector crate instead, read from here *and* from its own
    // `metadata()`, so the two cannot drift the way a hand-copied duplicate
    // would.
    types.insert(
        loom_connector_docker::TYPE_ID,
        ConnectorTypeRegistration {
            type_id: loom_connector_docker::TYPE_ID,
            display_name: loom_connector_docker::DISPLAY_NAME,
            icon: Some(loom_connector_docker::ICON.to_owned()),
            // This factory really does await: it opens the endpoint and
            // pings the host, so an invalid config and an unreachable endpoint
            // remain distinct and actionable failures.
            factory: |config| {
                Box::pin(async move {
                    DockerConnector::from_config_value(config)
                        .await
                        .map(|connector| Box::new(connector) as Box<dyn Connector>)
                })
            },
            connection_test_factory: Some(|config| {
                Box::pin(async move {
                    DockerConnector::from_config_value_for_connection_test(config)
                        .map(|connector| Box::new(connector) as Box<dyn Connector>)
                })
            }),
            schema: loom_connector_docker::config_schema(),
            // Containers are addressable sub-targets of this one connection,
            // not proposals for separate connector instances.
            setup_guide: Some(loom_connector_docker::setup_guide()),
            discoverable_type: None,
            discovery_target_field: None,
        },
    );

    // Like Docker, a TrueNAS connector must contact its configured service to
    // exist, so every descriptor needed before construction comes from shared
    // crate-level constants/functions rather than a fake default instance.
    types.insert(
        loom_connector_truenas::TYPE_ID,
        ConnectorTypeRegistration {
            type_id: loom_connector_truenas::TYPE_ID,
            display_name: loom_connector_truenas::DISPLAY_NAME,
            icon: Some(loom_connector_truenas::ICON.to_owned()),
            factory: |config| {
                Box::pin(async move {
                    TrueNasConnector::from_config_value(config)
                        .await
                        .map(|connector| Box::new(connector) as Box<dyn Connector>)
                })
            },
            connection_test_factory: None,
            schema: loom_connector_truenas::config_schema(),
            setup_guide: None,
            discoverable_type: None,
            discovery_target_field: None,
        },
    );

    Arc::new(types)
}

#[cfg(test)]
mod tests {
    use super::*;
    use loom_core::connector::debug::TYPE_ID as DEBUG_TYPE_ID;
    use serde_json::json;

    #[test]
    fn the_debug_type_is_registered_with_a_usable_schema() {
        let registry = builtin_registry();
        let registration = registry
            .get(DEBUG_TYPE_ID)
            .expect("the debug type must always be registered");

        assert_eq!(registration.type_id, DEBUG_TYPE_ID);
        assert!(!registration.display_name.is_empty());

        let schema = &registration.schema;
        assert_eq!(schema["type"], json!("object"));
        assert!(
            schema["properties"].is_object(),
            "the add-connector form is generated from this"
        );
    }

    #[test]
    fn every_registered_type_publishes_a_form_generatable_schema() {
        // The add-connector form is generated from this, so a type whose schema
        // is not an object is a type nobody can add through the UI. Checked for
        // every registration rather than only the debug one, because a schema
        // that needs no daemon to read is exactly what lets the Docker form be
        // rendered before a host has been named.
        for registration in builtin_registry().values() {
            assert!(!registration.display_name.is_empty());
            assert_eq!(
                registration.schema["type"],
                json!("object"),
                "{} must publish an object schema",
                registration.type_id
            );
            assert!(
                registration.schema["properties"].is_object(),
                "{} publishes no properties",
                registration.type_id
            );
        }
    }

    #[test]
    fn the_docker_type_is_registered_and_describable_without_a_daemon() {
        let registry = builtin_registry();
        let registration = registry
            .get(loom_connector_docker::TYPE_ID)
            .expect("the Docker type must be registered");

        assert_eq!(registration.type_id, "docker");
        assert_eq!(registration.icon.as_deref(), Some("brand:docker"));
        assert!(registration.schema["properties"]["dockerHost"].is_object());
        assert!(registration.schema["properties"]
            .get("containerName")
            .is_none());

        let guide = registration.setup_guide.as_ref().expect("setup guide");
        assert_eq!(guide.variants.len(), 2);
        assert_eq!(guide.variants[0].id, "socket");
        assert_eq!(guide.variants[1].id, "proxy");
        assert!(registration.connection_test_factory.is_some());

        // Discovery is configuration-dependent, so the static catalog does
        // not claim it unconditionally.
        assert!(registration.discoverable_type.is_none());
        assert!(registration.discovery_target_field.is_none());
    }

    #[test]
    fn the_truenas_type_is_registered_with_an_encrypted_api_key_schema() {
        let registry = builtin_registry();
        let registration = registry
            .get(loom_connector_truenas::TYPE_ID)
            .expect("the TrueNAS type must be registered");

        assert_eq!(registration.type_id, "truenas");
        assert_eq!(registration.display_name, "TrueNAS");
        assert_eq!(registration.icon.as_deref(), Some("brand:truenas"));
        assert_eq!(
            registration.schema["properties"]["apiKey"]["x-loom-sensitive"],
            true
        );
        assert!(registration.setup_guide.is_none());
        assert!(registration.connection_test_factory.is_none());
        assert!(registration.discoverable_type.is_none());
    }

    #[tokio::test]
    async fn every_registration_declares_its_connector_s_own_icon() {
        // The registration copies the icon so the type picker can draw one
        // without an instance. A copy that disagrees with the connector would
        // show one icon in the picker and a different one on the card that the
        // picker just created, which reads as a bug in the icon system rather
        // than as the transcription error it is.
        for registration in builtin_registry().values() {
            let Ok(built) = (registration.factory)(Value::Object(Default::default())).await else {
                // A type that cannot be built from an empty configuration is
                // skipped. The Docker connector is one: it needs a container
                // name and a reachable host, which is why its icon comes from a
                // shared `const` instead of a copy this test would police.
                continue;
            };
            assert_eq!(
                built.metadata().icon.as_deref(),
                registration.icon.as_deref(),
                "{} registers an icon its connector does not declare",
                registration.type_id
            );
        }
    }

    #[test]
    fn the_debug_type_bundles_discovery_and_setup_descriptors() {
        let registry = builtin_registry();
        let registration = registry.get(DEBUG_TYPE_ID).expect("registered");

        assert_eq!(
            registration.discoverable_type.as_deref(),
            Some(DEBUG_TYPE_ID)
        );
        assert_eq!(registration.discovery_target_field, None);
        let guide = registration.setup_guide.as_ref().expect("setup guide");
        assert_eq!(guide.variants.len(), 2);
        assert!(guide.variants[1].template.contains("{{label}}"));
    }

    #[test]
    fn every_registration_is_keyed_by_its_own_type_id() {
        // A key that disagrees with the registration it holds would make a
        // lookup succeed and then build the wrong connector.
        for (key, registration) in builtin_registry().iter() {
            assert_eq!(*key, registration.type_id);
        }
    }

    #[tokio::test]
    async fn the_factory_validates_before_it_constructs() {
        let registry = builtin_registry();
        let factory = registry.get(DEBUG_TYPE_ID).expect("registered").factory;

        let Ok(built) = factory(json!({ "baseLoad": 10 })).await else {
            panic!("a valid configuration must build");
        };
        assert_eq!(built.metadata().id, DEBUG_TYPE_ID);

        // `Box<dyn Connector>` is not `Debug`, so the happy arm is discarded
        // before the error is examined rather than unwrapped through it.
        let error = factory(json!({ "baseLoad": 900 }))
            .await
            .err()
            .expect("an invalid configuration must be refused");
        assert!(matches!(error, ConnectorError::InvalidConfig { .. }));
    }

    /// Omitting the container selects host mode, which still proves the daemon
    /// is reachable before returning a live connector.
    #[tokio::test]
    async fn the_docker_factory_treats_no_container_as_host_mode() {
        let registry = builtin_registry();
        let factory = registry
            .get(loom_connector_docker::TYPE_ID)
            .expect("registered")
            .factory;

        // Loopback port 1 is reserved and has no daemon. If the factory still
        // treated stale containerName as an error this would be InvalidConfig
        // instead of the reachability failure host mode must report.
        let error = factory(json!({ "dockerHost": "tcp://127.0.0.1:1" }))
            .await
            .err()
            .expect("host mode still validates daemon reachability");
        assert!(
            matches!(error, ConnectorError::Unreachable { .. }),
            "host mode must reach the configured daemon: {error}"
        );
    }
}
