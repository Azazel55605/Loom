//! Opt-in check against a real local UniFi Network console.
//!
//! Set `LOOM_TEST_UNIFI_NETWORK_HOST`, `LOOM_TEST_UNIFI_NETWORK_API_KEY`, and
//! optionally `LOOM_TEST_UNIFI_NETWORK_SITE` to enable it. Set
//! `LOOM_TEST_UNIFI_NETWORK_ALLOW_INSECURE_CERT=1` only for a deliberately
//! untrusted local certificate.

use std::collections::HashSet;

use loom_connector_unifi_network::{
    UniFiNetworkConnector, ACTION_AUTHORIZE_GUEST, ACTION_CREATE_VOUCHER, ACTION_CYCLE_POE,
    ACTION_RESTART, ACTION_REVOKE_VOUCHER, DATA_POINT_CLIENT_COUNT, DATA_POINT_DEVICE_COUNT,
    DATA_POINT_MODEL, DATA_POINT_ONLINE_DEVICE_COUNT, DATA_POINT_STATE, DATA_POINT_UPTIME,
    RESOURCE_KIND_CLIENTS, RESOURCE_KIND_PORTS, RESOURCE_KIND_VOUCHERS,
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

    let clients = connector
        .list_resource_items(RESOURCE_KIND_CLIENTS, None)
        .await
        .expect("the live site should list connected clients");
    assert!(clients.iter().all(|row| {
        row.fields.get("name").is_some_and(Value::is_string)
            && row.fields.get("mac").is_some_and(Value::is_string)
            && row.fields.get("ipAddress").is_some_and(Value::is_string)
            && row.fields.get("connectedTo").is_some_and(Value::is_string)
            && row.fields.get("isGuest").is_some_and(Value::is_boolean)
            && row.fields.get("authorized").is_some_and(Value::is_boolean)
    }));
    let vouchers = connector
        .list_resource_items(RESOURCE_KIND_VOUCHERS, None)
        .await
        .expect("the live site should list hotspot vouchers");
    assert!(vouchers.iter().all(|row| {
        row.fields.get("code").is_some_and(Value::is_string)
            && row.fields.contains_key("expiresAt")
            && row.fields.contains_key("usesRemaining")
            && row.fields.get("createdAt").is_some_and(Value::is_string)
    }));
    let mut port_count = 0usize;
    for target in &targets {
        let ports = connector
            .list_resource_items(RESOURCE_KIND_PORTS, Some(&target.id))
            .await
            .expect("the live device detail should expose its port collection");
        assert!(ports.iter().all(|row| {
            row.fields.get("port").is_some_and(Value::is_number)
                && row.fields.get("poeEnabled").is_some_and(Value::is_boolean)
                && row.fields.get("linkStatus").is_some_and(Value::is_string)
        }));
        port_count += ports.len();
    }

    let mut contract_targets = vec![None];
    contract_targets.extend(targets.iter().map(|target| Some(target.id.clone())));
    loom_connector_test_kit::assert_connector_contract(&connector, &contract_targets).await;
    eprintln!(
        "{test_name}: {:#}; {} client rows, {} voucher rows, {} port rows",
        status.details,
        clients.len(),
        vouchers.len(),
        port_count
    );
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

#[tokio::test]
#[ignore = "cuts power to a real port; run manually with explicit target, port, and acknowledgement"]
async fn a_real_unifi_port_can_be_power_cycled_only_with_explicit_opt_in() {
    let test_name = "a_real_unifi_port_can_be_power_cycled_only_with_explicit_opt_in";
    assert_eq!(
        std::env::var("LOOM_TEST_UNIFI_NETWORK_ALLOW_POE_CYCLE").as_deref(),
        Ok("1"),
        "set LOOM_TEST_UNIFI_NETWORK_ALLOW_POE_CYCLE=1 only when cutting power to the selected port is safe"
    );
    let raw_target = std::env::var("LOOM_TEST_UNIFI_NETWORK_POE_DEVICE")
        .expect("set LOOM_TEST_UNIFI_NETWORK_POE_DEVICE to the device UUID");
    let target = if raw_target.starts_with("device:") {
        raw_target
    } else {
        format!("device:{raw_target}")
    };
    let port = std::env::var("LOOM_TEST_UNIFI_NETWORK_POE_PORT")
        .expect("set LOOM_TEST_UNIFI_NETWORK_POE_PORT to the port number");
    let connector = live_connector(test_name)
        .await
        .expect("live UniFi Network variables are required for the manual PoE test");

    let result = connector
        .execute_action(ACTION_CYCLE_POE, Some(&target), json!({"resourceId": port}))
        .await
        .expect("the selected PoE-capable port should accept a power-cycle");
    assert!(result.success, "{}", result.message);
    eprintln!("{test_name}: {}", result.message);
}

#[tokio::test]
#[ignore = "changes access for a real guest; run manually with explicit client and acknowledgement"]
async fn a_real_unifi_guest_can_be_authorized_only_with_explicit_opt_in() {
    let test_name = "a_real_unifi_guest_can_be_authorized_only_with_explicit_opt_in";
    assert_eq!(
        std::env::var("LOOM_TEST_UNIFI_NETWORK_ALLOW_GUEST_AUTH").as_deref(),
        Ok("1"),
        "set LOOM_TEST_UNIFI_NETWORK_ALLOW_GUEST_AUTH=1 only for a client whose access may be changed"
    );
    let client_id = std::env::var("LOOM_TEST_UNIFI_NETWORK_GUEST_CLIENT")
        .expect("set LOOM_TEST_UNIFI_NETWORK_GUEST_CLIENT to a connected guest client UUID");
    let connector = live_connector(test_name)
        .await
        .expect("live UniFi Network variables are required for the manual guest test");

    let result = connector
        .execute_action(
            ACTION_AUTHORIZE_GUEST,
            None,
            json!({"resourceId": client_id, "timeLimitMinutes": 15}),
        )
        .await
        .expect("the selected guest should accept authorization");
    assert!(result.success, "{}", result.message);
    eprintln!("{test_name}: {}", result.message);
}

#[tokio::test]
#[ignore = "creates and revokes a real voucher; run manually with explicit acknowledgement"]
async fn a_real_unifi_voucher_can_be_created_and_revoked_with_explicit_opt_in() {
    let test_name = "a_real_unifi_voucher_can_be_created_and_revoked_with_explicit_opt_in";
    assert_eq!(
        std::env::var("LOOM_TEST_UNIFI_NETWORK_ALLOW_VOUCHER_WRITE").as_deref(),
        Ok("1"),
        "set LOOM_TEST_UNIFI_NETWORK_ALLOW_VOUCHER_WRITE=1 only when a temporary voucher is acceptable"
    );
    let connector = live_connector(test_name)
        .await
        .expect("live UniFi Network variables are required for the manual voucher test");
    let before = connector
        .list_resource_items(RESOURCE_KIND_VOUCHERS, None)
        .await
        .expect("list vouchers before creation");
    let before_ids = before
        .iter()
        .map(|voucher| voucher.id.as_str())
        .collect::<HashSet<_>>();

    connector
        .execute_action(
            ACTION_CREATE_VOUCHER,
            None,
            json!({
                "name": "Loom integration test",
                "timeLimitMinutes": 15,
                "authorizedGuestLimit": 1
            }),
        )
        .await
        .expect("create one temporary voucher");
    let after = connector
        .list_resource_items(RESOURCE_KIND_VOUCHERS, None)
        .await
        .expect("list vouchers after creation");
    let created = after
        .iter()
        .find(|voucher| !before_ids.contains(voucher.id.as_str()))
        .expect("the newly created voucher should appear in the listing");
    let created_id = created.id.clone();

    connector
        .execute_action(
            ACTION_REVOKE_VOUCHER,
            None,
            json!({"resourceId": created_id}),
        )
        .await
        .expect("revoke the temporary voucher");
    let final_rows = connector
        .list_resource_items(RESOURCE_KIND_VOUCHERS, None)
        .await
        .expect("list vouchers after revocation");
    assert!(final_rows.iter().all(|voucher| voucher.id != created_id));
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
