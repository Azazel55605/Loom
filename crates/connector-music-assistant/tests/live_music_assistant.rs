//! Opt-in connectivity proof against a real Music Assistant server.
//!
//! Set `LOOM_TEST_MUSIC_ASSISTANT_HOST` and, for current servers,
//! `LOOM_TEST_MUSIC_ASSISTANT_TOKEN` to enable it. Port defaults to 8095.

use chrono::Duration;
use loom_connector_music_assistant::{MusicAssistantClient, MusicAssistantConnector, DEFAULT_PORT};
use loom_core::connector::Connector;
use loom_core::media::PlaybackStatus;
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

#[tokio::test]
async fn a_real_music_assistant_exposes_library_players_and_obeys_the_contract() {
    let test_name = "a_real_music_assistant_exposes_library_players_and_obeys_the_contract";
    let Some(connector) = live_connector(test_name).await else {
        return;
    };
    let targets = connector
        .list_sub_targets()
        .await
        .expect("live player listing");
    let first = targets
        .first()
        .expect("the live server should expose a player");
    let source = connector.as_media_source(None).expect("host media source");
    let browsed = source.browse(None).await.expect("library root browse");
    let query =
        std::env::var("LOOM_TEST_MUSIC_ASSISTANT_SEARCH").unwrap_or_else(|_| "music".to_owned());
    let searched = source.search(&query).await.expect("library search");
    if let Some(item) = searched.first() {
        let resolved = source.resolve(&item.id).await.expect("item resolution");
        assert_eq!(resolved.stream_url, resolved.item.id);
    }
    let target = connector
        .as_media_target(Some(&first.id))
        .expect("player media target");
    let state = target.playback_state().await.expect("player state");
    let queue = target.queue().await.ok();
    loom_connector_test_kit::assert_connector_contract(&connector, &[None, Some(first.id.clone())])
        .await;
    eprintln!(
        "{test_name}: players={} browse_items={} search_items={} first_player={} state={:?} queue_items={}",
        targets.len(),
        browsed.len(),
        searched.len(),
        first.label,
        state.status,
        queue.map(|queue| queue.items.len()).unwrap_or(0)
    );
}

#[tokio::test]
#[ignore = "audibly controls a real player; requires explicit target, media URI, and acknowledgement"]
async fn a_real_player_can_run_the_full_transport_sequence() {
    assert_eq!(
        std::env::var("LOOM_TEST_MUSIC_ASSISTANT_ALLOW_PLAYBACK").as_deref(),
        Ok("yes"),
        "set LOOM_TEST_MUSIC_ASSISTANT_ALLOW_PLAYBACK=yes to acknowledge audible playback"
    );
    let connector = live_connector("a_real_player_can_run_the_full_transport_sequence")
        .await
        .expect("live Music Assistant configuration");
    let target_id = std::env::var("LOOM_TEST_MUSIC_ASSISTANT_PLAYER_TARGET")
        .expect("set player:<id> in LOOM_TEST_MUSIC_ASSISTANT_PLAYER_TARGET");
    let item_uri = std::env::var("LOOM_TEST_MUSIC_ASSISTANT_ITEM_URI")
        .expect("set an intentionally playable MA URI");
    let source = connector.as_media_source(None).unwrap();
    let target = connector
        .as_media_target(Some(&target_id))
        .expect("configured target must exist");
    let playable = source
        .resolve(&item_uri)
        .await
        .expect("resolve selected item");
    target.play(playable).await.expect("play");
    target.pause().await.expect("pause");
    target.resume().await.expect("resume");
    target.seek(Duration::seconds(5)).await.expect("seek");
    target.skip_next().await.expect("skip next");
    target.stop().await.expect("stop");
}

#[tokio::test]
#[ignore = "groups and audibly controls real players; requires two explicit targets, media URI, and acknowledgement"]
async fn two_real_compatible_players_group_and_follow_transport_together() {
    assert_eq!(
        std::env::var("LOOM_TEST_MUSIC_ASSISTANT_ALLOW_GROUPING").as_deref(),
        Ok("yes"),
        "set LOOM_TEST_MUSIC_ASSISTANT_ALLOW_GROUPING=yes to acknowledge real grouping and playback"
    );
    let connector =
        live_connector("two_real_compatible_players_group_and_follow_transport_together")
            .await
            .expect("live Music Assistant configuration");
    let primary_id = std::env::var("LOOM_TEST_MUSIC_ASSISTANT_GROUP_PRIMARY")
        .expect("set the primary player:<id>");
    let member_id = std::env::var("LOOM_TEST_MUSIC_ASSISTANT_GROUP_MEMBER")
        .expect("set the member player:<id>");
    let item_uri = std::env::var("LOOM_TEST_MUSIC_ASSISTANT_ITEM_URI")
        .expect("set an intentionally playable MA URI");
    let primary = connector
        .as_media_target(Some(&primary_id))
        .expect("primary target must exist");
    let member = connector
        .as_media_target(Some(&member_id))
        .expect("member target must exist");
    let playable = connector
        .as_media_source(None)
        .expect("host media source")
        .resolve(&item_uri)
        .await
        .expect("resolve selected item");

    let outcome = async {
        primary.join_group(std::slice::from_ref(&member_id)).await?;
        primary.play(playable).await?;
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        assert_eq!(
            primary.playback_state().await?.status,
            member.playback_state().await?.status
        );
        primary.pause().await?;
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        assert_eq!(
            primary.playback_state().await?.status,
            PlaybackStatus::Paused
        );
        assert_eq!(
            member.playback_state().await?.status,
            PlaybackStatus::Paused
        );
        Ok::<(), loom_core::media::MediaError>(())
    }
    .await;

    let _ = primary.stop().await;
    let leave_result = primary.leave_group().await;
    outcome.expect("native grouping and shared transport should succeed");
    leave_result.expect("the native group should be left during cleanup");
}

async fn live_connector(test_name: &str) -> Option<MusicAssistantConnector> {
    let Ok(host) = std::env::var("LOOM_TEST_MUSIC_ASSISTANT_HOST") else {
        eprintln!("SKIPPING {test_name}: LOOM_TEST_MUSIC_ASSISTANT_HOST is not set");
        return None;
    };
    let port = std::env::var("LOOM_TEST_MUSIC_ASSISTANT_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(DEFAULT_PORT);
    let token = std::env::var("LOOM_TEST_MUSIC_ASSISTANT_TOKEN").ok();
    let config = json!({ "host": host, "port": port, "token": token });
    match MusicAssistantConnector::from_config_value(config).await {
        Ok(connector) => Some(connector),
        Err(error) => {
            eprintln!("SKIPPING {test_name}: configured Music Assistant is unavailable: {error}");
            None
        }
    }
}
