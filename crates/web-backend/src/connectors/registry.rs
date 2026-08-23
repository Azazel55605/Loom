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
use loom_core::connector::{Connector, ConnectorError};
use serde_json::Value;

/// Builds a live connector from a stored configuration, or explains why it
/// cannot.
///
/// A plain `fn` pointer rather than a boxed closure: every registration is a
/// free function today, and a `fn` keeps [`ConnectorTypeRegistration`] `Copy`-
/// cheap to clone and trivially `Send + Sync`. Swap to `Arc<dyn Fn…>` only when
/// a connector type genuinely needs to capture something.
pub type ConnectorFactory = fn(Value) -> Result<Box<dyn Connector>, ConnectorError>;

/// Returns a type's configuration schema without constructing an instance.
///
/// Separate from the factory because the "add connector" form needs the schema
/// *before* there is any configuration to build from —
/// [`Connector::config_schema`] is an instance method, and requiring an
/// instance to ask what an instance needs is a chicken-and-egg the frontend
/// should not have to solve.
pub type ConnectorSchemaFn = fn() -> Value;

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
    /// Duplicated from the connector's own `metadata()` rather than read
    /// through it, because the type picker draws an icon *before* any instance
    /// exists and `metadata()` is an instance method — the same chicken-and-egg
    /// [`ConnectorSchemaFn`] exists to solve. `every_registration_declares_its_
    /// connector_s_own_icon` below is what keeps the copy honest.
    pub icon: Option<&'static str>,
    /// Turns stored configuration into a live connector.
    pub factory: ConnectorFactory,
    /// The configuration this type accepts, as JSON Schema.
    pub schema: ConnectorSchemaFn,
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

    types.insert(
        loom_core::connector::debug::TYPE_ID,
        ConnectorTypeRegistration {
            type_id: loom_core::connector::debug::TYPE_ID,
            display_name: "Debug Connector",
            icon: Some("lucide:bug"),
            factory: |config| {
                DebugConnector::from_config_value(config)
                    .map(|connector| Box::new(connector) as Box<dyn Connector>)
            },
            // Constructing a default fixture to read its schema off is cheap
            // and allocates nothing the connector holds onto. A `const` schema
            // would duplicate the document that already lives next to the
            // parser that enforces it, which is exactly how the two drift.
            schema: || DebugConnector::default().config_schema(),
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

        let schema = (registration.schema)();
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
                registration.icon,
                "{} registers an icon its connector does not declare",
                registration.type_id
            );
        }
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
