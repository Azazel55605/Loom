//! Transient media grouping state owned by the backend.
//!
//! Native groups live in the media service, while fan-out groups exist only as
//! a Loom dispatch policy. Both are remembered here so leave requests and
//! subsequent controls can make the same distinction. This state intentionally
//! follows the backend process lifetime; it is session-like coordination, not
//! user-authored durable configuration.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Serialize;
use tokio::sync::RwLock;

/// How a recorded media group executes controls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MediaGroupMode {
    /// The connector's upstream service owns synchronization and dispatch.
    Native,
    /// Loom sends each write sequentially to the primary and every member.
    FanOut,
}

/// One primary target and the members grouped under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaGroup {
    pub mode: MediaGroupMode,
    pub member_target_ids: Vec<String>,
}

/// Per-instance, per-primary grouping records shared by all request handlers.
#[derive(Clone, Default)]
pub struct MediaGroupRegistry {
    groups: Arc<RwLock<HashMap<(String, String), MediaGroup>>>,
}

impl MediaGroupRegistry {
    pub async fn get(&self, instance_id: &str, target_id: &str) -> Option<MediaGroup> {
        self.groups
            .read()
            .await
            .get(&(instance_id.to_owned(), target_id.to_owned()))
            .cloned()
    }

    pub async fn record(
        &self,
        instance_id: &str,
        target_id: &str,
        mode: MediaGroupMode,
        member_target_ids: Vec<String>,
    ) {
        self.groups.write().await.insert(
            (instance_id.to_owned(), target_id.to_owned()),
            MediaGroup {
                mode,
                member_target_ids,
            },
        );
    }

    pub async fn clear(&self, instance_id: &str, target_id: &str) -> Option<MediaGroup> {
        self.groups
            .write()
            .await
            .remove(&(instance_id.to_owned(), target_id.to_owned()))
    }

    pub async fn forget_instance(&self, instance_id: &str) {
        self.groups
            .write()
            .await
            .retain(|(recorded_instance, _), _| recorded_instance != instance_id);
    }

    /// Primary-first target list for one write operation.
    pub async fn dispatch_targets(&self, instance_id: &str, target_id: &str) -> Vec<String> {
        let mut targets = vec![target_id.to_owned()];
        if let Some(group) = self.get(instance_id, target_id).await {
            if group.mode == MediaGroupMode::FanOut {
                targets.extend(group.member_target_ids);
            }
        }
        targets
    }
}
