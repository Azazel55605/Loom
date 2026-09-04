//! Opt-in check against a real local UniFi Network console.
//!
//! Set `LOOM_TEST_UNIFI_NETWORK_HOST`, `LOOM_TEST_UNIFI_NETWORK_API_KEY`, and
//! optionally `LOOM_TEST_UNIFI_NETWORK_SITE` to enable it. Set
//! `LOOM_TEST_UNIFI_NETWORK_ALLOW_INSECURE_CERT=1` only for a deliberately
//! untrusted local certificate.

use loom_connector_unifi_network::{
    UniFiNetworkConnector, ACTION_RESTART, DATA_POINT_CLIENT_COUNT, DATA_POINT_DEVICE_COUNT,
    DATA_POINT_MODEL, DATA_POINT_ONLINE_DEVICE_COUNT, DATA_POINT_STATE, DATA_POINT_UPTIME,
};
use loom_core::connector::{Connector, HealthState};
use serde_json::{json, Value};

#[tokio::test]
async fn a_real_unifi_console_reports_site_counts_and_obeys_the_contract() {
    let test_name = "a_real_unifi_console_reports_site_counts_and_obeys_the_contract";
    let Some(connector) = live_connector(test_name).await else {
        return;
    };

    let status = connector
        .status()
        .await
        .expect("the live UniFi Network status call should complete");
    assert_eq!(status.health, HealthState::Healthy, "{:#}", status.details);
    for id in [
        DATA_POINT_DEVICE_COUNT,
        DATA_POINT_ONLINE_DEVICE_COUNT,
        DATA_POINT_CLIENT_COUNT,
    ] {
        assert!(
            status
                .data_point_value(id)
                .and_then(Value::as_u64)
                .is_some(),
            "{id} should be a non-negative integer: {:#}",
            status.details
        );
    }
    let total = status
        .data_point_value(DATA_POINT_DEVICE_COUNT)
        .and_then(Value::as_u64)
        .expect("device count");
    let online = status
        .data_point_value(DATA_POINT_ONLINE_DEVICE_COUNT)
        .and_then(Value::as_u64)
        .expect("online device count");
    assert!(online <= total);

    let targets = connector
        .list_sub_targets()
        .await
        .expect("the live UniFi Network site should list devices");
    assert!(
        !targets.is_empty(),
        "the configured site should have devices"
    );
    assert!(targets
        .iter()
        .all(|target| target.kind == "device" && target.id.starts_with("device:")));
    for target in &targets {
        assert!(status
            .data_point_value_for(Some(&target.id), DATA_POINT_STATE)
            .is_some_and(Value::is_string));
        assert!(status
            .data_point_value_for(Some(&target.id), DATA_POINT_MODEL)
            .is_some_and(Value::is_string));
        assert!(status
            .data_point_value_for(Some(&target.id), DATA_POINT_UPTIME)
            .is_some_and(Value::is_string));
        assert!(status.target_health.contains_key(&target.id));
    }

    let mut contract_targets = vec![None];
    contract_targets.extend(targets.iter().map(|target| Some(target.id.clone())));
    loom_connector_test_kit::assert_connector_contract(&connector, &contract_targets).await;
    eprintln!("{test_name}: {:#}", status.details);
}

#[tokio::test]
#[ignore = "restarts a real network device; run manually with explicit target and acknowledgement"]
async fn a_real_unifi_device_can_be_restarted_only_with_explicit_opt_in() {
    let test_name = "a_real_unifi_device_can_be_restarted_only_with_explicit_opt_in";
    assert_eq!(
        std::env::var("LOOM_TEST_UNIFI_NETWORK_ALLOW_RESTART").as_deref(),
        Ok("1"),
        "set LOOM_TEST_UNIFI_NETWORK_ALLOW_RESTART=1 only when disconnecting the selected device is safe"
    );
    let raw_target = std::env::var("LOOM_TEST_UNIFI_NETWORK_RESTART_TARGET")
        .expect("set LOOM_TEST_UNIFI_NETWORK_RESTART_TARGET to a disposable device UUID");
    let target = if raw_target.starts_with("device:") {
        raw_target
    } else {
        format!("device:{raw_target}")
    };
    let connector = live_connector(test_name)
        .await
        .expect("live UniFi Network variables are required for the manual restart test");

    let result = connector
        .execute_action(ACTION_RESTART, Some(&target), json!({}))
        .await
        .expect("the selected device should accept the restart request");
    assert!(result.success, "{}", result.message);
    eprintln!("{test_name}: {}", result.message);
}

async fn live_connector(test_name: &str) -> Option<UniFiNetworkConnector> {
    let Ok(host) = std::env::var("LOOM_TEST_UNIFI_NETWORK_HOST") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_UNIFI_NETWORK_HOST is not set");
        return None;
    };
    let Ok(api_key) = std::env::var("LOOM_TEST_UNIFI_NETWORK_API_KEY") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_UNIFI_NETWORK_API_KEY is not set");
        return None;
    };
    let site =
        std::env::var("LOOM_TEST_UNIFI_NETWORK_SITE").unwrap_or_else(|_| "default".to_owned());
    let allow_insecure_cert =
        std::env::var("LOOM_TEST_UNIFI_NETWORK_ALLOW_INSECURE_CERT").as_deref() == Ok("1");

    match UniFiNetworkConnector::from_config_value(json!({
        "host": host,
        "apiKey": api_key,
        "site": site,
        "allowInsecureCert": allow_insecure_cert
    }))
    .await
    {
        Ok(connector) => Some(connector),
        Err(error) => panic!("{test_name}: could not connect: {error}"),
    }
}
