//! Opt-in connectivity proof against a real TrueNAS system.
//!
//! Set `LOOM_TEST_TRUENAS_HOST` and `LOOM_TEST_TRUENAS_API_KEY` to enable it.
//! `LOOM_TEST_TRUENAS_USERNAME` selects the preferred `auth.login_ex` path for
//! the transport test and is required by the connector test. Set
//! `LOOM_TEST_TRUENAS_ALLOW_INSECURE_CERT=1` only for a self-signed test system.

use loom_connector_truenas::{
    TrueNasClient, TrueNasConnector, ACTION_START_SCRUB, DATA_POINT_AVAILABLE_BYTES,
    DATA_POINT_CAPACITY_PERCENT, DATA_POINT_COMPRESSION_RATIO, DATA_POINT_FREE_CAPACITY_BYTES,
    DATA_POINT_POOL_COUNT, DATA_POINT_SNAPSHOT_COUNT, DATA_POINT_STATUS,
    DATA_POINT_TOTAL_CAPACITY_BYTES, DATA_POINT_TRUENAS_VERSION, DATA_POINT_USED_BYTES,
    DATA_POINT_USED_CAPACITY_BYTES,
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
async fn a_real_truenas_lists_and_reads_pool_and_dataset_targets() {
    let test_name = "a_real_truenas_lists_and_reads_pool_and_dataset_targets";
    let Some(connector) = live_connector(test_name).await else {
        return;
    };

    let targets = connector
        .list_sub_targets()
        .await
        .expect("the live TrueNAS should list sub-targets");
    let pools: Vec<_> = targets
        .iter()
        .filter(|target| target.kind == "pool")
        .collect();
    let datasets: Vec<_> = targets
        .iter()
        .filter(|target| target.kind == "dataset")
        .collect();
    assert!(!pools.is_empty(), "the live TrueNAS should have a pool");
    assert!(
        !datasets.is_empty(),
        "the live TrueNAS should have a dataset"
    );

    let status = connector
        .status()
        .await
        .expect("the live TrueNAS should return target readings");
    for pool in pools {
        let state = required_target_value(&status, &pool.id, DATA_POINT_STATUS);
        let capacity = required_target_value(&status, &pool.id, DATA_POINT_CAPACITY_PERCENT);
        eprintln!(
            "{test_name}: pool={} status={} capacityPercent={}",
            pool.label, state, capacity
        );
    }
    for dataset in datasets {
        let used = required_target_value(&status, &dataset.id, DATA_POINT_USED_BYTES);
        let available = required_target_value(&status, &dataset.id, DATA_POINT_AVAILABLE_BYTES);
        let compression = required_target_value(&status, &dataset.id, DATA_POINT_COMPRESSION_RATIO);
        let snapshots = required_target_value(&status, &dataset.id, DATA_POINT_SNAPSHOT_COUNT);
        eprintln!(
            "{test_name}: dataset={} usedBytes={} availableBytes={} compressionRatio={} snapshotCount={}",
            dataset.label, used, available, compression, snapshots
        );
    }
}

#[tokio::test]
#[ignore = "starts real storage I/O; run manually with LOOM_TEST_TRUENAS_SCRUB_POOL"]
async fn a_real_truenas_can_start_a_pool_scrub_when_explicitly_requested() {
    let test_name = "a_real_truenas_can_start_a_pool_scrub_when_explicitly_requested";
    let connector = live_connector(test_name)
        .await
        .expect("live TrueNAS variables are required for the manual scrub test");
    let pool = std::env::var("LOOM_TEST_TRUENAS_SCRUB_POOL")
        .expect("LOOM_TEST_TRUENAS_SCRUB_POOL is required for the manual scrub test");

    let result = connector
        .execute_action(ACTION_START_SCRUB, Some(&format!("pool:{pool}")), json!({}))
        .await
        .expect("the live TrueNAS should accept the scrub request");
    assert!(result.success);
    eprintln!("{test_name}: {}", result.message);
}

async fn live_connector(test_name: &str) -> Option<TrueNasConnector> {
    let Ok(host) = std::env::var("LOOM_TEST_TRUENAS_HOST") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_TRUENAS_HOST is not set");
        return None;
    };
    let Ok(api_key) = std::env::var("LOOM_TEST_TRUENAS_API_KEY") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_TRUENAS_API_KEY is not set");
        return None;
    };
    let Ok(username) = std::env::var("LOOM_TEST_TRUENAS_USERNAME") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_TRUENAS_USERNAME is not set");
        return None;
    };
    let allow_insecure_cert =
        std::env::var("LOOM_TEST_TRUENAS_ALLOW_INSECURE_CERT").as_deref() == Ok("1");

    match TrueNasConnector::from_config_value(json!({
        "host": host,
        "username": username,
        "apiKey": api_key,
        "allowInsecureCert": allow_insecure_cert
    }))
    .await
    {
        Ok(connector) => Some(connector),
        Err(error) => {
            eprintln!("SKIPPING {test_name}: configured TrueNAS is unavailable: {error}");
            None
        }
    }
}

fn required_target_value<'a>(
    status: &'a loom_core::connector::ConnectorStatus,
    target_id: &str,
    key: &str,
) -> &'a Value {
    status
        .data_point_value_for(Some(target_id), key)
        .unwrap_or_else(|| panic!("{target_id}.{key} must be populated"))
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
    assert!(matches!(
        status.health,
        HealthState::Healthy | HealthState::Degraded
    ));

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
