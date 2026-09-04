//! Opt-in check against a real local UniFi Network console.
//!
//! Set `LOOM_TEST_UNIFI_NETWORK_HOST`, `LOOM_TEST_UNIFI_NETWORK_API_KEY`, and
//! optionally `LOOM_TEST_UNIFI_NETWORK_SITE` to enable it. Set
//! `LOOM_TEST_UNIFI_NETWORK_ALLOW_INSECURE_CERT=1` only for a deliberately
//! untrusted local certificate. Set
//! `LOOM_TEST_UNIFI_NETWORK_EXPECTED_DEVICE_COUNT` to make the pagination
//! regression check assert the controller's known inventory size.

use std::{
    collections::HashSet,
    time::{SystemTime, UNIX_EPOCH},
};

use loom_connector_unifi_network::{
    UniFiNetworkConnector, ACTION_ADOPT, ACTION_AUTHORIZE_GUEST, ACTION_CREATE_A_RECORD,
    ACTION_CREATE_VOUCHER, ACTION_CYCLE_POE, ACTION_DELETE_DNS_POLICY, ACTION_RESTART,
    ACTION_REVOKE_VOUCHER, ACTION_TOGGLE_FIREWALL_LOGGING, ACTION_TOGGLE_WLAN_ENABLED,
    ACTION_UNAUTHORIZE_GUEST, DATA_POINT_CLIENT_COUNT, DATA_POINT_DEVICE_COUNT,
    DATA_POINT_LAST_HEARTBEAT_AT, DATA_POINT_LOAD_AVERAGE_15M, DATA_POINT_LOAD_AVERAGE_1M,
    DATA_POINT_LOAD_AVERAGE_5M, DATA_POINT_MODEL, DATA_POINT_ONLINE_DEVICE_COUNT,
    DATA_POINT_RADIO_TX_RETRY_PERCENT, DATA_POINT_STATE, DATA_POINT_UPTIME, DATA_POINT_WAN_COUNT,
    RESOURCE_KIND_ACL_RULES, RESOURCE_KIND_CLIENTS, RESOURCE_KIND_DNS_POLICIES,
    RESOURCE_KIND_FIREWALL_POLICIES, RESOURCE_KIND_FIREWALL_ZONES, RESOURCE_KIND_NETWORKS,
    RESOURCE_KIND_PENDING_DEVICES, RESOURCE_KIND_PORTS, RESOURCE_KIND_VOUCHERS, RESOURCE_KIND_WANS,
    RESOURCE_KIND_WLAN_BROADCASTS,
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
        DATA_POINT_WAN_COUNT,
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
    assert!(targets.iter().all(|target| target.icon.is_some()));
    if let Ok(expected) = std::env::var("LOOM_TEST_UNIFI_NETWORK_EXPECTED_DEVICE_COUNT") {
        let expected = expected
            .parse::<usize>()
            .expect("LOOM_TEST_UNIFI_NETWORK_EXPECTED_DEVICE_COUNT must be a non-negative integer");
        assert_eq!(
            targets.len(),
            expected,
            "the configured site returned these targets instead: {targets:#?}"
        );
    }
    let mut saw_load_averages = false;
    let mut saw_heartbeat = false;
    let mut saw_radio_retries = false;
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
        saw_load_averages |= [
            DATA_POINT_LOAD_AVERAGE_1M,
            DATA_POINT_LOAD_AVERAGE_5M,
            DATA_POINT_LOAD_AVERAGE_15M,
        ]
        .into_iter()
        .all(|id| {
            status
                .data_point_value_for(Some(&target.id), id)
                .is_some_and(Value::is_number)
        });
        saw_heartbeat |= status
            .data_point_value_for(Some(&target.id), DATA_POINT_LAST_HEARTBEAT_AT)
            .is_some_and(Value::is_string);
        saw_radio_retries |= status
            .data_point_value_for(Some(&target.id), DATA_POINT_RADIO_TX_RETRY_PERCENT)
            .is_some_and(Value::is_number);
    }
    assert!(
        saw_load_averages,
        "at least one live device should expose load averages"
    );
    assert!(
        saw_heartbeat,
        "at least one live device should expose a heartbeat timestamp"
    );
    assert!(
        saw_radio_retries,
        "at least one live AP should expose radio retry statistics"
    );

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
    let wans = connector
        .list_resource_items(RESOURCE_KIND_WANS, None)
        .await
        .expect("the live site should list WAN definitions");
    assert!(wans.iter().all(|row| {
        row.fields.get("name").is_some_and(Value::is_string)
            && row.fields.get("id").is_some_and(Value::is_string)
    }));
    let pending_devices = connector
        .list_resource_items(RESOURCE_KIND_PENDING_DEVICES, None)
        .await
        .expect("the console should list pending devices, even when empty");
    assert!(pending_devices.iter().all(|row| {
        row.fields.get("model").is_some_and(Value::is_string)
            && row.fields.get("macAddress").is_some_and(Value::is_string)
            && row.fields.get("state").is_some_and(Value::is_string)
            && row.fields.contains_key("firmwareVersion")
    }));
    let acl_rules = connector
        .list_resource_items(RESOURCE_KIND_ACL_RULES, None)
        .await
        .expect("the live site should list ACL rules");
    assert!(acl_rules.iter().all(|row| {
        row.fields.get("name").is_some_and(Value::is_string)
            && row.fields.get("type").is_some_and(Value::is_string)
            && row.fields.get("action").is_some_and(Value::is_string)
            && row.fields.get("enabled").is_some_and(Value::is_boolean)
    }));
    let dns_policies = connector
        .list_resource_items(RESOURCE_KIND_DNS_POLICIES, None)
        .await
        .expect("the live site should list DNS policies");
    assert!(dns_policies.iter().all(|row| {
        row.fields.get("type").is_some_and(Value::is_string)
            && row.fields.get("domain").is_some_and(Value::is_string)
            && row.fields.get("target").is_some_and(Value::is_string)
            && row.fields.get("enabled").is_some_and(Value::is_boolean)
    }));
    let firewall_zones = connector
        .list_resource_items(RESOURCE_KIND_FIREWALL_ZONES, None)
        .await
        .expect("the live site should list firewall zones");
    assert!(firewall_zones.iter().all(|row| {
        row.fields.get("name").is_some_and(Value::is_string)
            && row.fields.get("networks").is_some_and(Value::is_string)
            && row
                .fields
                .get("systemDerived")
                .is_some_and(Value::is_boolean)
    }));
    let firewall_policies = connector
        .list_resource_items(RESOURCE_KIND_FIREWALL_POLICIES, None)
        .await
        .expect("the live site should list firewall policies");
    assert!(firewall_policies.iter().all(|row| {
        ["name", "action", "sourceZone", "destinationZone"]
            .into_iter()
            .all(|key| row.fields.get(key).is_some_and(Value::is_string))
            && ["enabled", "loggingEnabled"]
                .into_iter()
                .all(|key| row.fields.get(key).is_some_and(Value::is_boolean))
    }));
    let networks = connector
        .list_resource_items(RESOURCE_KIND_NETWORKS, None)
        .await
        .expect("the live site should list networks");
    assert!(networks.iter().all(|row| {
        row.fields.get("name").is_some_and(Value::is_string)
            && row.fields.get("vlanId").is_some_and(Value::is_number)
            && row.fields.get("management").is_some_and(Value::is_string)
            && row.fields.get("enabled").is_some_and(Value::is_boolean)
    }));
    let wlan_broadcasts = connector
        .list_resource_items(RESOURCE_KIND_WLAN_BROADCASTS, None)
        .await
        .expect("the live site should list WLAN broadcasts and their full configurations");
    assert!(wlan_broadcasts.iter().all(|row| {
        row.fields.get("name").is_some_and(Value::is_string)
            && row.fields.get("enabled").is_some_and(Value::is_boolean)
            && row.fields.get("hidden").is_some_and(Value::is_boolean)
            && row.fields.get("securityType").is_some_and(Value::is_string)
            && row.fields.get("frequencies").is_some_and(Value::is_string)
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
        "{test_name}: {:#}; {} client rows, {} voucher rows, {} WAN rows, {} pending-device rows, {} ACL rows, {} DNS rows, {} zone rows, {} policy rows, {} network rows, {} WLAN rows, {} port rows",
        status.details,
        clients.len(),
        vouchers.len(),
        wans.len(),
        pending_devices.len(),
        acl_rules.len(),
        dns_policies.len(),
        firewall_zones.len(),
        firewall_policies.len(),
        networks.len(),
        wlan_broadcasts.len(),
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
            json!({"resourceId": &client_id, "timeLimitMinutes": 15}),
        )
        .await
        .expect("the selected guest should accept authorization");
    assert!(result.success, "{}", result.message);
    eprintln!("{test_name}: {}", result.message);

    let result = connector
        .execute_action(
            ACTION_UNAUTHORIZE_GUEST,
            None,
            json!({"resourceId": client_id}),
        )
        .await
        .expect("the selected guest should accept unauthorization");
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
                "timeLimitMinutes": 15
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

#[tokio::test]
#[ignore = "creates and deletes a real DNS A record; run manually with explicit acknowledgement"]
async fn a_real_unifi_dns_record_can_be_created_and_deleted_with_explicit_opt_in() {
    let test_name = "a_real_unifi_dns_record_can_be_created_and_deleted_with_explicit_opt_in";
    assert_eq!(
        std::env::var("LOOM_TEST_UNIFI_NETWORK_ALLOW_DNS_WRITE").as_deref(),
        Ok("1"),
        "set LOOM_TEST_UNIFI_NETWORK_ALLOW_DNS_WRITE=1 only when a temporary local DNS record is acceptable"
    );
    let connector = live_connector(test_name)
        .await
        .expect("live UniFi Network variables are required for the manual DNS test");
    let before = connector
        .list_resource_items(RESOURCE_KIND_DNS_POLICIES, None)
        .await
        .expect("list DNS policies before creation");
    let before_ids = before
        .iter()
        .map(|record| record.id.as_str())
        .collect::<HashSet<_>>();
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is after the Unix epoch")
        .as_secs();
    let domain = format!("loom-test-{suffix}.invalid");

    connector
        .execute_action(
            ACTION_CREATE_A_RECORD,
            None,
            json!({"domain": domain, "ipv4Address": "192.0.2.10", "ttl": 60}),
        )
        .await
        .expect("create temporary DNS A record");
    let after = connector
        .list_resource_items(RESOURCE_KIND_DNS_POLICIES, None)
        .await
        .expect("list DNS policies after creation");
    let created = after
        .iter()
        .find(|record| !before_ids.contains(record.id.as_str()))
        .expect("new DNS record should appear");
    let created_id = created.id.clone();

    connector
        .execute_action(
            ACTION_DELETE_DNS_POLICY,
            None,
            json!({"resourceId": created_id}),
        )
        .await
        .expect("delete temporary DNS A record");
    let final_rows = connector
        .list_resource_items(RESOURCE_KIND_DNS_POLICIES, None)
        .await
        .expect("list DNS policies after deletion");
    assert!(final_rows.iter().all(|record| record.id != created_id));
}

#[tokio::test]
#[ignore = "toggles real firewall logging and restores it; run manually with an explicit policy"]
async fn a_real_firewall_policy_logging_can_be_toggled_and_restored_with_explicit_opt_in() {
    let test_name =
        "a_real_firewall_policy_logging_can_be_toggled_and_restored_with_explicit_opt_in";
    assert_eq!(
        std::env::var("LOOM_TEST_UNIFI_NETWORK_ALLOW_FIREWALL_LOGGING_TOGGLE").as_deref(),
        Ok("1"),
        "set LOOM_TEST_UNIFI_NETWORK_ALLOW_FIREWALL_LOGGING_TOGGLE=1 only when two real policy updates are acceptable"
    );
    let policy_id = std::env::var("LOOM_TEST_UNIFI_NETWORK_FIREWALL_POLICY")
        .expect("set LOOM_TEST_UNIFI_NETWORK_FIREWALL_POLICY to a disposable policy UUID");
    let connector = live_connector(test_name)
        .await
        .expect("live UniFi Network variables are required for the manual firewall test");
    for _ in 0..2 {
        connector
            .execute_action(
                ACTION_TOGGLE_FIREWALL_LOGGING,
                None,
                json!({"resourceId": &policy_id}),
            )
            .await
            .expect("toggle firewall logging");
    }
}

#[tokio::test]
#[ignore = "toggles a real WLAN and restores it; clients can disconnect, so run manually"]
async fn a_real_wlan_can_be_toggled_and_restored_with_explicit_opt_in() {
    let test_name = "a_real_wlan_can_be_toggled_and_restored_with_explicit_opt_in";
    assert_eq!(
        std::env::var("LOOM_TEST_UNIFI_NETWORK_ALLOW_WLAN_TOGGLE").as_deref(),
        Ok("1"),
        "set LOOM_TEST_UNIFI_NETWORK_ALLOW_WLAN_TOGGLE=1 only when disconnecting every client on the selected WLAN is safe"
    );
    let wlan_id = std::env::var("LOOM_TEST_UNIFI_NETWORK_WLAN")
        .expect("set LOOM_TEST_UNIFI_NETWORK_WLAN to a disposable WLAN UUID");
    let connector = live_connector(test_name)
        .await
        .expect("live UniFi Network variables are required for the manual WLAN test");
    for _ in 0..2 {
        connector
            .execute_action(
                ACTION_TOGGLE_WLAN_ENABLED,
                None,
                json!({"resourceId": &wlan_id}),
            )
            .await
            .expect("toggle WLAN enabled state");
    }
}

#[tokio::test]
#[ignore = "adopts a real pending device; run manually with an explicit MAC and acknowledgement"]
async fn a_real_pending_unifi_device_can_be_adopted_only_with_explicit_opt_in() {
    let test_name = "a_real_pending_unifi_device_can_be_adopted_only_with_explicit_opt_in";
    assert_eq!(
        std::env::var("LOOM_TEST_UNIFI_NETWORK_ALLOW_ADOPTION").as_deref(),
        Ok("1"),
        "set LOOM_TEST_UNIFI_NETWORK_ALLOW_ADOPTION=1 only when adopting the selected device is intended"
    );
    let mac_address = std::env::var("LOOM_TEST_UNIFI_NETWORK_PENDING_DEVICE_MAC")
        .expect("set LOOM_TEST_UNIFI_NETWORK_PENDING_DEVICE_MAC to a currently pending device");
    let connector = live_connector(test_name)
        .await
        .expect("live UniFi Network variables are required for the manual adoption test");

    let result = connector
        .execute_action(ACTION_ADOPT, None, json!({"resourceId": mac_address}))
        .await
        .expect("the selected pending device should accept adoption");
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
