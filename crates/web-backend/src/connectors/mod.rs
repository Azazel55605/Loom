//! The connector type registry and the live-instance runtime.
//!
//! Two halves of one idea, kept in one module because neither is useful alone:
//!
//! - The **registry** ([`registry`]) is code. It answers "what kinds of
//!   connector does this build know how to make?" and, for each kind, how to
//!   build one from a JSON blob and what that blob is allowed to contain.
//! - The **runtime** ([`runtime`]) is state. It answers "which connectors does
//!   *this instance* actually have?", holding one live
//!   [`Connector`](loom_core::connector::Connector) per row in
//!   `connector_instances`.
//!
//! The split is what lets "add a connector" be a data operation rather than a
//! code change: adding a *type* is a registration in Rust, but adding an
//! *instance* of an already-registered type is an INSERT plus a factory call,
//! driven entirely by a form the frontend generates from the published schema.
//!
//! See `docs/adr/0011-connector-instance-registry.md`.

pub mod diagnostics;
pub mod registry;
pub mod runtime;

pub use registry::builtin_registry;
pub use runtime::ConnectorRuntime;
