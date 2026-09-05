//! Opt-in checks against a real Tasmota smart plug.
//!
//! Set `LOOM_TEST_TASMOTA_HOST` to enable the read-only test. If the device has
//! a web password, also set `LOOM_TEST_TASMOTA_PASSWORD`. The power test is
//! ignored and additionally requires `LOOM_TEST_TASMOTA_ALLOW_POWER_TOGGLE=1`:
//! it changes a real relay briefly, then makes a best-effort restoration.

use loom_connector_tasmota::{
    TasmotaConnector, ACTION_SET_POWER, DATA_POINT_FIRMWARE_VERSION, DATA_POINT_POWER_STATE,
    DATA_POINT_UPTIME, DATA_POINT_WIFI_SIGNAL_PERCENT,
};
use loom_core::connector::{Connector, HealthState};
use serde_json::{json, Value};

#[tokio::test]
async fn a_real_tasmota_reports_status_and_obeys_the_contract() {
    let test_name = "a_real_tasmota_reports_status_and_obeys_the_contract";
    let Some(connector) = live_connector(test_name).await else {
        return;
    };
    let status = connector
        .status()
        .await
        .expect("status call should complete");
    assert_eq!(status.health, HealthState::Healthy, "{:#}", status.details);
    assert!(status
        .data_point_value(DATA_POINT_POWER_STATE)
        .is_some_and(Value::is_boolean));
    assert!(status
        .data_point_value(DATA_POINT_WIFI_SIGNAL_PERCENT)
        .and_then(Value::as_f64)
        .is_some_and(|value| (0.0..=100.0).contains(&value)));
    assert!(status
        .data_point_value(DATA_POINT_UPTIME)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty()));
    assert!(status
        .data_point_value(DATA_POINT_FIRMWARE_VERSION)
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty()));

    loom_connector_test_kit::assert_connector_contract(&connector, &[None]).await;
    eprintln!("{test_name}: {:#}", status.details);
}

#[tokio::test]
#[ignore = "changes a real relay; run manually with LOOM_TEST_TASMOTA_ALLOW_POWER_TOGGLE=1"]
async fn a_real_tasmota_can_change_power_and_restore_the_original_state() {
    let test_name = "a_real_tasmota_can_change_power_and_restore_the_original_state";
    assert_eq!(
        std::env::var("LOOM_TEST_TASMOTA_ALLOW_POWER_TOGGLE").as_deref(),
        Ok("1"),
        "set LOOM_TEST_TASMOTA_ALLOW_POWER_TOGGLE=1 only when briefly changing the attached load is safe"
    );
    let connector = live_connector(test_name)
        .await
        .expect("live Tasmota variables are required for the manual power test");
    let original = power_state(&connector).await;
    let changed_to = !original;

    let changed = connector
        .execute_action(ACTION_SET_POWER, None, json!({ "on": changed_to }))
        .await;
    let observed_changed = power_state(&connector).await;
    let restored = connector
        .execute_action(ACTION_SET_POWER, None, json!({ "on": original }))
        .await;
    let observed_restored = power_state(&connector).await;

    changed.expect("Tasmota should accept the temporary state");
    restored.expect("Tasmota should restore the original state");
    assert_eq!(observed_changed, changed_to);
    assert_eq!(observed_restored, original);
}

async fn live_connector(test_name: &str) -> Option<TasmotaConnector> {
    let Ok(host) = std::env::var("LOOM_TEST_TASMOTA_HOST") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_TASMOTA_HOST is not set");
        return None;
    };
    let password = std::env::var("LOOM_TEST_TASMOTA_PASSWORD").ok();
    let config = match password {
        Some(password) => json!({ "host": host, "password": password }),
        None => json!({ "host": host }),
    };
    match TasmotaConnector::from_config_value(config).await {
        Ok(connector) => Some(connector),
        Err(error) => {
            eprintln!("SKIPPING {test_name}: configured Tasmota is unavailable: {error}");
            None
        }
    }
}

async fn power_state(connector: &TasmotaConnector) -> bool {
    connector
        .status()
        .await
        .expect("status call")
        .data_point_value(DATA_POINT_POWER_STATE)
        .and_then(Value::as_bool)
        .expect("powerState status value")
}
