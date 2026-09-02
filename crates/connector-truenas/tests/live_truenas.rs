//! Opt-in connectivity proof against a real TrueNAS system.
//!
//! Set `LOOM_TEST_TRUENAS_HOST` and `LOOM_TEST_TRUENAS_API_KEY` to enable it.
//! `LOOM_TEST_TRUENAS_USERNAME` selects the preferred `auth.login_ex` path;
//! without it, the stable key-only compatibility method is used. Set
//! `LOOM_TEST_TRUENAS_ALLOW_INSECURE_CERT=1` only for a self-signed test system.

use loom_connector_truenas::TrueNasClient;
use serde_json::{json, Value};

#[tokio::test]
async fn a_real_truenas_answers_core_ping() {
    let test_name = "a_real_truenas_answers_core_ping";
    let Ok(host) = std::env::var("LOOM_TEST_TRUENAS_HOST") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_TRUENAS_HOST is not set");
        return;
    };
    let Ok(api_key) = std::env::var("LOOM_TEST_TRUENAS_API_KEY") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_TRUENAS_API_KEY is not set");
        return;
    };
    let allow_insecure_cert =
        std::env::var("LOOM_TEST_TRUENAS_ALLOW_INSECURE_CERT").as_deref() == Ok("1");

    let connected = match std::env::var("LOOM_TEST_TRUENAS_USERNAME") {
        Ok(username) => {
            TrueNasClient::connect_with_username(&host, &username, &api_key, allow_insecure_cert)
                .await
        }
        Err(_) => TrueNasClient::connect(&host, &api_key, allow_insecure_cert).await,
    };
    let client = match connected {
        Ok(client) => client,
        Err(error) => {
            eprintln!("SKIPPING {test_name}: configured TrueNAS is unavailable: {error}");
            return;
        }
    };

    let response = client
        .call("core.ping", Value::Array(Vec::new()))
        .await
        .expect("an authenticated live TrueNAS connection should answer core.ping");

    assert_eq!(response, json!("pong"));
    eprintln!("{test_name}: core.ping returned {response}");
}
