//! One end-to-end check against a real container registry.
//!
//! The digest-check *flow* — challenge, token, retry, header — is unit-tested
//! against canned responses in `src/registry.rs`, because that is the only way
//! to reach a 429 or a malformed challenge on demand. What a fake cannot prove
//! is that the real request is well-formed: that the `Accept` header is one a
//! registry will answer, that the token round-trip works against a live token
//! service, and that the digest arrives in the header this code reads.
//!
//! # Skipping
//!
//! **Opt-in, not opt-out.** Unlike the Docker tests, which skip only when no
//! daemon is there, this one runs only when `LOOM_TEST_REGISTRY=1` is set. A
//! test suite that reaches a third party on every `cargo test` makes someone
//! else's availability part of this project's build, and spends their rate
//! limit to tell us something that has not changed since the last run.
//!
//! ```sh
//! LOOM_TEST_REGISTRY=1 cargo test -p loom-connector-docker --test live_registry -- --nocapture
//! ```

use loom_connector_docker::{current_digest, ImageReference};

/// A small, stable, public official image. Official images are the case worth
/// checking, because they are the one that needs Docker Hub's `library/`
/// namespacing.
const REFERENCE: &str = "alpine:3.20";

#[tokio::test]
async fn a_real_registry_answers_with_a_digest() {
    let test_name = "a_real_registry_answers_with_a_digest";
    if std::env::var("LOOM_TEST_REGISTRY").as_deref() != Ok("1") {
        eprintln!("SKIPPING {test_name}: set LOOM_TEST_REGISTRY=1 to query a real registry");
        return;
    }

    let transport = loom_connector_docker::http_registry().expect("building an HTTPS client");
    let reference = ImageReference::parse(REFERENCE).expect("a parseable reference");

    let digest = current_digest(transport.as_ref(), &reference)
        .await
        .expect("a public image's digest must be readable anonymously");

    // The shape, not the value: the value changes whenever the tag is rebuilt,
    // and a test asserting today's digest would fail on someone else's release
    // schedule.
    assert!(
        digest.starts_with("sha256:") && digest.len() > 20,
        "unexpected digest shape: {digest}"
    );
    eprintln!("{test_name}: {REFERENCE} is currently {digest}");
}
