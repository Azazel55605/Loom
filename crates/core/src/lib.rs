//! Shared library for Loom.
//!
//! `loom-core` is a library and nothing else. It holds the pieces that more
//! than one part of Loom needs:
//!
//! - the connector trait and its implementations (talking to the services a
//!   homelab actually runs),
//! - business logic that must behave identically no matter which client
//!   triggered it,
//! - and, later, a shared UI component kit consumed by the Tauri desktop and
//!   mobile clients.
//!
//! It deliberately has **no network surface of its own**: no listener, no
//! daemon, no background task that outlives its caller. Anything that needs to
//! be reachable is exposed by `web-backend`, which is the single process that
//! owns auth, access control, and feature management. Keeping that boundary
//! sharp is what makes it safe to link this crate into clients that run on a
//! user's machine.
//!
//! See `docs/ARCHITECTURE.md` and `docs/adr/` for the reasoning.

#![warn(missing_docs)]

/// The connector contract and the types that cross the wire with it.
///
/// A connector is Loom's adapter for one manageable service: it reports that
/// service's health and exposes the actions Loom can ask it to perform. This is
/// the piece that makes Loom a management platform rather than a status page,
/// and it lives in core because the backend and the native clients must agree
/// on it exactly. Includes [`connector::debug::DebugConnector`], a permanent
/// fixture for developing and testing clients with no real services around.
pub mod connector;

/// Returns the version of `loom-core` this binary was built against.
///
/// This is the crate's `CARGO_PKG_VERSION`, baked in at compile time. It exists
/// so a client can report which core it is running, and so the wiring between
/// crates is exercised by something real.
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_matches_cargo_manifest() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
        assert!(!version().is_empty());
    }
}
