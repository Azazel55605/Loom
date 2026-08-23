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
use std::sync::Arc;

use loom_core::connector::debug::DebugConnector;
use loom_core::connector::{Connector, ConnectorError, SetupGuide};
use serde_json::Value;

/// Builds a live connector from a stored configuration, or explains why it
/// cannot.
///
/// A plain `fn` pointer rather than a boxed closure: every registration is a
/// free function today, and a `fn` stays trivially `Send + Sync`. Swap to
/// `Arc<dyn Fn…>` only when
/// a connector type genuinely needs to capture something.
pub type ConnectorFactory = fn(Value) -> Result<Box<dyn Connector>, ConnectorError>;

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
    /// The configuration this type accepts, as JSON Schema.
    pub schema: Value,
    /// Optional descriptive setup content published with the schema.
    pub setup_guide: Option<SetupGuide>,
    /// Type id this connector can discover through a configured instance.
    pub discoverable_type: Option<String>,
}

/// Every registered type, keyed by [`ConnectorTypeRegistration::type_id`].
///
/// Behind an `Arc` because it is built once at startup and then read by every
/// request; it never changes after construction, so there is nothing to lock.
pub type ConnectorTypeRegistry = Arc<HashMap<&'static str, ConnectorTypeRegistration>>;

/// The types compiled into this build.
///
/// Exactly one today: the debug fixture. Real integrations (Docker, a reverse
/// proxy, a hypervisor) register here alongside it, and nothing else in the
/// backend has to change when they do — that is the point of the indirection.
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
            factory: |config| {
                DebugConnector::from_config_value(config)
                    .map(|connector| Box::new(connector) as Box<dyn Connector>)
            },
            schema: debug.config_schema(),
            setup_guide: debug.setup_guide(),
            discoverable_type: debug.discoverable_type(),
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
    fn every_registration_declares_its_connector_s_own_icon() {
        // The registration copies the icon so the type picker can draw one
        // without an instance. A copy that disagrees with the connector would
        // show one icon in the picker and a different one on the card that the
        // picker just created, which reads as a bug in the icon system rather
        // than as the transcription error it is.
        for registration in builtin_registry().values() {
            let Ok(built) = (registration.factory)(Value::Object(Default::default())) else {
                // A type whose default configuration is not buildable cannot be
                // checked this way. None exist today; skipping is honest.
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
        let guide = registration.setup_guide.as_ref().expect("setup guide");
        assert!(guide.template.contains("{{simulatedHealth}}"));
    }

    #[test]
    fn every_registration_is_keyed_by_its_own_type_id() {
        // A key that disagrees with the registration it holds would make a
        // lookup succeed and then build the wrong connector.
        for (key, registration) in builtin_registry().iter() {
            assert_eq!(*key, registration.type_id);
        }
    }

    #[test]
    fn the_factory_validates_before_it_constructs() {
        let registry = builtin_registry();
        let factory = registry.get(DEBUG_TYPE_ID).expect("registered").factory;

        let Ok(built) = factory(json!({ "baseLoad": 10 })) else {
            panic!("a valid configuration must build");
        };
        assert_eq!(built.metadata().id, DEBUG_TYPE_ID);

        // `Box<dyn Connector>` is not `Debug`, so the happy arm is discarded
        // before the error is examined rather than unwrapped through it.
        let error = factory(json!({ "baseLoad": 900 }))
            .err()
            .expect("an invalid configuration must be refused");
        assert!(matches!(error, ConnectorError::InvalidConfig { .. }));
    }
}
