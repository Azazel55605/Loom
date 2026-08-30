use loom_connector_test_kit::assert_connector_contract;
use loom_core::connector::debug::{DebugConnector, FIXTURE_TARGETS};

#[tokio::test]
async fn debug_connector_obeys_the_public_connector_contract() {
    let connector = DebugConnector::default();
    let targets = vec![
        None,
        Some(FIXTURE_TARGETS[0].to_owned()),
        Some(FIXTURE_TARGETS[1].to_owned()),
    ];

    assert_connector_contract(&connector, &targets).await;
}
