//! Opt-in checks against a real Pi-hole v6 instance.
//!
//! Set `LOOM_TEST_PIHOLE_BASE_URL` and `LOOM_TEST_PIHOLE_PASSWORD` to enable
//! the read-only test. For a deliberately untrusted HTTPS certificate, also
//! set `LOOM_TEST_PIHOLE_ALLOW_INSECURE_CERT=1`. The blocking test is ignored and additionally requires
//! `LOOM_TEST_PIHOLE_ALLOW_TOGGLE=1` because it changes DNS policy, even though
//! it restores the original state before making assertions.
//! Domain-list mutation is likewise ignored and requires
//! `LOOM_TEST_PIHOLE_ALLOW_DOMAIN_WRITES=1`; it uses a unique `.invalid` name,
//! verifies add/toggle/remove, and makes a best-effort cleanup before failing.

use loom_connector_pihole::{
    PiHoleConnector, ACTION_ADD_DOMAIN, ACTION_REMOVE_DOMAIN, ACTION_SET_BLOCKING,
    ACTION_TOGGLE_DOMAIN_ENABLED, DATA_POINT_BLOCKING_ENABLED, DATA_POINT_BLOCK_PERCENTAGE,
    DATA_POINT_DOMAINS_ON_BLOCKLIST, DATA_POINT_QUERIES_BLOCKED_TODAY, DATA_POINT_QUERIES_HISTORY,
    DATA_POINT_QUERIES_TODAY, DATA_POINT_UNIQUE_CLIENTS, RESOURCE_KIND_CLIENTS,
    RESOURCE_KIND_DOMAINS,
};
use loom_core::connector::{Connector, HealthState};
use serde_json::{json, Value};

#[tokio::test]
async fn a_real_pihole_reports_host_statistics_and_obeys_the_contract() {
    let test_name = "a_real_pihole_reports_host_statistics_and_obeys_the_contract";
    let Some(connector) = live_connector(test_name).await else {
        return;
    };

    let status = connector
        .status()
        .await
        .expect("the live Pi-hole status call should complete");
    assert_eq!(status.health, HealthState::Healthy, "{:#}", status.details);
    for id in [
        DATA_POINT_QUERIES_TODAY,
        DATA_POINT_QUERIES_BLOCKED_TODAY,
        DATA_POINT_BLOCK_PERCENTAGE,
        DATA_POINT_DOMAINS_ON_BLOCKLIST,
        DATA_POINT_UNIQUE_CLIENTS,
    ] {
        assert!(
            status
                .data_point_value(id)
                .and_then(Value::as_f64)
                .is_some_and(|value| value >= 0.0),
            "{id} should be a non-negative number: {:#}",
            status.details
        );
    }
    assert!(status
        .data_point_value(DATA_POINT_BLOCKING_ENABLED)
        .is_some_and(Value::is_boolean));
    assert!(status
        .data_point_value(DATA_POINT_QUERIES_HISTORY)
        .is_some_and(Value::is_array));

    loom_connector_test_kit::assert_connector_contract(&connector, &[None]).await;

    let domains = connector
        .list_resource_items(RESOURCE_KIND_DOMAINS, None)
        .await
        .expect("the live Pi-hole should list exact allow/deny domains");
    assert!(domains.iter().all(|row| {
        row.fields.get("domain").is_some_and(Value::is_string)
            && row.fields.get("type").is_some_and(Value::is_string)
            && row.fields.get("enabled").is_some_and(Value::is_boolean)
    }));
    let clients = connector
        .list_resource_items(RESOURCE_KIND_CLIENTS, None)
        .await
        .expect("the live Pi-hole should list top clients");
    assert!(clients.iter().all(|row| {
        row.fields.get("client").is_some_and(Value::is_string)
            && row.fields.get("queryCount").is_some_and(Value::is_number)
    }));
    eprintln!("{test_name}: {:#}", status.details);
}

#[tokio::test]
#[ignore = "changes real DNS blocking; run manually with LOOM_TEST_PIHOLE_ALLOW_TOGGLE=1"]
async fn a_real_pihole_can_toggle_blocking_and_restore_the_original_state() {
    let test_name = "a_real_pihole_can_toggle_blocking_and_restore_the_original_state";
    assert_eq!(
        std::env::var("LOOM_TEST_PIHOLE_ALLOW_TOGGLE").as_deref(),
        Ok("1"),
        "set LOOM_TEST_PIHOLE_ALLOW_TOGGLE=1 only when a temporary DNS-policy change is safe"
    );
    let connector = live_connector(test_name)
        .await
        .expect("live Pi-hole variables are required for the manual toggle test");
    let original = blocking_state(&connector).await;
    let changed_to = !original;

    let changed = connector
        .execute_action(ACTION_SET_BLOCKING, None, json!({ "enabled": changed_to }))
        .await;
    let observed_changed = blocking_state(&connector).await;
    let restored = connector
        .execute_action(ACTION_SET_BLOCKING, None, json!({ "enabled": original }))
        .await;
    let observed_restored = blocking_state(&connector).await;

    changed.expect("Pi-hole should accept the temporary blocking state");
    restored.expect("Pi-hole should restore the original blocking state");
    assert_eq!(observed_changed, changed_to);
    assert_eq!(observed_restored, original);
    eprintln!("{test_name}: changed blocking from {original} to {changed_to}, then restored it");
}

#[tokio::test]
#[ignore = "changes the real domain list; run manually with LOOM_TEST_PIHOLE_ALLOW_DOMAIN_WRITES=1"]
async fn a_real_pihole_can_add_toggle_and_remove_an_exact_domain() {
    let test_name = "a_real_pihole_can_add_toggle_and_remove_an_exact_domain";
    assert_eq!(
        std::env::var("LOOM_TEST_PIHOLE_ALLOW_DOMAIN_WRITES").as_deref(),
        Ok("1"),
        "set LOOM_TEST_PIHOLE_ALLOW_DOMAIN_WRITES=1 only when a temporary deny-list entry is safe"
    );
    let connector = live_connector(test_name)
        .await
        .expect("live Pi-hole variables are required for the manual domain test");
    let suffix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("current time is after the Unix epoch")
        .as_nanos();
    let domain = format!("loom-connector-test-{suffix}.invalid");

    let verification = verify_domain_lifecycle(&connector, &domain).await;
    // Cleanup is intentionally separate from the verification chain: if a
    // toggle assertion fails, the test still removes the row it created.
    if let Some(row) = find_domain(&connector, &domain).await {
        let _ = connector
            .execute_action(ACTION_REMOVE_DOMAIN, None, json!({ "resourceId": row.id }))
            .await;
    }
    if let Err(error) = verification {
        panic!("{error}");
    }
    assert!(find_domain(&connector, &domain).await.is_none());
}

async fn verify_domain_lifecycle(connector: &PiHoleConnector, domain: &str) -> Result<(), String> {
    let added = connector
        .execute_action(
            ACTION_ADD_DOMAIN,
            None,
            json!({
                "domain": domain,
                "listType": "deny",
                "comment": "Temporary Loom connector integration test"
            }),
        )
        .await
        .map_err(|error| error.to_string())?;
    if !added.success {
        return Err(added.message);
    }
    let row = find_domain(connector, domain)
        .await
        .ok_or_else(|| "the added domain did not appear in the domain resource list".to_owned())?;
    if row.fields.get("enabled") != Some(&json!(true)) {
        return Err("the new domain was not enabled".to_owned());
    }

    connector
        .execute_action(
            ACTION_TOGGLE_DOMAIN_ENABLED,
            None,
            json!({ "resourceId": row.id }),
        )
        .await
        .map_err(|error| error.to_string())?;
    let disabled = find_domain(connector, domain)
        .await
        .ok_or_else(|| "the disabled domain disappeared instead of being retained".to_owned())?;
    if disabled.fields.get("enabled") != Some(&json!(false)) {
        return Err("toggling the domain did not disable it".to_owned());
    }

    connector
        .execute_action(
            ACTION_TOGGLE_DOMAIN_ENABLED,
            None,
            json!({ "resourceId": disabled.id }),
        )
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn find_domain(
    connector: &PiHoleConnector,
    domain: &str,
) -> Option<loom_core::connector::ResourceItem> {
    connector
        .list_resource_items(RESOURCE_KIND_DOMAINS, None)
        .await
        .ok()?
        .into_iter()
        .find(|row| row.fields.get("domain") == Some(&json!(domain)))
}

async fn live_connector(test_name: &str) -> Option<PiHoleConnector> {
    let Ok(base_url) = std::env::var("LOOM_TEST_PIHOLE_BASE_URL") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_PIHOLE_BASE_URL is not set");
        return None;
    };
    let Ok(password) = std::env::var("LOOM_TEST_PIHOLE_PASSWORD") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_PIHOLE_PASSWORD is not set");
        return None;
    };
    let allow_insecure_cert =
        std::env::var("LOOM_TEST_PIHOLE_ALLOW_INSECURE_CERT").as_deref() == Ok("1");
    match PiHoleConnector::from_config_value(json!({
        "baseUrl": base_url,
        "password": password,
        "allowInsecureCert": allow_insecure_cert
    }))
    .await
    {
        Ok(connector) => Some(connector),
        Err(error) => {
            eprintln!("SKIPPING {test_name}: configured Pi-hole is unavailable: {error}");
            None
        }
    }
}

async fn blocking_state(connector: &PiHoleConnector) -> bool {
    connector
        .status()
        .await
        .expect("status call")
        .data_point_value(DATA_POINT_BLOCKING_ENABLED)
        .and_then(Value::as_bool)
        .expect("blockingEnabled status value")
}
