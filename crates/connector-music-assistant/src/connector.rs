use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
};

use async_trait::async_trait;
use chrono::Duration;
use loom_core::connector::{
    details::set_detail, ActionResult, ApplicableTarget, BrowsableItem, BrowsableItemKind,
    ColumnDescriptor, ColumnValueType, Connector, ConnectorAction, ConnectorError,
    ConnectorMetadata, ConnectorStatus, DataPointDescriptor, DataPointValueType, DisplayField,
    DisplayWidgetType, HealthState, NetworkTarget, ResourceItem, ResourceKindDescriptor, SubTarget,
    WidgetBinding, WidgetLayout,
};
use loom_media::{
    AudioInfo, MediaError, MediaItem, MediaKind, MediaSourceCapable, MediaTargetCapable,
    PlaybackState, PlaybackStatus, Queue, RepeatMode, ResolvedPlayable,
};
use serde_json::{json, Map, Value};

use crate::{
    config_schema, MusicAssistantClient, MusicAssistantConnectorConfig, MusicAssistantError,
};

pub const TYPE_ID: &str = "music-assistant";
pub const DISPLAY_NAME: &str = "Music Assistant";
pub const ICON: &str = "brand:music-assistant";
pub const RESOURCE_KIND_PLAYERS: &str = "players";
pub const DATA_POINT_PLAYER_COUNT: &str = "playerCount";
pub const DATA_POINT_MUSIC_ASSISTANT_VERSION: &str = "musicAssistantVersion";
pub const DATA_POINT_PLAYER_NAME: &str = "playerName";
pub const DATA_POINT_PLAYER_TYPE: &str = "playerType";
pub const DATA_POINT_IS_AVAILABLE: &str = "isAvailable";

const TARGET_PREFIX: &str = "player:";

/// One Music Assistant server, library source, and collection of player targets.
pub struct MusicAssistantConnector {
    client: MusicAssistantClient,
    host: String,
    port: u16,
    image_base: String,
    players: RwLock<Vec<Value>>,
    lyrics_cache: Arc<RwLock<HashMap<String, Option<String>>>>,
    media_targets: HashMap<String, MusicAssistantPlayerTarget>,
}

impl MusicAssistantConnector {
    /// Connects, authenticates, and verifies the command path by listing players.
    pub async fn from_config_value(value: Value) -> Result<Self, ConnectorError> {
        let config = MusicAssistantConnectorConfig::from_value(value)?;
        let client =
            MusicAssistantClient::connect(&config.host, config.port, config.token.as_deref())
                .await
                .map_err(connector_error)?;
        let players = list_players(&client).await.map_err(connector_error)?;
        let image_base = http_base(&config.host, config.port);
        let lyrics_cache = Arc::new(RwLock::new(HashMap::new()));
        let media_targets = players
            .iter()
            .filter_map(|player| player_id(player))
            .map(|id| {
                let target_id = target_id(id);
                (
                    target_id,
                    MusicAssistantPlayerTarget {
                        client: client.clone(),
                        player_id: id.to_owned(),
                        image_base: image_base.clone(),
                        lyrics_cache: Arc::clone(&lyrics_cache),
                        last_item_id: Arc::new(Mutex::new(None)),
                    },
                )
            })
            .collect();
        Ok(Self {
            client,
            host: config.host,
            port: config.port,
            image_base,
            players: RwLock::new(players),
            lyrics_cache,
            media_targets,
        })
    }

    fn player_snapshot(&self) -> Vec<Value> {
        self.players
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn remember_players(&self, players: Vec<Value>) {
        *self
            .players
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = players;
    }

    async fn current_players(&self) -> Result<Vec<Value>, ConnectorError> {
        let players = list_players(&self.client).await.map_err(connector_error)?;
        self.remember_players(players.clone());
        Ok(players)
    }
}

#[async_trait]
impl Connector for MusicAssistantConnector {
    fn as_media_source(&self, target_id: Option<&str>) -> Option<&dyn MediaSourceCapable> {
        target_id.is_none().then_some(self)
    }

    fn as_media_target(&self, target_id: Option<&str>) -> Option<&dyn MediaTargetCapable> {
        target_id.and_then(|target| self.media_targets.get(target).map(|value| value as _))
    }

    async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
        let players = self.current_players().await?;
        let mut details = Value::Object(Map::new());
        set_detail(
            &mut details,
            None,
            DATA_POINT_PLAYER_COUNT,
            json!(players.len()),
        );
        set_detail(
            &mut details,
            None,
            DATA_POINT_MUSIC_ASSISTANT_VERSION,
            json!(self.client.server_info().server_version),
        );
        let mut status = ConnectorStatus::new(HealthState::Healthy, details)
            .with_target_health(String::new(), HealthState::Healthy);
        for player in &players {
            let Some(id) = player_id(player) else {
                continue;
            };
            let target = target_id(id);
            let available = player_bool(player, "available").unwrap_or(false);
            set_detail(
                &mut status.details,
                Some(&target),
                DATA_POINT_PLAYER_NAME,
                json!(player_name(player)),
            );
            set_detail(
                &mut status.details,
                Some(&target),
                DATA_POINT_PLAYER_TYPE,
                json!(player_type(player)),
            );
            set_detail(
                &mut status.details,
                Some(&target),
                DATA_POINT_IS_AVAILABLE,
                json!(available),
            );
            status = status.with_target_health(
                target,
                if available {
                    HealthState::Healthy
                } else {
                    HealthState::Down
                },
            );
        }
        Ok(status)
    }

    async fn actions(&self) -> Vec<ConnectorAction> {
        Vec::new()
    }

    async fn execute_action(
        &self,
        action_id: &str,
        _target_id: Option<&str>,
        _params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        Err(ConnectorError::invalid_action(action_id))
    }

    fn supports_sub_targets(&self) -> bool {
        true
    }

    async fn list_sub_targets(&self) -> Result<Vec<SubTarget>, ConnectorError> {
        Ok(self
            .current_players()
            .await?
            .iter()
            .filter_map(player_sub_target)
            .collect())
    }

    fn supports_browsable_content(&self) -> bool {
        true
    }

    async fn browse_content(
        &self,
        target_id: Option<&str>,
        path: Option<&str>,
    ) -> Result<Vec<BrowsableItem>, ConnectorError> {
        if target_id.is_some() {
            return Ok(Vec::new());
        }
        self.browse_values(path)
            .await
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| browsable_item(value, &self.image_base))
                    .collect()
            })
            .map_err(media_to_connector_error)
    }

    async fn search_content(
        &self,
        target_id: Option<&str>,
        query: &str,
    ) -> Result<Vec<BrowsableItem>, ConnectorError> {
        if target_id.is_some() {
            return Ok(Vec::new());
        }
        self.search_values(query)
            .await
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| browsable_item(value, &self.image_base))
                    .collect()
            })
            .map_err(media_to_connector_error)
    }

    fn resource_kinds(&self, target_id: Option<&str>) -> Vec<ResourceKindDescriptor> {
        if target_id.is_some() {
            return Vec::new();
        }
        vec![ResourceKindDescriptor::new(
            RESOURCE_KIND_PLAYERS,
            "Players",
            vec![
                ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
                ColumnDescriptor::new("type", "Type", ColumnValueType::Text),
                ColumnDescriptor::new("available", "Available", ColumnValueType::Bool),
            ],
        )
        .applicable_to(ApplicableTarget::HostOnly)
        .with_rows_mapped_to_sub_targets()]
    }

    async fn list_resource_items(
        &self,
        kind: &str,
        target_id: Option<&str>,
    ) -> Result<Vec<ResourceItem>, ConnectorError> {
        if kind != RESOURCE_KIND_PLAYERS || target_id.is_some() {
            return Ok(Vec::new());
        }
        Ok(self
            .player_snapshot()
            .iter()
            .filter_map(player_resource_item)
            .collect())
    }

    fn config_schema(&self) -> Value {
        config_schema()
    }

    fn metadata(&self) -> ConnectorMetadata {
        ConnectorMetadata {
            id: TYPE_ID.to_owned(),
            name: DISPLAY_NAME.to_owned(),
            icon: Some(ICON.to_owned()),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            min_size: (3, 3),
        }
    }

    fn display_fields(&self) -> Vec<DisplayField> {
        vec![
            DisplayField::new("Host", &self.host),
            DisplayField::new("Port", self.port.to_string()),
            DisplayField::new("Server version", &self.client.server_info().server_version),
        ]
    }

    fn data_points(&self) -> Vec<DataPointDescriptor> {
        let mut points = vec![
            DataPointDescriptor::new(
                DATA_POINT_PLAYER_COUNT,
                "Players",
                DataPointValueType::Number,
            ),
            DataPointDescriptor::new(
                DATA_POINT_MUSIC_ASSISTANT_VERSION,
                "Music Assistant version",
                DataPointValueType::String,
            ),
        ];
        for target in self.media_targets.keys() {
            points.extend([
                DataPointDescriptor::new(
                    DATA_POINT_PLAYER_NAME,
                    "Player name",
                    DataPointValueType::String,
                )
                .for_target(target),
                DataPointDescriptor::new(
                    DATA_POINT_PLAYER_TYPE,
                    "Player type",
                    DataPointValueType::String,
                )
                .for_target(target),
                DataPointDescriptor::new(
                    DATA_POINT_IS_AVAILABLE,
                    "Available",
                    DataPointValueType::Bool,
                )
                .for_target(target),
            ]);
        }
        points
    }

    fn default_layout(&self) -> WidgetLayout {
        WidgetLayout::new(vec![
            WidgetBinding::display(DATA_POINT_PLAYER_COUNT, DisplayWidgetType::StatTile),
            WidgetBinding::display(
                DATA_POINT_MUSIC_ASSISTANT_VERSION,
                DisplayWidgetType::StatTile,
            ),
        ])
    }

    fn default_layout_for(&self, target_id: Option<&str>) -> WidgetLayout {
        match target_id {
            None => self.default_layout(),
            Some(target) if self.media_targets.contains_key(target) => WidgetLayout::new(vec![
                WidgetBinding::display(DATA_POINT_IS_AVAILABLE, DisplayWidgetType::StatusDot),
                WidgetBinding::media_player(true),
            ]),
            Some(_) => WidgetLayout::default(),
        }
    }

    fn network_target(&self) -> Option<NetworkTarget> {
        Some(NetworkTarget::new(network_host(&self.host), self.port))
    }
}

#[async_trait]
impl MediaSourceCapable for MusicAssistantConnector {
    fn supports_lyrics_lookup(&self) -> bool {
        true
    }

    async fn browse(&self, path: Option<&str>) -> Result<Vec<MediaItem>, MediaError> {
        self.browse_values(path).await.map(|values| {
            values
                .iter()
                .filter_map(|value| media_item(value, &self.image_base))
                .collect()
        })
    }

    async fn search(&self, query: &str) -> Result<Vec<MediaItem>, MediaError> {
        self.search_values(query).await.map(|values| {
            values
                .iter()
                .filter_map(|value| media_item(value, &self.image_base))
                .collect()
        })
    }

    /// Resolves metadata through MA and carries MA's canonical URI in
    /// `stream_url`. Music Assistant's queue command consumes that internal URI
    /// rather than a client-fetchable raw stream URL; this is the intentional
    /// source-specific-token interpretation required by the MA target adapter.
    async fn resolve(&self, item_id: &str) -> Result<ResolvedPlayable, MediaError> {
        let value = self
            .client
            .call("music/item_by_uri", json!({ "uri": item_id }))
            .await
            .map_err(media_error)?;
        let item = media_item(&value, &self.image_base).ok_or(MediaError::NotFound)?;
        Ok(ResolvedPlayable {
            stream_url: item.id.clone(),
            kind: item.kind.clone(),
            item,
        })
    }

    async fn fetch_lyrics(&self, item_id: &str) -> Result<Option<String>, MediaError> {
        let track = self
            .client
            .call(
                "music/item_by_uri",
                json!({ "uri": item_id, "allow_update_metadata": false }),
            )
            .await
            .map_err(media_error)?;
        let result = self
            .client
            .call("metadata/get_track_lyrics", json!({ "track": track }))
            .await
            .map_err(media_error)?;
        let lyrics = result
            .as_array()
            .and_then(|values| values.first())
            .and_then(Value::as_str)
            .map(str::to_owned);
        self.lyrics_cache
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(item_id.to_owned(), lyrics.clone());
        Ok(lyrics)
    }
}

impl MusicAssistantConnector {
    async fn browse_values(&self, path: Option<&str>) -> Result<Vec<Value>, MediaError> {
        array_result(
            self.client
                .call("music/browse", json!({ "path": path }))
                .await
                .map_err(media_error)?,
            "music/browse",
        )
    }

    async fn search_values(&self, query: &str) -> Result<Vec<Value>, MediaError> {
        let result = self
            .client
            .call(
                "music/search",
                json!({
                    "search_query": query,
                    "media_types": ["artist", "album", "track", "playlist", "radio", "audiobook", "podcast", "sound_effect"],
                    "limit": 50,
                    "library_only": false
                }),
            )
            .await
            .map_err(media_error)?;
        Ok(flatten_search_results(&result))
    }
}

#[derive(Clone)]
struct MusicAssistantPlayerTarget {
    client: MusicAssistantClient,
    player_id: String,
    image_base: String,
    lyrics_cache: Arc<RwLock<HashMap<String, Option<String>>>>,
    last_item_id: Arc<Mutex<Option<String>>>,
}

impl MusicAssistantPlayerTarget {
    async fn active_queue(&self) -> Result<Option<Value>, MediaError> {
        self.client
            .call(
                "player_queues/get_active_queue",
                json!({ "player_id": self.player_id }),
            )
            .await
            .map_err(media_error)
            .map(|value| (!value.is_null()).then_some(value))
    }

    async fn queue_id(&self) -> Result<String, MediaError> {
        let queue = self.active_queue().await?.ok_or_else(|| {
            MediaError::PlaybackFailed("the player has no active Music Assistant queue".to_owned())
        })?;
        string_field(&queue, "queue_id")
            .map(str::to_owned)
            .ok_or_else(|| MediaError::PlaybackFailed("active queue has no queue_id".to_owned()))
    }

    async fn queue_command(&self, command: &str, extra: Value) -> Result<(), MediaError> {
        let queue_id = self.queue_id().await?;
        let mut params = extra.as_object().cloned().unwrap_or_default();
        params.insert("queue_id".to_owned(), json!(queue_id));
        self.client
            .call(command, Value::Object(params))
            .await
            .map_err(media_error)?;
        Ok(())
    }

    async fn enrich_current_lyrics(&self, state: &mut PlaybackState) {
        let Some(item) = state.current_item.as_mut() else {
            *self
                .last_item_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
            return;
        };
        let item_id = item.id.clone();
        let changed = {
            let mut last = self
                .last_item_id
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if last.as_deref() == Some(item_id.as_str()) {
                false
            } else {
                *last = Some(item_id.clone());
                true
            }
        };

        if let Some(lyrics) = item.lyrics.clone() {
            self.lyrics_cache
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(item_id.clone(), Some(lyrics));
        } else if changed
            && !self
                .lyrics_cache
                .read()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .contains_key(&item_id)
        {
            // Track detail is the one additional read in the regular polling
            // path, and only happens when the current item changes. A missing
            // lyric is cached as such; provider lookups remain manual.
            let lyrics = self
                .client
                .call(
                    "music/item_by_uri",
                    json!({ "uri": item_id, "allow_update_metadata": false }),
                )
                .await
                .ok()
                .and_then(|value| lyrics(&value));
            self.lyrics_cache
                .write()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .insert(item_id.clone(), lyrics);
        }

        item.lyrics = self
            .lyrics_cache
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(&item_id)
            .cloned()
            .flatten();
    }
}

#[async_trait]
impl MediaTargetCapable for MusicAssistantPlayerTarget {
    fn supports_grouping(&self) -> bool {
        true
    }

    async fn join_group(&self, member_target_ids: &[String]) -> Result<(), MediaError> {
        let players = list_players(&self.client).await.map_err(media_error)?;
        let member_player_ids =
            grouping_member_player_ids(&players, &self.player_id, member_target_ids)?;
        self.client
            .call(
                "players/cmd/set_members",
                json!({
                    "target_player": self.player_id,
                    "player_ids_to_add": member_player_ids,
                    "player_ids_to_remove": null
                }),
            )
            .await
            .map_err(grouping_error)?;
        Ok(())
    }

    async fn leave_group(&self) -> Result<(), MediaError> {
        self.client
            .call(
                "players/cmd/ungroup",
                json!({ "player_id": self.player_id }),
            )
            .await
            .map_err(grouping_error)?;
        Ok(())
    }

    async fn playback_state(&self) -> Result<PlaybackState, MediaError> {
        let players = list_players(&self.client).await.map_err(media_error)?;
        let player = players
            .iter()
            .find(|player| player_id(player) == Some(self.player_id.as_str()))
            .ok_or(MediaError::NotFound)?;
        let queue = self.active_queue().await?;
        let mut state = playback_state(player, queue.as_ref(), &self.image_base);
        self.enrich_current_lyrics(&mut state).await;
        Ok(state)
    }

    async fn play(&self, playable: ResolvedPlayable) -> Result<(), MediaError> {
        self.queue_command(
            "player_queues/play_media",
            json!({ "media": playable.stream_url, "option": "replace", "radio_mode": false }),
        )
        .await
    }

    async fn pause(&self) -> Result<(), MediaError> {
        self.queue_command("player_queues/pause", json!({})).await
    }

    async fn resume(&self) -> Result<(), MediaError> {
        self.queue_command("player_queues/resume", json!({ "fade_in": null }))
            .await
    }

    async fn stop(&self) -> Result<(), MediaError> {
        self.queue_command("player_queues/stop", json!({})).await
    }

    async fn seek(&self, position: Duration) -> Result<(), MediaError> {
        if position < Duration::zero() {
            return Err(MediaError::Unsupported(
                "a seek position cannot be negative".to_owned(),
            ));
        }
        self.queue_command(
            "player_queues/seek",
            json!({ "position": position.num_seconds() }),
        )
        .await
    }

    async fn set_volume(&self, percent: u8) -> Result<(), MediaError> {
        self.client
            .call(
                "players/cmd/volume_set",
                json!({ "player_id": self.player_id, "volume_level": percent }),
            )
            .await
            .map_err(media_error)?;
        Ok(())
    }

    async fn set_shuffle(&self, enabled: bool) -> Result<(), MediaError> {
        self.queue_command(
            "player_queues/shuffle",
            json!({ "shuffle_enabled": enabled }),
        )
        .await
    }

    async fn set_repeat(&self, mode: RepeatMode) -> Result<(), MediaError> {
        let repeat_mode = match mode {
            RepeatMode::Off => "off",
            RepeatMode::One => "one",
            RepeatMode::All => "all",
        };
        self.queue_command(
            "player_queues/repeat",
            json!({ "repeat_mode": repeat_mode }),
        )
        .await
    }

    async fn skip_next(&self) -> Result<(), MediaError> {
        self.queue_command("player_queues/next", json!({})).await
    }

    async fn skip_previous(&self) -> Result<(), MediaError> {
        self.queue_command("player_queues/previous", json!({}))
            .await
    }

    async fn queue(&self) -> Result<Queue, MediaError> {
        let state = self.active_queue().await?.ok_or_else(|| {
            MediaError::PlaybackFailed("the player has no active Music Assistant queue".to_owned())
        })?;
        let queue_id = string_field(&state, "queue_id")
            .ok_or_else(|| MediaError::PlaybackFailed("active queue has no queue_id".to_owned()))?;
        let items = self
            .client
            .call(
                "player_queues/items",
                json!({ "queue_id": queue_id, "limit": 500, "offset": 0 }),
            )
            .await
            .map_err(media_error)?;
        let items = array_result(items, "player_queues/items")?
            .iter()
            .filter_map(|item| queue_media_item(item, &self.image_base))
            .collect();
        Ok(Queue {
            items,
            current_index: usize_field(&state, "current_index"),
        })
    }

    async fn queue_add(&self, playable: ResolvedPlayable) -> Result<(), MediaError> {
        self.queue_command(
            "player_queues/play_media",
            json!({ "media": playable.stream_url, "option": "add", "radio_mode": false }),
        )
        .await
    }
}

async fn list_players(client: &MusicAssistantClient) -> Result<Vec<Value>, MusicAssistantError> {
    let value = client.call("players/all", json!({})).await?;
    value
        .as_array()
        .cloned()
        .ok_or_else(|| MusicAssistantError::CommandError {
            code: None,
            message: "players/all returned a non-array result".to_owned(),
        })
}

fn grouping_member_player_ids(
    players: &[Value],
    primary_player_id: &str,
    member_target_ids: &[String],
) -> Result<Vec<String>, MediaError> {
    let primary = players
        .iter()
        .find(|player| player_id(player) == Some(primary_player_id))
        .ok_or(MediaError::NotFound)?;
    if primary
        .get("supported_features")
        .and_then(Value::as_array)
        .is_some_and(|features| !features.iter().any(|feature| feature == "set_members"))
    {
        return Err(MediaError::Unsupported(format!(
            "player `{}` does not advertise Music Assistant's set_members feature",
            player_name(primary)
        )));
    }

    let can_group_with = primary
        .get("can_group_with")
        .and_then(Value::as_array)
        .map(|values| values.iter().filter_map(Value::as_str).collect::<Vec<_>>());
    let mut members = Vec::with_capacity(member_target_ids.len());
    for target_id in member_target_ids {
        let member_id = target_id
            .strip_prefix(TARGET_PREFIX)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                MediaError::Unsupported(format!(
                    "`{target_id}` is not a Music Assistant player target"
                ))
            })?;
        if member_id == primary_player_id {
            return Err(MediaError::Unsupported(
                "the primary player cannot also be one of its group members".to_owned(),
            ));
        }
        let member = players
            .iter()
            .find(|player| player_id(player) == Some(member_id))
            .ok_or(MediaError::NotFound)?;
        if can_group_with
            .as_ref()
            .is_some_and(|compatible| !compatible.contains(&member_id))
        {
            return Err(MediaError::Unsupported(format!(
                "player `{}` cannot be grouped with `{}`",
                player_name(primary),
                player_name(member)
            )));
        }
        members.push(member_id.to_owned());
    }
    Ok(members)
}

fn connector_error(error: MusicAssistantError) -> ConnectorError {
    match error {
        MusicAssistantError::AuthFailed(reason) => ConnectorError::AuthFailed { reason },
        MusicAssistantError::ConnectionFailed(reason) => ConnectorError::unreachable(reason),
        MusicAssistantError::Disconnected | MusicAssistantError::Timeout => {
            ConnectorError::unreachable(error.to_string())
        }
        MusicAssistantError::CommandError { .. } => ConnectorError::Internal(error.to_string()),
    }
}

fn media_error(error: MusicAssistantError) -> MediaError {
    match error {
        MusicAssistantError::ConnectionFailed(_)
        | MusicAssistantError::Disconnected
        | MusicAssistantError::Timeout => MediaError::Unreachable,
        other => MediaError::PlaybackFailed(other.to_string()),
    }
}

fn grouping_error(error: MusicAssistantError) -> MediaError {
    match error {
        // MA assigns 9 to UnsupportedFeaturedException and 11 to
        // PlayerCommandFailed. Both are normal compatibility outcomes for
        // set_members/ungroup, so the backend may deliberately fan out.
        MusicAssistantError::CommandError {
            code: Some(9 | 11),
            message,
        } => MediaError::Unsupported(message),
        other => media_error(other),
    }
}

fn media_to_connector_error(error: MediaError) -> ConnectorError {
    match error {
        MediaError::Unreachable => ConnectorError::unreachable("Music Assistant is unreachable"),
        other => ConnectorError::Internal(other.to_string()),
    }
}

fn array_result(value: Value, command: &str) -> Result<Vec<Value>, MediaError> {
    value
        .as_array()
        .cloned()
        .ok_or_else(|| MediaError::PlaybackFailed(format!("{command} returned a non-array result")))
}

fn player_id(player: &Value) -> Option<&str> {
    string_field(player, "player_id")
}

fn player_name(player: &Value) -> &str {
    string_field(player, "name").unwrap_or("Unnamed player")
}

fn player_type(player: &Value) -> String {
    let provider = string_field(player, "provider").unwrap_or("unknown");
    let kind = string_field(player, "type").unwrap_or("player");
    format!("{provider} / {kind}")
}

fn player_bool(player: &Value, field: &str) -> Option<bool> {
    player.get(field).and_then(Value::as_bool)
}

fn target_id(player_id: &str) -> String {
    format!("{TARGET_PREFIX}{player_id}")
}

fn player_sub_target(player: &Value) -> Option<SubTarget> {
    Some(
        SubTarget::new(target_id(player_id(player)?), player_name(player))
            .of_kind("player")
            .with_icon("lucide:speaker"),
    )
}

fn player_resource_item(player: &Value) -> Option<ResourceItem> {
    Some(
        ResourceItem::new(target_id(player_id(player)?))
            .with_field("name", player_name(player))
            .with_field("type", player_type(player))
            .with_field(
                "available",
                player_bool(player, "available").unwrap_or(false),
            ),
    )
}

fn playback_state(player: &Value, queue: Option<&Value>, image_base: &str) -> PlaybackState {
    let state_source = queue.unwrap_or(player);
    let status = match string_field(state_source, "state")
        .or_else(|| string_field(state_source, "playback_state"))
    {
        Some("playing") => PlaybackStatus::Playing,
        Some("paused") => PlaybackStatus::Paused,
        _ => PlaybackStatus::Stopped,
    };
    let current = queue
        .and_then(|queue| queue.get("current_item"))
        .and_then(|item| queue_media_item(item, image_base))
        .or_else(|| {
            player
                .get("current_media")
                .and_then(|item| player_media_item(item, image_base))
        });
    let position = current.as_ref().and_then(|item| {
        item.duration.and_then(|_| {
            queue
                .and_then(|queue| seconds_duration(queue.get("elapsed_time")))
                .or_else(|| seconds_duration(player.get("elapsed_time")))
        })
    });
    PlaybackState {
        status,
        position,
        volume_percent: player
            .get("volume_level")
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .unwrap_or(0),
        current_item: current,
        audio_info: queue
            .and_then(|queue| queue.get("current_item"))
            .and_then(audio_info),
        shuffle: queue
            .and_then(|queue| queue.get("shuffle_enabled"))
            .and_then(Value::as_bool)
            .unwrap_or(false),
        repeat: match queue.and_then(|queue| string_field(queue, "repeat_mode")) {
            Some("one") => RepeatMode::One,
            Some("all") => RepeatMode::All,
            _ => RepeatMode::Off,
        },
    }
}

fn player_media_item(value: &Value, image_base: &str) -> Option<MediaItem> {
    let id = string_field(value, "uri")?;
    let duration = seconds_duration(value.get("duration"));
    Some(MediaItem {
        id: id.to_owned(),
        title: string_field(value, "title")
            .unwrap_or("Unknown item")
            .to_owned(),
        kind: MediaKind::Audio,
        artist: string_field(value, "artist").map(str::to_owned),
        album: string_field(value, "album").map(str::to_owned),
        artwork_ref: string_field(value, "image_url")
            .and_then(|reference| browser_player_artwork_ref(reference, image_base)),
        lyrics: lyrics(value),
        duration,
    })
}

fn queue_media_item(value: &Value, image_base: &str) -> Option<MediaItem> {
    value
        .get("media_item")
        .filter(|value| !value.is_null())
        .and_then(|item| media_item(item, image_base))
        .or_else(|| {
            let id = string_field(value, "queue_item_id")?;
            Some(MediaItem {
                id: id.to_owned(),
                title: string_field(value, "name")
                    .unwrap_or("Unknown item")
                    .to_owned(),
                kind: MediaKind::Audio,
                artist: None,
                album: None,
                artwork_ref: artwork_ref(value, image_base),
                lyrics: lyrics(value),
                duration: seconds_duration(value.get("duration")),
            })
        })
}

fn media_item(value: &Value, image_base: &str) -> Option<MediaItem> {
    let id = string_field(value, "uri").or_else(|| string_field(value, "item_id"))?;
    let media_type = string_field(value, "media_type").unwrap_or("unknown");
    let duration = if media_type == "radio" {
        None
    } else {
        seconds_duration(value.get("duration"))
    };
    Some(MediaItem {
        id: id.to_owned(),
        title: string_field(value, "name")
            .unwrap_or("Unknown item")
            .to_owned(),
        kind: MediaKind::Audio,
        artist: value
            .get("artists")
            .and_then(Value::as_array)
            .and_then(|artists| artists.first())
            .and_then(|artist| string_field(artist, "name"))
            .map(str::to_owned),
        album: value
            .get("album")
            .and_then(|album| string_field(album, "name"))
            .map(str::to_owned),
        artwork_ref: artwork_ref(value, image_base),
        lyrics: lyrics(value),
        duration,
    })
}

fn lyrics(value: &Value) -> Option<String> {
    value
        .get("metadata")
        .and_then(|metadata| string_field(metadata, "lyrics"))
        .filter(|lyrics| !lyrics.trim().is_empty())
        .map(str::to_owned)
}

fn audio_info(queue_item: &Value) -> Option<AudioInfo> {
    let format = queue_item.get("streamdetails")?.get("audio_format")?;
    let positive_u32 = |field: &str| {
        format
            .get(field)
            .and_then(Value::as_u64)
            .and_then(|value| u32::try_from(value).ok())
            .filter(|value| *value > 0)
    };
    let positive_u8 = |field: &str| {
        format
            .get(field)
            .and_then(Value::as_u64)
            .and_then(|value| u8::try_from(value).ok())
            .filter(|value| *value > 0)
    };
    let codec = string_field(format, "codec_type")
        .filter(|value| *value != "unknown")
        .or_else(|| string_field(format, "content_type").filter(|value| *value != "unknown"))
        .map(str::to_owned);
    Some(AudioInfo {
        codec,
        sample_rate_hz: positive_u32("sample_rate"),
        bit_depth: positive_u8("bit_depth"),
        channels: positive_u8("channels"),
        bitrate_kbps: positive_u32("bit_rate"),
    })
}

fn browsable_item(value: &Value, image_base: &str) -> Option<BrowsableItem> {
    let id = string_field(value, "path")
        .or_else(|| string_field(value, "uri"))
        .or_else(|| string_field(value, "item_id"))?;
    let media_type = string_field(value, "media_type").unwrap_or("unknown");
    let playable = value
        .get("is_playable")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let kind = if media_type == "folder" || !playable {
        BrowsableItemKind::Container
    } else {
        BrowsableItemKind::Leaf
    };
    let mut item = BrowsableItem::new(
        id,
        string_field(value, "name").unwrap_or("Unknown item"),
        kind,
    )
    .with_metadata("mediaType", media_type);
    if let Some(thumbnail) = artwork_ref(value, image_base) {
        item = item.with_thumbnail(thumbnail);
    }
    Some(item)
}

fn artwork_ref(value: &Value, image_base: &str) -> Option<String> {
    let image = value
        .get("image")
        .filter(|image| !image.is_null())
        .or_else(|| value.get("metadata")?.get("images")?.as_array()?.first())?;
    if let Some(proxy_id) = string_field(image, "proxy_id") {
        return Some(format!("{image_base}/imageproxy/{proxy_id}"));
    }
    let path = string_field(image, "path")?;
    (path.starts_with("http://") || path.starts_with("https://") || path.starts_with("data:"))
        .then(|| path.to_owned())
}

/// Turns MA's player-facing artwork URL into one the Loom client can fetch.
///
/// Current MA builds deliberately publish queue artwork through the stream
/// server because the physical player must be able to reach it. That absolute
/// URL can name an internal address or the separate stream-server port, neither
/// of which is necessarily reachable from a browser using Loom remotely. The
/// same canonical `/imageproxy/<proxy_id>` route is available on MA's configured
/// web/API origin, so only that MA-owned path is rebased. Provider-hosted image
/// URLs stay untouched.
fn browser_player_artwork_ref(reference: &str, image_base: &str) -> Option<String> {
    if reference.starts_with("data:") {
        return Some(reference.to_owned());
    }

    let path_and_query = if reference.starts_with('/') {
        return Some(format!("{}{}", image_base.trim_end_matches('/'), reference));
    } else if let Some(path) = reference.strip_prefix("imageproxy/") {
        return Some(format!(
            "{}/imageproxy/{path}",
            image_base.trim_end_matches('/')
        ));
    } else if reference.starts_with("http://") || reference.starts_with("https://") {
        reference
            .split_once("://")
            .and_then(|(_, authority_and_path)| {
                authority_and_path
                    .find('/')
                    .map(|at| &authority_and_path[at..])
            })
    } else {
        None
    };
    if let Some(path) = path_and_query.filter(|path| path.starts_with("/imageproxy/")) {
        return Some(format!("{}{path}", image_base.trim_end_matches('/')));
    }

    (reference.starts_with("http://") || reference.starts_with("https://"))
        .then(|| reference.to_owned())
}

fn flatten_search_results(value: &Value) -> Vec<Value> {
    let Some(object) = value.as_object() else {
        return Vec::new();
    };
    [
        "artists",
        "albums",
        "genres",
        "tracks",
        "playlists",
        "radio",
        "audiobooks",
        "podcasts",
        "sound_effects",
    ]
    .iter()
    .flat_map(|key| {
        object
            .get(*key)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .cloned()
    })
    .collect()
}

fn string_field<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field).and_then(Value::as_str)
}

fn usize_field(value: &Value, field: &str) -> Option<usize> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .and_then(|value| usize::try_from(value).ok())
}

fn seconds_duration(value: Option<&Value>) -> Option<Duration> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite() && *value >= 0.0)
        .map(|value| Duration::milliseconds((value * 1_000.0).round() as i64))
}

fn network_host(host: &str) -> String {
    host.strip_prefix("https://")
        .or_else(|| host.strip_prefix("http://"))
        .or_else(|| host.strip_prefix("wss://"))
        .or_else(|| host.strip_prefix("ws://"))
        .unwrap_or(host)
        .to_owned()
}

fn http_base(host: &str, port: u16) -> String {
    let host = host.trim_end_matches('/');
    if let Some(authority) = host
        .strip_prefix("https://")
        .or_else(|| host.strip_prefix("wss://"))
    {
        format!("https://{authority}:{port}")
    } else if let Some(authority) = host
        .strip_prefix("http://")
        .or_else(|| host.strip_prefix("ws://"))
    {
        format!("http://{authority}:{port}")
    } else {
        format!("http://{host}:{port}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_player() -> Value {
        json!({
            "player_id": "living-room",
            "provider": "sonos",
            "type": "player",
            "name": "Living room",
            "available": true,
            "volume_level": 42,
            "supported_features": ["set_members", "volume_set"],
            "can_group_with": ["kitchen"]
        })
    }

    #[test]
    fn player_shape_maps_to_target_and_resource_row() {
        let player = fixture_player();
        let target = player_sub_target(&player).unwrap();
        assert_eq!(target.id, "player:living-room");
        assert_eq!(target.label, "Living room");
        let row = player_resource_item(&player).unwrap();
        assert_eq!(row.id, target.id);
        assert_eq!(row.fields["type"], "sonos / player");
        assert_eq!(row.fields["available"], true);
    }

    #[test]
    fn grouping_preflight_uses_current_feature_and_compatibility_fields() {
        let players = vec![
            fixture_player(),
            json!({
                "player_id": "kitchen", "name": "Kitchen",
                "supported_features": [], "can_group_with": ["living-room"]
            }),
            json!({
                "player_id": "office", "name": "Office",
                "supported_features": [], "can_group_with": []
            }),
        ];
        assert_eq!(
            grouping_member_player_ids(&players, "living-room", &["player:kitchen".to_owned()])
                .unwrap(),
            ["kitchen"]
        );
        let error =
            grouping_member_player_ids(&players, "living-room", &["player:office".to_owned()])
                .unwrap_err();
        assert!(matches!(error, MediaError::Unsupported(message) if message.contains("Office")));
    }

    #[test]
    fn grouping_preflight_rejects_a_primary_without_set_members() {
        let players = vec![json!({
            "player_id": "limited", "name": "Limited player",
            "supported_features": ["volume_set"], "can_group_with": []
        })];
        let error = grouping_member_player_ids(&players, "limited", &[]).unwrap_err();
        assert!(
            matches!(error, MediaError::Unsupported(message) if message.contains("set_members"))
        );
    }

    #[test]
    fn grouping_command_compatibility_errors_remain_unsupported() {
        assert_eq!(
            grouping_error(MusicAssistantError::CommandError {
                code: Some(9),
                message: "player does not support group commands".to_owned(),
            }),
            MediaError::Unsupported("player does not support group commands".to_owned())
        );
    }

    #[test]
    fn browse_and_search_shapes_map_hierarchy_and_metadata() {
        let folder = json!({
            "item_id": "artists", "provider": "library", "name": "Artists",
            "uri": "library://folder/artists", "path": "library://artists",
            "media_type": "folder", "is_playable": false
        });
        let track = json!({
            "item_id": "track-1", "provider": "library", "name": "A Song",
            "uri": "library://track/track-1", "media_type": "track", "is_playable": true,
            "duration": 183, "artists": [{"name": "An Artist"}],
            "album": {"name": "An Album"},
            "image": {"proxy_id": "opaque-image-id"}
        });
        let image_base = "http://music.example.com:8095";
        assert_eq!(
            browsable_item(&folder, image_base).unwrap().kind,
            BrowsableItemKind::Container
        );
        let browsable = browsable_item(&track, image_base).unwrap();
        assert_eq!(browsable.kind, BrowsableItemKind::Leaf);
        assert_eq!(
            browsable.thumbnail.as_deref(),
            Some("http://music.example.com:8095/imageproxy/opaque-image-id")
        );
        let item = media_item(&track, image_base).unwrap();
        assert_eq!(item.id, "library://track/track-1");
        assert_eq!(item.artist.as_deref(), Some("An Artist"));
        assert_eq!(item.album.as_deref(), Some("An Album"));
        assert_eq!(item.duration, Some(Duration::seconds(183)));

        let flattened = flatten_search_results(&json!({
            "artists": [folder], "tracks": [track], "albums": []
        }));
        assert_eq!(flattened.len(), 2);
    }

    #[test]
    fn queue_shape_maps_complete_playback_state() {
        let state = playback_state(
            &json!({ "volume_level": 42 }),
            Some(&json!({
                "queue_id": "living-room", "state": "playing", "elapsed_time": 12.5,
                "shuffle_enabled": true, "repeat_mode": "all",
                "current_item": {
                    "queue_item_id": "queued-1", "name": "A Song", "duration": 180,
                    "streamdetails": {
                        "audio_format": {
                            "content_type": "flac", "codec_type": "flac",
                            "sample_rate": 96000, "bit_depth": 24,
                            "channels": 2, "bit_rate": 2304
                        }
                    },
                    "media_item": {
                        "item_id": "track-1", "provider": "library", "name": "A Song",
                        "uri": "library://track/track-1", "media_type": "track",
                        "duration": 180,
                        "metadata": { "lyrics": "Fixture lyric" }
                    }
                }
            })),
            "http://music.example.com:8095",
        );
        assert_eq!(state.status, PlaybackStatus::Playing);
        assert_eq!(state.position, Some(Duration::milliseconds(12_500)));
        assert_eq!(state.volume_percent, 42);
        assert!(state.shuffle);
        assert_eq!(state.repeat, RepeatMode::All);
        let item = state.current_item.unwrap();
        assert_eq!(item.title, "A Song");
        assert_eq!(item.lyrics.as_deref(), Some("Fixture lyric"));
        assert_eq!(
            state.audio_info,
            Some(AudioInfo {
                codec: Some("flac".to_owned()),
                sample_rate_hz: Some(96_000),
                bit_depth: Some(24),
                channels: Some(2),
                bitrate_kbps: Some(2_304),
            })
        );
    }

    #[test]
    fn player_stream_server_artwork_is_rebased_to_the_configured_web_origin() {
        // Current MA PlayerMedia wire shape, with documentation-only hosts:
        // current playback advertises the player-facing stream server rather
        // than the web/API origin the connector was configured to reach. This
        // mirrors the upstream queue controller's serialized response.
        let state = playback_state(
            &json!({
                "state": "playing",
                "volume_level": 34,
                "current_media": {
                    "uri": "library://track/fixture-track",
                    "media_type": "track",
                    "title": "Fixture track",
                    "image_url": "http://192.0.2.10:8097/imageproxy/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?size=512&fmt=jpeg",
                    "duration": 180
                }
            }),
            None,
            "https://music.example.com:443",
        );

        assert_eq!(
            state.current_item.unwrap().artwork_ref.as_deref(),
            Some("https://music.example.com:443/imageproxy/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa?size=512&fmt=jpeg")
        );
    }

    #[test]
    fn player_artwork_resolution_keeps_provider_urls_and_resolves_relative_proxy_paths() {
        assert_eq!(
            browser_player_artwork_ref(
                "/imageproxy/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb?size=512",
                "http://music.example.com:8095",
            )
            .as_deref(),
            Some("http://music.example.com:8095/imageproxy/bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb?size=512")
        );
        assert_eq!(
            browser_player_artwork_ref(
                "https://images.example.org/cover.jpg",
                "http://music.example.com:8095",
            )
            .as_deref(),
            Some("https://images.example.org/cover.jpg")
        );
    }

    #[test]
    fn live_radio_has_no_seek_position() {
        let state = playback_state(
            &json!({}),
            Some(&json!({
                "state": "playing", "elapsed_time": 90, "current_item": {
                    "queue_item_id": "radio", "name": "Live radio", "duration": null,
                    "media_item": {"item_id": "radio", "provider": "library",
                        "name": "Live radio", "uri": "library://radio/live",
                        "media_type": "radio", "duration": null}
                }
            })),
            "http://music.example.com:8095",
        );
        assert_eq!(state.position, None);
        assert_eq!(state.audio_info, None);
    }
}
