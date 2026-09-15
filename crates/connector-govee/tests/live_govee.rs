//! Opt-in checks against a real Govee account.
//!
//! Set `LOOM_TEST_GOVEE_API_KEY` to enable the read-only account/state and
//! contract check. Effectful tests are ignored and require an explicit device
//! id plus their own acknowledgement variable because they visibly change a
//! real light.

use loom_connector_govee::GoveeConnector;
use loom_core::connector::{Connector, HealthState};
use serde_json::{json, Value};

#[tokio::test]
async fn a_real_govee_account_reports_devices_and_obeys_the_contract() {
    let test_name = "a_real_govee_account_reports_devices_and_obeys_the_contract";
    let Some(connector) = live_connector(test_name).await else {
        return;
    };
    let targets = connector
        .list_sub_targets()
        .await
        .expect("device list should succeed");
    let status = connector.status().await.expect("status should complete");
    assert!(
        matches!(status.health, HealthState::Healthy | HealthState::Degraded),
        "{:#}",
        status.details
    );
    assert_eq!(
        status
            .data_point_value("deviceCount")
            .and_then(Value::as_u64),
        Some(targets.len() as u64)
    );
    let mut scopes = vec![None];
    scopes.extend(targets.iter().map(|target| Some(target.id.clone())));
    loom_connector_test_kit::assert_connector_contract(&connector, &scopes).await;
    eprintln!("{test_name}: verified {} device target(s)", targets.len());
}

#[tokio::test]
#[ignore = "changes a real light; requires explicit device and acknowledgement"]
async fn a_real_govee_light_can_toggle_power_and_restore_it() {
    assert_opt_in("LOOM_TEST_GOVEE_ALLOW_POWER_TOGGLE");
    let connector = live_connector("a_real_govee_light_can_toggle_power_and_restore_it")
        .await
        .expect("LOOM_TEST_GOVEE_API_KEY is required");
    let target = selected_target();
    let original = current_value(&connector, &target, "powerState")
        .await
        .as_bool()
        .expect("selected device must report powerState");
    connector
        .execute_action("setPower", Some(&target), json!({ "on": !original }))
        .await
        .expect("temporary power change");
    connector
        .execute_action("setPower", Some(&target), json!({ "on": original }))
        .await
        .expect("restore original power state");
}

#[tokio::test]
#[ignore = "changes a real light; requires explicit device, brightness, and acknowledgement"]
async fn a_real_govee_light_accepts_an_in_range_brightness() {
    assert_opt_in("LOOM_TEST_GOVEE_ALLOW_BRIGHTNESS_CHANGE");
    let connector = live_connector("a_real_govee_light_accepts_an_in_range_brightness")
        .await
        .expect("LOOM_TEST_GOVEE_API_KEY is required");
    let target = selected_target();
    let brightness: f64 = std::env::var("LOOM_TEST_GOVEE_BRIGHTNESS")
        .expect("set an in-range LOOM_TEST_GOVEE_BRIGHTNESS")
        .parse()
        .expect("brightness must be numeric");
    connector
        .execute_action(
            "setBrightness",
            Some(&target),
            json!({ "brightness": brightness }),
        )
        .await
        .expect("brightness change");
}

#[tokio::test]
#[ignore = "changes a real light; requires explicit device, colour, and acknowledgement"]
async fn a_real_govee_light_accepts_a_hex_colour() {
    assert_opt_in("LOOM_TEST_GOVEE_ALLOW_COLOR_CHANGE");
    let connector = live_connector("a_real_govee_light_accepts_a_hex_colour")
        .await
        .expect("LOOM_TEST_GOVEE_API_KEY is required");
    let target = selected_target();
    let color = std::env::var("LOOM_TEST_GOVEE_COLOR_HEX")
        .expect("set LOOM_TEST_GOVEE_COLOR_HEX to a deliberate #RRGGBB colour");
    connector
        .execute_action("setColor", Some(&target), json!({ "colorHex": color }))
        .await
        .expect("colour change");
}

async fn live_connector(test_name: &str) -> Option<GoveeConnector> {
    let Ok(api_key) = std::env::var("LOOM_TEST_GOVEE_API_KEY") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_GOVEE_API_KEY is not set");
        return None;
    };
    match GoveeConnector::from_config_value(json!({ "apiKey": api_key })).await {
        Ok(connector) => Some(connector),
        Err(error) => {
            eprintln!("SKIPPING {test_name}: configured Govee account is unavailable: {error}");
            None
        }
    }
}

async fn current_value(connector: &GoveeConnector, target: &str, point: &str) -> Value {
    connector
        .status()
        .await
        .expect("status")
        .data_point_value_for(Some(target), point)
        .cloned()
        .expect("selected data point")
}

fn selected_target() -> String {
    let id = std::env::var("LOOM_TEST_GOVEE_DEVICE_ID")
        .expect("set LOOM_TEST_GOVEE_DEVICE_ID to the deliberate test light");
    if id.starts_with("device:") {
        id
    } else {
        format!("device:{id}")
    }
}

fn assert_opt_in(variable: &str) {
    assert_eq!(
        std::env::var(variable).as_deref(),
        Ok("1"),
        "set {variable}=1 only when changing the selected real light is safe"
    );
}
