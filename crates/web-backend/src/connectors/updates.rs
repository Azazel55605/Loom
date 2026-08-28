//! The update-check scheduler and its cache.
//!
//! Separate from the status poller on purpose, and the separation is not
//! organisational. The poller asks a *local* daemon how something is doing,
//! every few seconds, and backs off when it fails. This asks a *third party*
//! whether a newer image exists, every few hours, and is rate-limited by
//! someone else. Two cadences, two failure modes, two things that must not
//! share a schedule: a status poll must never wait behind a registry, and a
//! registry must never be asked at status-poll frequency.
//!
//! # What is generic here
//!
//! Nothing in this module knows what Docker is. It works for any connector that
//! reports [`Connector::supports_update_checking`] and whose stored
//! configuration carries the update settings named in [`UpdateSettings`] —
//! `checkForUpdates`, `checkIntervalMinutes`, `autoApplyUpdates`,
//! `autoApplyAtTime`, `excludeFromAutoUpdate`. Those keys are a **platform
//! convention**, documented in `docs/API_CONTRACT.md`: a connector that wants
//! scheduled checking publishes them in its own config schema, with its own
//! descriptions, and the scheduler reads them from the stored configuration.
//! The alternative — a Rust type in the backend naming Docker's fields — would
//! make the second connector to want this a backend change.
//!
//! # Automatic updates go through the front door
//!
//! An auto-applied update is invoked through
//! [`crate::routes::connectors::invoke_action`], the same function the HTTP
//! endpoint calls. It gets the same audit-log entry (attributed to the system
//! rather than to a person), the same pending-operation overlay, and the same
//! immediate re-poll. An automation with a quieter path of its own is an
//! automation whose actions are invisible exactly when someone is trying to
//! work out what happened overnight.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Local, Timelike, Utc};
use serde::Serialize;
use serde_json::{json, Value};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tokio::time::{self, MissedTickBehavior};
use uuid::Uuid;

use crate::state::AppState;

/// How often the scheduler wakes up to see whose turn it is.
///
/// A minute, which is the resolution a `HH:MM` maintenance window needs and no
/// finer. The *check* interval is per instance and measured in hours; this is
/// only how often the question "is anything due?" is asked.
pub const UPDATE_TICK: Duration = Duration::from_secs(60);

/// Pause between two registry checks within one tick.
///
/// A host with thirty containers would otherwise open thirty registry
/// connections in the same instant, from one address, which is both rude and
/// the fastest way to be rate-limited. Two seconds turns that burst into a
/// minute of trickle, invisible to a check that runs every six hours.
pub const CHECK_STAGGER: Duration = Duration::from_secs(2);

/// The action id an automatic update invokes.
///
/// A convention shared with the connector rather than a Docker import: the
/// backend depending on the Docker crate for a string would make every future
/// connector's update action a backend change. A connector that offers
/// scheduled updates offers an action by this name, taking `targetImageRef`.
pub const APPLY_UPDATE_ACTION: &str = "applyUpdate";

/// Parameter naming what to move the target to.
pub const TARGET_IMAGE_REF_PARAM: &str = "targetImageRef";

/// Update settings read from an instance's stored configuration.
///
/// Defaults are "do nothing": an instance whose configuration says none of this
/// is never checked and never updated. Reaching out to a third-party registry
/// is not something to start doing because somebody added a connector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSettings {
    /// Whether this instance is checked at all.
    pub check_for_updates: bool,
    /// Minutes between checks.
    pub check_interval_minutes: u64,
    /// Whether a found update may be applied unattended.
    pub auto_apply_updates: bool,
    /// Minutes since midnight, local time, to apply at — or `None` to apply as
    /// soon as an update is found.
    pub auto_apply_at_minute: Option<u32>,
    /// Never auto-apply, whatever `auto_apply_updates` says.
    pub exclude_from_auto_update: bool,
}

impl Default for UpdateSettings {
    fn default() -> Self {
        Self {
            check_for_updates: false,
            check_interval_minutes: 360,
            auto_apply_updates: false,
            auto_apply_at_minute: None,
            exclude_from_auto_update: false,
        }
    }
}

impl UpdateSettings {
    /// Reads the convention's keys out of a stored configuration.
    ///
    /// Tolerant by design: a missing or wrongly-typed key falls back to the
    /// default rather than failing. This runs against configurations written by
    /// connectors that may not have heard of update checking, and a scheduler
    /// that refused to start over an unexpected value would take checking down
    /// for every other instance too.
    pub fn from_config(config: &Value) -> Self {
        let defaults = Self::default();
        let flag = |key: &str, fallback: bool| {
            config.get(key).and_then(Value::as_bool).unwrap_or(fallback)
        };

        Self {
            check_for_updates: flag("checkForUpdates", defaults.check_for_updates),
            check_interval_minutes: config
                .get("checkIntervalMinutes")
                .and_then(Value::as_u64)
                .filter(|minutes| *minutes > 0)
                .unwrap_or(defaults.check_interval_minutes),
            auto_apply_updates: flag("autoApplyUpdates", defaults.auto_apply_updates),
            auto_apply_at_minute: config
                .get("autoApplyAtTime")
                .and_then(Value::as_str)
                .and_then(parse_hh_mm),
            exclude_from_auto_update: flag(
                "excludeFromAutoUpdate",
                defaults.exclude_from_auto_update,
            ),
        }
    }

    /// Whether a found update may be applied without anyone asking.
    pub fn auto_apply_enabled(&self) -> bool {
        self.auto_apply_updates && !self.exclude_from_auto_update
    }
}

/// Minutes since midnight for an `HH:MM` string, or `None` if it is not one.
fn parse_hh_mm(value: &str) -> Option<u32> {
    let (hour, minute) = value.trim().split_once(':')?;
    let hour: u32 = hour.parse().ok()?;
    let minute: u32 = minute.parse().ok()?;
    (hour < 24 && minute < 60).then_some(hour * 60 + minute)
}

/// What the last check found for one target.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateStatus {
    /// Whether something newer exists.
    pub available: bool,
    /// What the newer thing is called, in the managed system's own terms.
    pub latest_ref: Option<String>,
    /// When this was established — the reason a client can show the age of the
    /// answer rather than implying it is current.
    pub last_checked: DateTime<Utc>,
}

/// Per-instance scheduling state.
///
/// Not persisted. A restart therefore checks everything once, sooner than it
/// strictly had to — which is the harmless direction to be wrong in, and much
/// better than persisting a timestamp that could hold a check back for six
/// hours after an upgrade.
#[derive(Debug, Clone, Default)]
struct InstanceSchedule {
    last_checked: Option<DateTime<Utc>>,
    last_auto_applied: Option<DateTime<Utc>>,
}

/// The cache of update readings, shared with the HTTP layer.
///
/// Keyed by instance and then by target, with `""` for the instance itself —
/// the same convention `ConnectorStatus::details` uses, so a client indexes
/// both the same way.
#[derive(Clone, Default)]
pub struct UpdateCache {
    statuses: Arc<RwLock<HashMap<Uuid, HashMap<String, UpdateStatus>>>>,
    schedules: Arc<RwLock<HashMap<Uuid, InstanceSchedule>>>,
}

impl std::fmt::Debug for UpdateCache {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("UpdateCache")
            .finish_non_exhaustive()
    }
}

impl UpdateCache {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// Everything known about one instance, target-keyed.
    pub async fn statuses_for(&self, instance_id: &Uuid) -> Option<HashMap<String, UpdateStatus>> {
        self.statuses.read().await.get(instance_id).cloned()
    }

    /// Records one target's reading.
    pub async fn record(&self, instance_id: Uuid, target_id: Option<&str>, status: UpdateStatus) {
        self.statuses
            .write()
            .await
            .entry(instance_id)
            .or_default()
            .insert(target_id.unwrap_or_default().to_owned(), status);
    }

    /// Forgets an instance entirely, for when one is deleted.
    pub async fn forget(&self, instance_id: &Uuid) {
        self.statuses.write().await.remove(instance_id);
        self.schedules.write().await.remove(instance_id);
    }

    /// Every target of `instance_id` with an update waiting, oldest reading
    /// first so a sequential apply is stable between runs.
    pub async fn available_for(&self, instance_id: &Uuid) -> Vec<(String, UpdateStatus)> {
        let Some(statuses) = self.statuses_for(instance_id).await else {
            return Vec::new();
        };
        let mut rows: Vec<(String, UpdateStatus)> = statuses
            .into_iter()
            .filter(|(_, status)| status.available)
            .collect();
        rows.sort_by(|left, right| left.0.cmp(&right.0));
        rows
    }
}

/// One stored instance, as the scheduler needs it.
#[derive(Debug, sqlx::FromRow)]
struct ScheduledInstance {
    id: String,
    config: String,
}

/// Runs one scheduler tick.
///
/// Sequential across instances and across targets within an instance, with
/// [`CHECK_STAGGER`] between registry calls. Concurrency here would be a
/// mistake rather than an optimisation: the thing being conserved is somebody
/// else's rate limit, and there is no deadline — a check that finishes a minute
/// later than it could have is indistinguishable from one that did not.
///
/// `stagger` is a parameter so tests do not have to wait real seconds to prove
/// that the pause happens.
pub async fn run_tick(state: &AppState, now: DateTime<Utc>, stagger: Duration) {
    let instances = match sqlx::query_as::<_, ScheduledInstance>(
        "SELECT id, config FROM connector_instances ORDER BY created_at",
    )
    .fetch_all(&state.pool)
    .await
    {
        Ok(rows) => rows,
        Err(error) => {
            tracing::error!(%error, "the update scheduler could not read its instances");
            return;
        }
    };

    for instance in instances {
        let Ok(uuid) = Uuid::parse_str(&instance.id) else {
            continue;
        };
        let config: Value = serde_json::from_str(&instance.config).unwrap_or(Value::Null);
        let settings = UpdateSettings::from_config(&config);
        if !settings.check_for_updates {
            continue;
        }

        let Some(connector) = state.connectors.get(&uuid).await else {
            continue;
        };
        if !connector.supports_update_checking() {
            continue;
        }

        if is_due(state, uuid, now, settings.check_interval_minutes).await {
            check_instance(state, uuid, connector.as_ref(), stagger).await;
            state
                .updates
                .schedules
                .write()
                .await
                .entry(uuid)
                .or_default()
                .last_checked = Some(now);
        }

        if settings.auto_apply_enabled() {
            auto_apply(state, uuid, &settings, now).await;
        }
    }
}

/// Whether this instance's interval has elapsed.
async fn is_due(state: &AppState, id: Uuid, now: DateTime<Utc>, interval_minutes: u64) -> bool {
    let last = state
        .updates
        .schedules
        .read()
        .await
        .get(&id)
        .and_then(|schedule| schedule.last_checked);

    // Never checked means due now: an instance that has just been configured
    // should not wait six hours to say anything.
    match last {
        None => true,
        Some(last) => {
            now.signed_duration_since(last).num_seconds() >= (interval_minutes as i64) * 60
        }
    }
}

/// Checks every target of one instance, pausing between them.
async fn check_instance(
    state: &AppState,
    id: Uuid,
    connector: &dyn loom_core::connector::Connector,
    stagger: Duration,
) {
    // A connector with sub-targets is asked about each of them; one without is
    // asked about itself. Both shapes exist, and the scheduler should not need
    // to know which kind it is looking at.
    let targets: Vec<Option<String>> = if connector.supports_sub_targets() {
        match connector.list_sub_targets().await {
            Ok(targets) => targets.into_iter().map(|target| Some(target.id)).collect(),
            Err(error) => {
                tracing::warn!(instance = %id, %error, "could not enumerate targets for an update check");
                return;
            }
        }
    } else {
        vec![None]
    };

    for (index, target) in targets.iter().enumerate() {
        if index > 0 && !stagger.is_zero() {
            time::sleep(stagger).await;
        }

        match connector.check_for_updates(target.as_deref()).await {
            Ok(result) => {
                state
                    .updates
                    .record(
                        id,
                        target.as_deref(),
                        UpdateStatus {
                            available: result.available,
                            latest_ref: result.latest_ref,
                            // Stamped per check, not per tick. With a stagger
                            // between them, the last container of thirty is
                            // checked a minute after the first, and a shared
                            // timestamp would claim otherwise on the one screen
                            // whose whole job is showing how old the answer is.
                            last_checked: Utc::now(),
                        },
                    )
                    .await;
            }
            // Logged, not cached. A failed check must not overwrite a previous
            // answer with "no update available", which is what a client would
            // read a cleared entry as — the last real reading plus its age is
            // more honest than a fresh-looking absence.
            Err(error) => {
                tracing::warn!(
                    instance = %id,
                    target = target.as_deref().unwrap_or("<host>"),
                    %error,
                    "update check failed"
                );
            }
        }
    }
}

/// Applies whatever is waiting, if the window says now.
async fn auto_apply(state: &AppState, id: Uuid, settings: &UpdateSettings, now: DateTime<Utc>) {
    let waiting = state.updates.available_for(&id).await;
    if waiting.is_empty() {
        return;
    }

    if !window_is_open(state, id, settings, now).await {
        return;
    }

    let Some(connector) = state.connectors.get(&id).await else {
        return;
    };

    for (target, status) in waiting {
        let Some(latest_ref) = status.latest_ref.as_deref() else {
            // Nothing to move to. An update reported without a reference cannot
            // be applied, and inventing one from the current image would apply
            // the version already running.
            continue;
        };
        let target_id = (!target.is_empty()).then_some(target.as_str());

        // The same call the HTTP endpoint makes, so this lands in the audit log
        // and raises the pending overlay exactly like a person pressing the
        // button.
        let outcome = crate::routes::connectors::invoke_action(
            state,
            connector.clone(),
            &id.to_string(),
            APPLY_UPDATE_ACTION,
            target_id,
            json!({ TARGET_IMAGE_REF_PARAM: latest_ref }),
            crate::routes::connectors::ActionActor::System,
        )
        .await;

        match outcome {
            Ok(result) if result.success => {
                tracing::info!(
                    instance = %id,
                    target = target_id.unwrap_or("<host>"),
                    latest_ref,
                    "applied an update automatically"
                );
                // Cleared so the next tick does not try again before the next
                // check has confirmed the new state.
                state
                    .updates
                    .record(
                        id,
                        target_id,
                        UpdateStatus {
                            available: false,
                            latest_ref: Some(latest_ref.to_owned()),
                            last_checked: now,
                        },
                    )
                    .await;
            }
            Ok(result) => tracing::warn!(
                instance = %id,
                target = target_id.unwrap_or("<host>"),
                message = %result.message,
                "an automatic update was declined"
            ),
            Err(error) => tracing::warn!(
                instance = %id,
                target = target_id.unwrap_or("<host>"),
                "an automatic update failed: {error:?}"
            ),
        }
    }

    state
        .updates
        .schedules
        .write()
        .await
        .entry(id)
        .or_default()
        .last_auto_applied = Some(now);
}

/// Whether the maintenance window permits applying right now.
///
/// With no window configured, always: "apply when found" is what an empty
/// `autoApplyAtTime` means. With one, the tick that *crosses* the configured
/// local time opens it, and it stays shut for the rest of that day — otherwise
/// a container whose update keeps failing would be recreated every minute from
/// 03:00 until someone noticed.
async fn window_is_open(
    state: &AppState,
    id: Uuid,
    settings: &UpdateSettings,
    now: DateTime<Utc>,
) -> bool {
    let Some(target_minute) = settings.auto_apply_at_minute else {
        return true;
    };

    // Local time, because a maintenance window is a statement about the hours
    // nobody is using the service — which is a fact about where the server is,
    // not about UTC.
    let local = now.with_timezone(&Local);
    let current_minute = local.hour() * 60 + local.minute();
    if current_minute < target_minute {
        return false;
    }

    let last = state
        .updates
        .schedules
        .read()
        .await
        .get(&id)
        .and_then(|schedule| schedule.last_auto_applied);

    match last {
        None => true,
        Some(last) => last.with_timezone(&Local).date_naive() != local.date_naive(),
    }
}

/// Starts the scheduler loop.
///
/// Dropping the handle detaches the task, matching the status poller. The task
/// holds a cloned [`AppState`], every field of which is a handle.
pub fn spawn_scheduler(state: AppState) -> JoinHandle<()> {
    tokio::spawn(async move {
        let start = time::Instant::now() + UPDATE_TICK;
        let mut interval = time::interval_at(start, UPDATE_TICK);
        interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

        loop {
            interval.tick().await;
            run_tick(&state, Utc::now(), CHECK_STAGGER).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_to_doing_nothing_and_read_the_conventions_keys() {
        let quiet = UpdateSettings::from_config(&json!({}));
        assert!(!quiet.check_for_updates);
        assert!(!quiet.auto_apply_enabled());
        assert_eq!(quiet.auto_apply_at_minute, None);

        let configured = UpdateSettings::from_config(&json!({
            "checkForUpdates": true,
            "checkIntervalMinutes": 45,
            "autoApplyUpdates": true,
            "autoApplyAtTime": "03:30",
        }));
        assert!(configured.check_for_updates);
        assert_eq!(configured.check_interval_minutes, 45);
        assert!(configured.auto_apply_enabled());
        assert_eq!(configured.auto_apply_at_minute, Some(210));

        // The exclusion outranks the switch.
        assert!(!UpdateSettings::from_config(&json!({
            "autoApplyUpdates": true,
            "excludeFromAutoUpdate": true,
        }))
        .auto_apply_enabled());

        // A configuration from a connector that has never heard of any of this
        // must not break the scheduler, and neither must a wrongly-typed value.
        let nonsense = UpdateSettings::from_config(&json!({
            "checkForUpdates": "yes please",
            "checkIntervalMinutes": 0,
            "autoApplyAtTime": "half past three",
        }));
        assert!(!nonsense.check_for_updates);
        assert_eq!(nonsense.check_interval_minutes, 360);
        assert_eq!(nonsense.auto_apply_at_minute, None);
    }

    #[test]
    fn a_window_is_a_real_time_of_day() {
        assert_eq!(parse_hh_mm("00:00"), Some(0));
        assert_eq!(parse_hh_mm("23:59"), Some(23 * 60 + 59));
        assert_eq!(parse_hh_mm(" 03:30 "), Some(210));
        for invalid in ["", "24:00", "12:60", "noon", "3"] {
            assert_eq!(parse_hh_mm(invalid), None, "{invalid} is not a time");
        }
    }
}
