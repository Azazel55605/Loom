//! Shared media source and playback-target contracts for Loom connectors.
//!
//! This crate contains only transport-neutral types and dyn-compatible traits.
//! It does not browse a real service, fetch artwork, host streams, render a
//! widget, or decide whether a caller may control a target. Connectors implement
//! one or both capability traits; Core exposes those implementations through
//! optional discovery methods, and Web/backend remains the authorization
//! boundary.
//!
//! `loom-media` deliberately does not depend on `loom-core`. Core's base
//! `Connector` trait must name [`MediaSourceCapable`] and
//! [`MediaTargetCapable`], so the reverse dependency would form a Cargo cycle.
//! The media error contract follows Core's structured, serializable error
//! conventions without importing Core itself. See ADR 0041.

#![warn(missing_docs)]

use async_trait::async_trait;
use chrono::Duration;
use serde::{Deserialize, Serialize};

/// The rendering and transport family of a playable media item.
///
/// Audio covers both finite tracks and live streams. A live audio stream is
/// represented by [`MediaItem::duration`] being `None`. Video carries an
/// explicit live/file distinction because a camera feed has no duration,
/// seeking, or meaningful queue position while a video file does. Keeping both
/// kinds in one model lets them share browsing, resolution, queue, and
/// now-playing plumbing while a future UI chooses an appropriate render
/// surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaKind {
    /// Audio content, either finite or live as indicated by its duration.
    Audio,
    /// Video content.
    Video {
        /// `true` for a live feed; `false` for file-like, seekable video.
        live: bool,
    },
}

/// A browsable item published by a media source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    /// Source-stable identifier passed back to
    /// [`MediaSourceCapable::resolve`].
    pub id: String,
    /// User-facing item title.
    pub title: String,
    /// Audio/video kind and, for video, whether it is live.
    pub kind: MediaKind,
    /// Optional artist, channel, or equivalent creator label.
    pub artist: Option<String>,
    /// Optional album, collection, or equivalent grouping label.
    pub album: Option<String>,
    /// Optional URL or source-specific token for artwork.
    ///
    /// This crate neither fetches nor embeds the referenced image. The
    /// eventual display surface resolves it.
    pub artwork_ref: Option<String>,
    /// Finite duration, or `None` for live audio and live video.
    pub duration: Option<Duration>,
}

/// A browsable item resolved into a stream a target can actually play.
///
/// Resolution is intentionally async and fallible. It may involve more than a
/// lookup: a future file-manager source could need to expose a file through its
/// own lightweight HTTP endpoint before it can produce `stream_url`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedPlayable {
    /// URL the playback target can request.
    pub stream_url: String,
    /// The resolved audio/video kind.
    pub kind: MediaKind,
    /// Source metadata for the resolved item.
    pub item: MediaItem,
}

/// Whether a playback target is actively playing, paused, or stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlaybackStatus {
    /// Media is currently advancing.
    Playing,
    /// Media is loaded but its position is not advancing.
    Paused,
    /// No media is actively playing.
    Stopped,
}

/// How the current queue repeats after an item completes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RepeatMode {
    /// Do not repeat.
    Off,
    /// Repeat the current item.
    One,
    /// Repeat the complete queue.
    All,
}

/// Current state reported by a media playback target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybackState {
    /// Current transport state.
    pub status: PlaybackStatus,
    /// Position within finite media, or `None` for anything live.
    pub position: Option<Duration>,
    /// Target volume on a zero-through-one-hundred scale.
    pub volume_percent: u8,
    /// Item currently loaded by the target, when one exists.
    pub current_item: Option<MediaItem>,
    /// Whether queue order is randomized.
    pub shuffle: bool,
    /// Current repeat behavior.
    pub repeat: RepeatMode,
}

/// Ordered media waiting on a playback target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Queue {
    /// Items in playback order.
    pub items: Vec<MediaItem>,
    /// Index of the active item, or `None` when the queue has no active item.
    pub current_index: Option<usize>,
}

/// Why a media source or target operation could not be completed.
///
/// Like Core's connector errors, this remains cloneable and serializable so a
/// backend can preserve a useful discriminant instead of flattening every
/// failure into one opaque string.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaError {
    /// The source or target could not be contacted.
    #[error("media service is unreachable")]
    Unreachable,
    /// The requested item or target no longer exists.
    #[error("media item or target was not found")]
    NotFound,
    /// A browsable item could not be turned into a playable stream.
    #[error("media resolution failed: {0}")]
    ResolutionFailed(String),
    /// The target failed to perform a playback operation.
    #[error("media playback failed: {0}")]
    PlaybackFailed(String),
    /// The requested operation is not meaningful or supported by this target.
    #[error("media operation is unsupported: {0}")]
    Unsupported(String),
}

/// Optional source-side media capability implemented by a connector.
///
/// A connector may implement this trait, [`MediaTargetCapable`], both, or
/// neither. Music Assistant is expected to implement both because it is a
/// source hub and a target orchestrator. A future file-manager-style connector
/// would implement only this source side, while a bespoke speaker would
/// implement only the target side.
#[async_trait]
pub trait MediaSourceCapable: Send + Sync {
    /// Lists media at the source root or within an optional source-defined path.
    async fn browse(&self, path: Option<&str>) -> Result<Vec<MediaItem>, MediaError>;

    /// Searches this source for media matching a user-entered query.
    async fn search(&self, query: &str) -> Result<Vec<MediaItem>, MediaError>;

    /// Resolves a source item into a URL a playback target can request.
    ///
    /// This may perform real work, including arranging temporary HTTP serving,
    /// which is why resolution is async and fallible rather than a field read.
    async fn resolve(&self, item_id: &str) -> Result<ResolvedPlayable, MediaError>;
}

/// Optional playback-target media capability implemented by a connector.
///
/// A connector may implement this trait, [`MediaSourceCapable`], both, or
/// neither. Music Assistant is expected to implement both; a bespoke speaker
/// may only be a target, and a file-manager-style connector may only be a
/// source. Operations that do not apply to live media—especially seeking and
/// queue mutation—must return [`MediaError::Unsupported`] rather than a generic
/// playback failure.
#[async_trait]
pub trait MediaTargetCapable: Send + Sync {
    /// Returns the target's current transport, item, volume, and repeat state.
    async fn playback_state(&self) -> Result<PlaybackState, MediaError>;

    /// Replaces or starts playback with a resolved item.
    async fn play(&self, playable: ResolvedPlayable) -> Result<(), MediaError>;

    /// Pauses the current item without discarding its position.
    async fn pause(&self) -> Result<(), MediaError>;

    /// Resumes a paused item.
    async fn resume(&self) -> Result<(), MediaError>;

    /// Stops playback.
    async fn stop(&self) -> Result<(), MediaError>;

    /// Moves finite media to `position`.
    ///
    /// Live targets return [`MediaError::Unsupported`].
    async fn seek(&self, position: Duration) -> Result<(), MediaError>;

    /// Sets target volume as a percentage.
    async fn set_volume(&self, percent: u8) -> Result<(), MediaError>;

    /// Enables or disables randomized queue order.
    async fn set_shuffle(&self, enabled: bool) -> Result<(), MediaError>;

    /// Sets how the target repeats its current item or queue.
    async fn set_repeat(&self, mode: RepeatMode) -> Result<(), MediaError>;

    /// Advances to the next queued item.
    async fn skip_next(&self) -> Result<(), MediaError>;

    /// Returns to the previous queued item.
    async fn skip_previous(&self) -> Result<(), MediaError>;

    /// Returns the target's current queue.
    ///
    /// A live-only target with no queue returns [`MediaError::Unsupported`].
    async fn queue(&self) -> Result<Queue, MediaError>;

    /// Adds a resolved item to the target's queue.
    ///
    /// A live-only target with no queue returns [`MediaError::Unsupported`].
    async fn queue_add(&self, playable: ResolvedPlayable) -> Result<(), MediaError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct EmptyMediaService;

    #[async_trait]
    impl MediaSourceCapable for EmptyMediaService {
        async fn browse(&self, _path: Option<&str>) -> Result<Vec<MediaItem>, MediaError> {
            Ok(Vec::new())
        }

        async fn search(&self, _query: &str) -> Result<Vec<MediaItem>, MediaError> {
            Ok(Vec::new())
        }

        async fn resolve(&self, item_id: &str) -> Result<ResolvedPlayable, MediaError> {
            Err(MediaError::ResolutionFailed(format!(
                "fixture has no item {item_id}"
            )))
        }
    }

    fn accepts_dyn_source(_source: &dyn MediaSourceCapable) {}

    #[tokio::test]
    async fn source_contract_is_dyn_compatible_and_errors_round_trip() {
        let source = EmptyMediaService;
        accepts_dyn_source(&source);
        assert!(source.browse(None).await.unwrap().is_empty());

        let error = source.resolve("missing").await.unwrap_err();
        let encoded = serde_json::to_value(&error).unwrap();
        assert_eq!(
            serde_json::from_value::<MediaError>(encoded).unwrap(),
            error
        );
    }

    #[test]
    fn live_and_file_video_remain_distinct_wire_values() {
        let live = serde_json::to_value(MediaKind::Video { live: true }).unwrap();
        let file = serde_json::to_value(MediaKind::Video { live: false }).unwrap();
        assert_ne!(live, file);
        assert_eq!(
            serde_json::from_value::<MediaKind>(live).unwrap(),
            MediaKind::Video { live: true }
        );
    }
}
