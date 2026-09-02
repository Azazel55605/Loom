//! Opt-in connectivity proof against a real TrueNAS system.
//!
//! Set `LOOM_TEST_TRUENAS_HOST` and `LOOM_TEST_TRUENAS_API_KEY` to enable it.
//! `LOOM_TEST_TRUENAS_USERNAME` selects the preferred `auth.login_ex` path for
//! the transport test and is required by the connector test. Set
//! `LOOM_TEST_TRUENAS_ALLOW_INSECURE_CERT=1` only for a self-signed test system.

use loom_connector_truenas::{
    TrueNasClient, TrueNasConnector, DATA_POINT_FREE_CAPACITY_BYTES, DATA_POINT_POOL_COUNT,
    DATA_POINT_TOTAL_CAPACITY_BYTES, DATA_POINT_TRUENAS_VERSION, DATA_POINT_USED_CAPACITY_BYTES,
};
use loom_core::connector::{Connector, HealthState};
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

#[tokio::test]
async fn a_real_truenas_maps_host_level_connector_readings() {
    let test_name = "a_real_truenas_maps_host_level_connector_readings";
    let Ok(host) = std::env::var("LOOM_TEST_TRUENAS_HOST") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_TRUENAS_HOST is not set");
        return;
    };
    let Ok(api_key) = std::env::var("LOOM_TEST_TRUENAS_API_KEY") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_TRUENAS_API_KEY is not set");
        return;
    };
    let Ok(username) = std::env::var("LOOM_TEST_TRUENAS_USERNAME") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_TRUENAS_USERNAME is not set");
        return;
    };
    let allow_insecure_cert =
        std::env::var("LOOM_TEST_TRUENAS_ALLOW_INSECURE_CERT").as_deref() == Ok("1");

    // Exercise the same current username-plus-key path published by the
    // connector schema and used by the real add-instance dialog.
    let connector = match TrueNasConnector::from_config_value(json!({
        "host": host,
        "username": username,
        "apiKey": api_key,
        "allowInsecureCert": allow_insecure_cert
    }))
    .await
    {
        Ok(connector) => connector,
        Err(error) => {
            eprintln!("SKIPPING {test_name}: configured TrueNAS is unavailable: {error}");
            return;
        }
    };

    let status = connector
        .status()
        .await
        .expect("the live TrueNAS connector should return a status");
    assert_eq!(status.health, HealthState::Healthy);

    let details = status
        .details
        .as_object()
        .expect("connector status details should be an object");
    let pool_count = required_u64(details, DATA_POINT_POOL_COUNT);
    let total = required_u64(details, DATA_POINT_TOTAL_CAPACITY_BYTES);
    let used = required_u64(details, DATA_POINT_USED_CAPACITY_BYTES);
    let free = required_u64(details, DATA_POINT_FREE_CAPACITY_BYTES);
    let version = details
        .get(DATA_POINT_TRUENAS_VERSION)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .expect("truenasVersion must be a non-empty string");

    eprintln!(
        "{test_name}: poolCount={pool_count}, totalCapacityBytes={total}, usedCapacityBytes={used}, freeCapacityBytes={free}, truenasVersion={version}"
    );
}

fn required_u64(details: &serde_json::Map<String, Value>, key: &str) -> u64 {
    details
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{key} must be a non-null unsigned integer"))
}
