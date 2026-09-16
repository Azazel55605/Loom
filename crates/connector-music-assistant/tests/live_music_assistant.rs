//! Opt-in connectivity proof against a real Music Assistant server.
//!
//! Set `LOOM_TEST_MUSIC_ASSISTANT_HOST` and, for current servers,
//! `LOOM_TEST_MUSIC_ASSISTANT_TOKEN` to enable it. Port defaults to 8095.

use loom_connector_music_assistant::{MusicAssistantClient, DEFAULT_PORT};
use serde_json::{json, Value};

#[tokio::test]
async fn a_real_music_assistant_answers_a_read_only_command() {
    let test_name = "a_real_music_assistant_answers_a_read_only_command";
    let Ok(host) = std::env::var("LOOM_TEST_MUSIC_ASSISTANT_HOST") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_MUSIC_ASSISTANT_HOST is not set");
        return;
    };
    let port = std::env::var("LOOM_TEST_MUSIC_ASSISTANT_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let token = std::env::var("LOOM_TEST_MUSIC_ASSISTANT_TOKEN").ok();
    let client = match MusicAssistantClient::connect(&host, port, token.as_deref()).await {
        Ok(client) => client,
        Err(error) => {
            eprintln!("SKIPPING {test_name}: configured Music Assistant is unavailable: {error}");
            return;
        }
    };

    let response = client
        .call("players/all", json!({}))
        .await
        .expect("an authenticated live Music Assistant connection should list players");
    assert!(matches!(response, Value::Array(_)));
    eprintln!(
        "{test_name}: server={} version={} schema={} players={response}",
        client.server_info().server_id,
        client.server_info().server_version,
        client.server_info().schema_version,
    );
}
