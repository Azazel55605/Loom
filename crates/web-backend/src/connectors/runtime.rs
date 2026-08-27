//! The live connectors this instance currently has.
//!
//! One [`Connector`] object per row in `connector_instances`, constructed at
//! startup and kept for the process's lifetime. The map exists because a
//! connector is not a value that can be rebuilt per request: a real one will
//! hold a client, a connection pool, a token cache, and rebuilding it on every
//! poll would throw all of that away. The database row is the durable record;
//! this map is the running thing the row describes.
//!
//! Writes go through here rather than straight to the map so the two can never
//! disagree: creating an instance persists *and* inserts, deleting removes
//! from both, and updating replaces the live entry with one built from the new
//! configuration. Nothing else may hold a long-lived reference to a connector,
//! or an update would leave a stale one in use.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use loom_core::connector::{Connector, ConnectorError, ConnectorStatus, HealthState};
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::sync::{broadcast, RwLock};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{self, MissedTickBehavior};
use uuid::Uuid;

use super::diagnostics;
use super::registry::{ConnectorTypeRegistration, ConnectorTypeRegistry};

/// How often a healthy connector is asked for a fresh status.
///
/// Named and public so tests and operator-facing documentation can refer to
/// the same value rather than duplicating a magic number.
pub const CONNECTOR_POLL_INTERVAL: Duration = Duration::from_secs(5);

/// The ceiling a persistently-failing connector's interval backs off to.
///
/// Two minutes, at the short end of what is defensible. The upper bound on this
/// number is not efficiency, it is how long someone waits to see a service they
/// just fixed *outside* Loom come back — nothing tells us they fixed it, so the
/// next scheduled poll is the soonest we can find out. Anything a user fixes
/// *through* Loom resets the schedule immediately (see [`ConnectorRuntime::
/// refresh_now`]), so this ceiling only governs the case where Loom was not
/// involved.
pub const CONNECTOR_POLL_MAX_INTERVAL: Duration = Duration::from_secs(120);

/// How often the poller wakes to see whose turn it is.
///
/// One second, and it is not the poll interval — it is the resolution of the
/// schedule. A due time can be up to this late, which is invisible next to a
/// five-second base interval, and it keeps the whole poller a single loop
/// rather than a timer task per instance.
pub const POLL_TICK: Duration = Duration::from_secs(1);

/// How long a pending operation may sit before it is assumed lost.
///
/// The marker is cleared when the action returns, so this only ever fires when
/// an action *never* returns — a connector that hangs on a socket with no
/// timeout of its own. Without it, one hung call would leave an instance
/// reading "Performing: Restart" until the process restarted, which is a worse
/// lie than the flicker the overlay exists to prevent. Two minutes is longer
/// than any lifecycle action should take and short enough that nobody watches
/// it for a whole afternoon.
pub const PENDING_OPERATION_TIMEOUT: Duration = Duration::from_secs(120);

/// Minimum gap between network probes for one instance while it stays Down.
///
/// A probe opens a real TCP connection to the user's service. Running one on
/// every failed poll would mean connecting to a struggling host every few
/// seconds, which is a thing to do *to* an outage, not about one. The diagnosis
/// is a stable fact anyway: a host that was unreachable a minute ago is
/// overwhelmingly likely to still be unreachable now.
pub const DIAGNOSIS_INTERVAL: Duration = Duration::from_secs(60);

/// A disruptive action currently running against an instance.
///
/// Its presence is what lets a client say **"Performing: Restart"** instead of
/// reporting the entirely accurate and entirely unhelpful "Down" that a service
/// mid-restart produces.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingOperation {
    /// The action's human label, ready to render — `"Restart"`, not
    /// `"restart"`. Taken from the connector's own `ConnectorAction`, so the
    /// word on the button and the word in the overlay are the same word.
    pub action_label: String,
    /// When it started, so a client can show elapsed time and so the safety net
    /// has something to measure against.
    pub started_at: DateTime<Utc>,
}

/// What a client is told about one instance right now.
///
/// Two layers, deliberately kept apart in the type. `status`/`status_error` are
/// the **poll result** — what the connector said. `pending_operation` and
/// `diagnosis` are the platform's **overlay** — context the connector cannot
/// have, because one is about a request in flight and the other is about the
/// network underneath it.
///
/// Merged into one struct because a client needs them together to render a
/// tile, and split into one field each because a poll result must never be
/// silently overwritten by an overlay: a restarting service really is Down, and
/// a client that wants to know that can still read it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorStatusSnapshot {
    pub status: Option<ConnectorStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_error: Option<ConnectorError>,
    /// Present while a disruptive action is running. Takes visual precedence
    /// over `status` in every client.
    pub pending_operation: Option<PendingOperation>,
    /// A sentence about *why* this instance is Down, established by probing the
    /// network beneath it. `None` when the instance is not Down, when the
    /// connector publishes no network target, or when no probe has run yet.
    pub diagnosis: Option<String>,
}

impl ConnectorStatusSnapshot {
    /// A snapshot carrying only a poll result.
    fn from_poll(status: Option<ConnectorStatus>, status_error: Option<ConnectorError>) -> Self {
        Self {
            status,
            status_error,
            pending_operation: None,
            diagnosis: None,
        }
    }

    /// Whether the connector is failing, for backoff and diagnosis purposes.
    ///
    /// **Both** a poll that could not be carried out *and* a poll that
    /// successfully reported `Down`. The second is not obviously a "failure",
    /// and including it is the deliberate part: the expensive case in practice
    /// is a connector whose every poll spends a full timeout before answering
    /// `Down` — which is exactly what a container connector does when its
    /// daemon has gone away. Backing off only on `Err` would leave the most
    /// common outage polling at full frequency.
    fn is_failing(&self) -> bool {
        match &self.status {
            None => true,
            Some(status) => status.health == HealthState::Down,
        }
    }

    /// Whether a pending operation has outlived the safety net.
    ///
    /// `to_std` fails on a negative duration, which is what a clock stepping
    /// backwards produces — treated as "not expired", so a time adjustment
    /// cannot cancel an operation that is genuinely still running.
    fn has_expired_operation(&self, timeout: Duration, now: DateTime<Utc>) -> bool {
        self.pending_operation.as_ref().is_some_and(|pending| {
            now.signed_duration_since(pending.started_at)
                .to_std()
                .is_ok_and(|elapsed| elapsed >= timeout)
        })
    }

    /// Drops a pending operation that has outlived the safety net.
    fn without_expired_operation(mut self, timeout: Duration, now: DateTime<Utc>) -> Self {
        if self.has_expired_operation(timeout, now) {
            self.pending_operation = None;
        }
        self
    }
}

/// When an instance is next due to be polled, and how it got there.
///
/// Never serialized: this is the poller's own bookkeeping, not something a
/// client has any use for.
#[derive(Debug, Clone)]
struct PollSchedule {
    /// Consecutive failing polls. Resets to zero on the first good one.
    consecutive_failures: u32,
    /// The tick at which this instance becomes eligible again.
    next_due: time::Instant,
    /// When this instance was last probed, for the diagnosis debounce.
    last_diagnosed_at: Option<time::Instant>,
}

impl PollSchedule {
    /// A schedule that is already due at `at`.
    ///
    /// Takes the instant rather than reading the clock, because the caller has
    /// usually captured "now" already and a freshly-read `Instant::now()` is
    /// strictly *later* than it — which would make a brand-new instance miss
    /// the very tick that created its schedule.
    fn due_at(at: time::Instant) -> Self {
        Self {
            consecutive_failures: 0,
            next_due: at,
            last_diagnosed_at: None,
        }
    }

    fn due_now() -> Self {
        Self::due_at(time::Instant::now())
    }

    /// The interval this instance has earned: the base, doubled once per
    /// consecutive failure, capped.
    ///
    /// Doubling rather than a fixed penalty because the cost of a failing poll
    /// is unbounded in *duration* (a timeout) while the value of retrying falls
    /// off with time: the first retry after a blip is worth a lot, the
    /// hundredth after an hour's outage is worth nothing.
    fn interval(&self) -> Duration {
        CONNECTOR_POLL_MAX_INTERVAL.min(
            CONNECTOR_POLL_INTERVAL.saturating_mul(
                1u32.checked_shl(self.consecutive_failures)
                    .unwrap_or(u32::MAX),
            ),
        )
    }

    /// Records the outcome of a poll and schedules the next one.
    fn record(&mut self, failed: bool) {
        if failed {
            // Saturating: 2^32 base intervals is long past the cap anyway, and
            // an overflow here would silently reset the backoff to nothing.
            self.consecutive_failures = self.consecutive_failures.saturating_add(1);
        } else {
            self.consecutive_failures = 0;
        }
        self.next_due = time::Instant::now() + self.interval();
    }
}

/// A status cache change, broadcast to interested WebSocket connections.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectorStatusUpdate {
    pub instance_id: Uuid,
    pub snapshot: ConnectorStatusSnapshot,
}

/// Why an instance could not be constructed from a type id and a configuration.
#[derive(Debug)]
pub enum BuildError {
    /// No such type is registered in this build.
    UnknownType(String),
    /// The type is registered and refused the configuration. Carries the
    /// connector's own objection, so the caller can be told what is wrong with
    /// their input rather than that "something" is.
    Rejected(ConnectorError),
}

/// The live connectors, plus the registry they were built from.
///
/// Cloned per request as part of [`crate::state::AppState`]; both fields are
/// `Arc`, so a clone is two pointer bumps.
#[derive(Clone)]
pub struct ConnectorRuntime {
    types: ConnectorTypeRegistry,
    /// `Arc<dyn Connector>` rather than `Box`, because a handler needs to hold
    /// a connector across an `await` (`status()` and `execute_action()` are
    /// both async) and must not hold the map's lock while doing so. Cloning the
    /// `Arc` out and releasing the guard is what keeps one slow connector from
    /// blocking every other request.
    instances: Arc<RwLock<HashMap<Uuid, Arc<dyn Connector>>>>,
    statuses: Arc<RwLock<HashMap<Uuid, ConnectorStatusSnapshot>>>,
    /// Per-instance backoff and debounce bookkeeping. A separate map from
    /// `statuses` because none of it is ever sent to a client, and folding it
    /// into the serialized type would be one `#[serde(skip)]` per field plus a
    /// standing invitation to leak one.
    schedules: Arc<RwLock<HashMap<Uuid, PollSchedule>>>,
    status_updates: broadcast::Sender<ConnectorStatusUpdate>,
    /// Overridable so a test can watch the safety net fire without waiting two
    /// minutes for it. Never changed in production.
    pending_timeout: Duration,
}

impl ConnectorRuntime {
    /// An empty runtime over `types`.
    pub fn new(types: ConnectorTypeRegistry) -> Self {
        let (status_updates, _) = broadcast::channel(256);
        Self {
            types,
            instances: Arc::new(RwLock::new(HashMap::new())),
            statuses: Arc::new(RwLock::new(HashMap::new())),
            schedules: Arc::new(RwLock::new(HashMap::new())),
            status_updates,
            pending_timeout: PENDING_OPERATION_TIMEOUT,
        }
    }

    /// Shortens the pending-operation safety net, for tests only.
    ///
    /// The behaviour under test is "a marker that outlives its action is
    /// dropped", which is identical at two minutes and at fifty milliseconds —
    /// and only testable at one of them.
    #[cfg(test)]
    pub fn with_pending_timeout(mut self, timeout: Duration) -> Self {
        self.pending_timeout = timeout;
        self
    }

    /// Builds a runtime and populates it from `connector_instances`.
    ///
    /// A row that cannot be turned into a live connector — unknown type,
    /// unparseable id, configuration the factory rejects — is **logged and
    /// skipped**, not fatal. The alternative is a server that refuses to start
    /// because of one bad connector, which would take authentication and every
    /// other connector down with it; the row survives on disk and can be fixed
    /// or deleted through the API. See `docs/adr/0004-zero-config-startup.md`
    /// for why startup fails as rarely as possible.
    pub async fn load(
        pool: &SqlitePool,
        types: ConnectorTypeRegistry,
    ) -> Result<Self, sqlx::Error> {
        let runtime = Self::new(types);

        let rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, connector_type, config FROM connector_instances",
        )
        .fetch_all(pool)
        .await?;

        let mut live = runtime.instances.write().await;
        for (id, connector_type, config) in rows {
            let Ok(uuid) = Uuid::parse_str(&id) else {
                tracing::warn!(instance = %id, "skipping connector instance with an unparseable id");
                continue;
            };

            let config: Value = match serde_json::from_str(&config) {
                Ok(value) => value,
                Err(error) => {
                    tracing::warn!(
                        instance = %id,
                        %error,
                        "skipping connector instance whose stored config is not valid JSON"
                    );
                    continue;
                }
            };

            match runtime.build(&connector_type, config).await {
                Ok(connector) => {
                    live.insert(uuid, connector);
                }
                Err(BuildError::UnknownType(type_id)) => tracing::warn!(
                    instance = %id,
                    connector_type = %type_id,
                    "skipping connector instance of a type this build does not register"
                ),
                Err(BuildError::Rejected(error)) => tracing::warn!(
                    instance = %id,
                    %error,
                    "skipping connector instance the connector refused to be built from"
                ),
            }
        }
        drop(live);

        tracing::info!(count = runtime.len().await, "loaded connector instances");

        Ok(runtime)
    }

    /// The registered connector types.
    pub fn types(&self) -> &ConnectorTypeRegistry {
        &self.types
    }

    /// The registration for `type_id`, if this build has one.
    pub fn registration(&self, type_id: &str) -> Option<&ConnectorTypeRegistration> {
        self.types.get(type_id)
    }

    /// Constructs a connector from a type id and a configuration, without
    /// touching the map.
    ///
    /// Separated from insertion so create and update can validate *before*
    /// they write: a configuration that the connector refuses must never reach
    /// the database, or the next startup would skip the row it created.
    pub async fn build(
        &self,
        type_id: &str,
        config: Value,
    ) -> Result<Arc<dyn Connector>, BuildError> {
        let registration = self
            .registration(type_id)
            .ok_or_else(|| BuildError::UnknownType(type_id.to_owned()))?;

        // Awaited because a connector to a real service validates by using it —
        // see `ConnectorFactory`. The registry lookup is a plain map read, so
        // nothing is held across this await.
        (registration.factory)(config)
            .await
            .map(Arc::from)
            .map_err(BuildError::Rejected)
    }

    /// Inserts or replaces the live connector for `id` and immediately seeds
    /// its cache. This keeps create/update responses useful without making
    /// request handlers call `status()` themselves.
    pub async fn insert(&self, id: Uuid, connector: Arc<dyn Connector>) {
        self.instances.write().await.insert(id, connector);
        self.poll_instance(id).await;
    }

    /// Drops the live connector for `id`.
    pub async fn remove(&self, id: &Uuid) {
        self.instances.write().await.remove(id);
        self.statuses.write().await.remove(id);
        self.schedules.write().await.remove(id);
    }

    /// The live connector for `id`, if there is one.
    ///
    /// Returns a clone of the `Arc` and releases the lock, so the caller can
    /// await on it freely.
    pub async fn get(&self, id: &Uuid) -> Option<Arc<dyn Connector>> {
        self.instances.read().await.get(id).cloned()
    }

    /// How many live connectors there are.
    ///
    /// Used for the startup log line and by tests; listing goes through the
    /// database, which is the ordering authority.
    pub async fn len(&self) -> usize {
        self.instances.read().await.len()
    }

    /// What a client should currently be told about `id`.
    ///
    /// Filters out a pending operation that has outlived the safety net rather
    /// than waiting for the poller tick to prune it, so a read landing between
    /// ticks cannot report an operation that is already presumed lost. The tick
    /// still does the authoritative prune, because that one also broadcasts.
    pub async fn cached_status(&self, id: &Uuid) -> Option<ConnectorStatusSnapshot> {
        self.statuses
            .read()
            .await
            .get(id)
            .cloned()
            .map(|snapshot| snapshot.without_expired_operation(self.pending_timeout, Utc::now()))
    }

    /// Applies `change` to an instance's snapshot and broadcasts if the result
    /// differs from what clients were last told.
    ///
    /// Every write to a snapshot goes through here — polls, operation markers,
    /// diagnoses — so there is exactly one place that decides what "changed"
    /// means and exactly one that sends. An overlay update that did not
    /// broadcast would leave "Performing: Restart" invisible until the next
    /// poll, which is most of the time it needed to be visible for.
    async fn update_snapshot(&self, id: Uuid, change: impl FnOnce(&mut ConnectorStatusSnapshot)) {
        let updated = {
            let mut statuses = self.statuses.write().await;
            let snapshot = statuses
                .entry(id)
                .or_insert_with(|| ConnectorStatusSnapshot::from_poll(None, None));
            let before = snapshot.clone();
            change(snapshot);
            (before != *snapshot).then(|| snapshot.clone())
        };

        if let Some(snapshot) = updated {
            let _ = self.status_updates.send(ConnectorStatusUpdate {
                instance_id: id,
                snapshot,
            });
        }
    }

    /// Marks `id` as running a disruptive action.
    ///
    /// Called before the action is dispatched, so the overlay is already in
    /// place when the service starts refusing connections — the gap between
    /// "the request went out" and "the marker appeared" is exactly where a
    /// spurious Down would be observed.
    pub async fn begin_operation(&self, id: Uuid, action_label: impl Into<String>) {
        let pending = PendingOperation {
            action_label: action_label.into(),
            started_at: Utc::now(),
        };
        self.update_snapshot(id, |snapshot| {
            snapshot.pending_operation = Some(pending);
        })
        .await;
    }

    /// Clears the marker for `id`, whatever the action's outcome was.
    ///
    /// Success and failure both end the operation; a failed restart is not
    /// still being performed. The instance's real state is then whatever the
    /// next poll says, which [`ConnectorRuntime::refresh_now`] brings forward.
    pub async fn end_operation(&self, id: Uuid) {
        self.update_snapshot(id, |snapshot| {
            snapshot.pending_operation = None;
        })
        .await;
    }

    /// Brings an instance's next poll forward to now and clears its backoff.
    ///
    /// Called after an action, because an action is the strongest possible
    /// signal that the state is about to change and that somebody is watching.
    /// Without this, restarting a container that had backed off to two minutes
    /// would leave the dashboard stale for two minutes at the exact moment its
    /// user is looking at it.
    pub async fn refresh_now(&self, id: Uuid) {
        {
            let mut schedules = self.schedules.write().await;
            let schedule = schedules.entry(id).or_insert_with(PollSchedule::due_now);
            // Only the due time. The failure history is deliberately kept: if
            // this poll fails too, the instance goes straight back to the
            // interval it had earned rather than starting its backoff over.
            // Pressing a button is a reason to look now, not evidence that the
            // service is fixed.
            schedule.next_due = time::Instant::now();
        }
        self.poll_instance(id).await;
    }

    /// Subscribe to status changes after the current cache snapshot.
    pub fn subscribe_statuses(&self) -> broadcast::Receiver<ConnectorStatusUpdate> {
        self.status_updates.subscribe()
    }

    /// Poll every live connector once, regardless of when each was last due.
    ///
    /// Used at startup and by tests. The scheduled poller uses
    /// [`ConnectorRuntime::poll_due`] instead; this one ignores backoff on
    /// purpose, because a fresh process has no reason to believe a stale
    /// schedule and startup should report what is true now.
    ///
    /// Connector calls happen without holding the instance map lock. A failed
    /// connector becomes an error snapshot and cannot prevent the remaining
    /// connectors from being polled.
    pub async fn poll_once(&self) {
        let instances: Vec<(Uuid, Arc<dyn Connector>)> = self
            .instances
            .read()
            .await
            .iter()
            .map(|(id, connector)| (*id, Arc::clone(connector)))
            .collect();
        self.poll_all(instances).await;
    }

    /// Poll only the instances whose next-due time has arrived.
    ///
    /// This is what makes backoff mean anything: a connector that has been
    /// failing for a while simply is not in this set most of the time.
    pub async fn poll_due(&self) {
        let now = time::Instant::now();

        let due: Vec<(Uuid, Arc<dyn Connector>)> = {
            let instances = self.instances.read().await;
            let mut schedules = self.schedules.write().await;
            instances
                .iter()
                .filter(|(id, _)| {
                    // An instance with no schedule yet has never been polled by
                    // the loop, so it is due on this very tick — hence
                    // `due_at(now)` rather than `due_now()`, which would read a
                    // later clock and defer it by one tick for no reason.
                    schedules
                        .entry(**id)
                        .or_insert_with(|| PollSchedule::due_at(now))
                        .next_due
                        <= now
                })
                .map(|(id, connector)| (*id, Arc::clone(connector)))
                .collect()
        };

        self.prune_expired_operations().await;
        if due.is_empty() {
            return;
        }
        self.poll_all(due).await;
    }

    /// Polls the given instances concurrently, one task each.
    async fn poll_all(&self, instances: Vec<(Uuid, Arc<dyn Connector>)>) {
        let mut polls = JoinSet::new();
        for (id, connector) in instances {
            let runtime = self.clone();
            polls.spawn(async move {
                runtime.poll_connector(id, connector).await;
            });
        }

        while let Some(result) = polls.join_next().await {
            if let Err(error) = result {
                tracing::error!(%error, "connector status poll task failed");
            }
        }
    }

    /// Drops pending-operation markers whose action never came back.
    ///
    /// Runs on the tick rather than on the poll, so an instance that has backed
    /// off to two minutes still has its stuck marker cleared on time — the
    /// safety net must not inherit the backoff of the thing it is protecting
    /// against.
    async fn prune_expired_operations(&self) {
        let now = Utc::now();
        let expired: Vec<Uuid> = self
            .statuses
            .read()
            .await
            .iter()
            .filter(|(_, snapshot)| snapshot.has_expired_operation(self.pending_timeout, now))
            .map(|(id, _)| *id)
            .collect();

        for id in expired {
            tracing::warn!(
                instance = %id,
                "clearing a pending operation that never reported back"
            );
            self.end_operation(id).await;
        }
    }

    /// Start the process-lifetime polling task.
    ///
    /// Wakes every [`POLL_TICK`] and polls whichever instances are due, rather
    /// than polling everything on one shared interval. The tick is the
    /// schedule's resolution, not its rate: a healthy instance is still polled
    /// every [`CONNECTOR_POLL_INTERVAL`], and a failing one drifts out towards
    /// [`CONNECTOR_POLL_MAX_INTERVAL`] without anything else changing pace.
    ///
    /// One loop, no timer task per instance. A hundred connectors is a hundred
    /// map entries and one wakeup a second, not a hundred sleeping tasks whose
    /// lifetimes have to be reconciled with instances being added and removed.
    ///
    /// Dropping the handle detaches the task; the task owns only cheap clones
    /// of the runtime's `Arc`s.
    pub fn spawn_poller(&self) -> JoinHandle<()> {
        let runtime = self.clone();
        tokio::spawn(async move {
            let start = time::Instant::now() + POLL_TICK;
            let mut interval = time::interval_at(start, POLL_TICK);
            interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

            loop {
                interval.tick().await;
                runtime.poll_due().await;
            }
        })
    }

    async fn poll_instance(&self, id: Uuid) {
        if let Some(connector) = self.get(&id).await {
            self.poll_connector(id, connector).await;
        }
    }

    async fn poll_connector(&self, id: Uuid, connector: Arc<dyn Connector>) {
        let outcome = match connector.status().await {
            Ok(status) => ConnectorStatusSnapshot::from_poll(Some(status), None),
            Err(error) => {
                tracing::warn!(instance = %id, %error, "connector status poll failed");
                ConnectorStatusSnapshot::from_poll(None, Some(error))
            }
        };

        // An update can replace a connector while its old status call is in
        // flight. Never let that late result overwrite the replacement's
        // freshly seeded snapshot.
        let is_current = self
            .instances
            .read()
            .await
            .get(&id)
            .is_some_and(|current| Arc::ptr_eq(current, &connector));
        if !is_current {
            return;
        }

        let failing = outcome.is_failing();
        let interval = {
            let mut schedules = self.schedules.write().await;
            let schedule = schedules.entry(id).or_insert_with(PollSchedule::due_now);
            schedule.record(failing);
            schedule.interval()
        };
        if failing && interval > CONNECTOR_POLL_INTERVAL {
            // Logged so an operator can *see* the backoff happening: successive
            // lines for one instance carry a growing interval and grow further
            // apart, which is the whole behaviour in one place.
            tracing::info!(
                instance = %id,
                next_poll_in_secs = interval.as_secs(),
                "backing off a persistently failing connector"
            );
        }

        // Diagnosis is computed *before* the snapshot is published, so a client
        // never sees a Down status without its explanation and then the same
        // status with one a moment later.
        let diagnosis = if failing {
            self.diagnose_if_due(id, connector.as_ref()).await
        } else {
            None
        };

        self.update_snapshot(id, |snapshot| {
            snapshot.status = outcome.status;
            snapshot.status_error = outcome.status_error;
            if failing {
                // An existing diagnosis survives a poll that was not due for a
                // fresh probe; a recovery clears it, because a sentence about
                // why something is unreachable is worse than nothing once it is
                // reachable.
                if diagnosis.is_some() {
                    snapshot.diagnosis = diagnosis;
                }
            } else {
                snapshot.diagnosis = None;
            }
        })
        .await;
    }

    /// Probes an instance's network target, unless it was probed recently.
    ///
    /// Returns `None` both when the debounce blocks a probe and when the
    /// connector publishes no target — the caller treats them the same way, by
    /// leaving any existing diagnosis alone.
    async fn diagnose_if_due(&self, id: Uuid, connector: &dyn Connector) -> Option<String> {
        let target = connector.network_target()?;

        {
            let mut schedules = self.schedules.write().await;
            let schedule = schedules.entry(id).or_insert_with(PollSchedule::due_now);
            let now = time::Instant::now();
            if schedule
                .last_diagnosed_at
                .is_some_and(|last| now.duration_since(last) < DIAGNOSIS_INTERVAL)
            {
                return None;
            }
            schedule.last_diagnosed_at = Some(now);
        }

        diagnostics::diagnose(&target).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::registry::builtin_registry;
    use loom_core::connector::debug::TYPE_ID as DEBUG_TYPE_ID;
    use serde_json::json;

    /// A connector whose polls always fail, for exercising backoff without
    /// waiting on anything real.
    async fn failing_instance(runtime: &ConnectorRuntime) -> Uuid {
        let id = Uuid::new_v4();
        let connector = runtime
            .build(DEBUG_TYPE_ID, json!({ "failMode": "unreachable" }))
            .await
            .expect("the fixture builds");
        runtime.instances.write().await.insert(id, connector);
        id
    }

    async fn schedule_of(runtime: &ConnectorRuntime, id: &Uuid) -> PollSchedule {
        runtime
            .schedules
            .read()
            .await
            .get(id)
            .cloned()
            .expect("a polled instance has a schedule")
    }

    #[tokio::test]
    async fn a_pending_operation_overlays_the_status_and_is_cleared_afterwards() {
        let runtime = ConnectorRuntime::new(builtin_registry());
        let id = Uuid::new_v4();
        let connector = runtime
            .build(DEBUG_TYPE_ID, json!({}))
            .await
            .expect("the fixture builds");
        runtime.insert(id, connector).await;

        // The poll result underneath the overlay is healthy, and stays healthy:
        // the marker is an addition, not a replacement, so a client that wants
        // the real state can still read it.
        let before = runtime.cached_status(&id).await.expect("a seeded status");
        assert!(before.pending_operation.is_none());
        assert_eq!(
            before.status.as_ref().map(|status| status.health),
            Some(HealthState::Healthy)
        );

        let mut updates = runtime.subscribe_statuses();
        runtime.begin_operation(id, "Restart").await;

        let during = runtime.cached_status(&id).await.expect("status");
        let pending = during
            .pending_operation
            .as_ref()
            .expect("the operation is in flight");
        assert_eq!(pending.action_label, "Restart");
        assert_eq!(
            during.status.as_ref().map(|status| status.health),
            Some(HealthState::Healthy),
            "the overlay must not overwrite the poll result"
        );

        // Pushed, not merely stored. A marker a client only learns about at the
        // next poll is a marker that was invisible for most of the window it
        // existed to cover.
        let update = updates
            .try_recv()
            .expect("beginning an operation is pushed");
        assert_eq!(update.instance_id, id);
        assert_eq!(
            update
                .snapshot
                .pending_operation
                .map(|pending| pending.action_label),
            Some("Restart".to_owned())
        );

        runtime.end_operation(id).await;
        assert!(runtime
            .cached_status(&id)
            .await
            .expect("status")
            .pending_operation
            .is_none());
        assert!(updates.try_recv().is_ok(), "clearing it is pushed too");
    }

    /// The safety net: a marker whose action never reports back must not pin an
    /// instance to "Performing…" for the life of the process.
    #[tokio::test]
    async fn a_pending_operation_that_never_returns_is_eventually_dropped() {
        let runtime = ConnectorRuntime::new(builtin_registry())
            .with_pending_timeout(Duration::from_millis(50));
        let id = Uuid::new_v4();
        let connector = runtime
            .build(DEBUG_TYPE_ID, json!({}))
            .await
            .expect("the fixture builds");
        runtime.insert(id, connector).await;

        runtime.begin_operation(id, "Restart").await;
        assert!(runtime
            .cached_status(&id)
            .await
            .expect("status")
            .pending_operation
            .is_some());

        tokio::time::sleep(Duration::from_millis(80)).await;

        // A read past the deadline is honest immediately, without waiting for
        // the tick that will prune it.
        assert!(
            runtime
                .cached_status(&id)
                .await
                .expect("status")
                .pending_operation
                .is_none(),
            "an expired marker must not be reported to a client"
        );

        // ...and the tick performs the authoritative removal, which is what
        // pushes the correction to anyone already connected.
        let mut updates = runtime.subscribe_statuses();
        runtime.prune_expired_operations().await;
        let update = updates.try_recv().expect("the correction is pushed");
        assert!(update.snapshot.pending_operation.is_none());
        assert!(
            runtime
                .statuses
                .read()
                .await
                .get(&id)
                .is_some_and(|snapshot| snapshot.pending_operation.is_none()),
            "the stored snapshot is cleared, not merely filtered on read"
        );
    }

    #[tokio::test]
    async fn repeated_failures_back_the_poll_interval_off_and_a_success_resets_it() {
        let runtime = ConnectorRuntime::new(builtin_registry());
        let id = failing_instance(&runtime).await;

        // Base interval before anything has failed.
        runtime.poll_once().await;
        let first = schedule_of(&runtime, &id).await;
        assert_eq!(first.consecutive_failures, 1);
        assert_eq!(first.interval(), CONNECTOR_POLL_INTERVAL * 2);

        // Each further failure doubles it.
        for expected in [4u32, 8, 16] {
            runtime.poll_once().await;
            assert_eq!(
                schedule_of(&runtime, &id).await.interval(),
                CONNECTOR_POLL_INTERVAL * expected
            );
        }

        // And it is capped rather than growing without bound.
        for _ in 0..20 {
            runtime.poll_once().await;
        }
        assert_eq!(
            schedule_of(&runtime, &id).await.interval(),
            CONNECTOR_POLL_MAX_INTERVAL
        );

        // One good poll resets it all the way, not one step: a connector that
        // has just answered is not "slightly less broken", it is working.
        let healthy = runtime
            .build(DEBUG_TYPE_ID, json!({}))
            .await
            .expect("the fixture builds");
        runtime.instances.write().await.insert(id, healthy);
        runtime.poll_once().await;

        let recovered = schedule_of(&runtime, &id).await;
        assert_eq!(recovered.consecutive_failures, 0);
        assert_eq!(recovered.interval(), CONNECTOR_POLL_INTERVAL);
    }

    #[tokio::test]
    async fn only_instances_whose_turn_has_come_are_polled() {
        let runtime = ConnectorRuntime::new(builtin_registry());
        let id = failing_instance(&runtime).await;

        // First pass: never polled, so due immediately.
        runtime.poll_due().await;
        assert_eq!(schedule_of(&runtime, &id).await.consecutive_failures, 1);

        // Second pass, moments later: the backed-off instance is not due, so
        // the tick does nothing. This is the behaviour backoff *is*.
        runtime.poll_due().await;
        assert_eq!(
            schedule_of(&runtime, &id).await.consecutive_failures,
            1,
            "an instance that is not due must not be polled"
        );

        // An action brings the next poll forward and runs it there and then,
        // so a user who just pressed a button sees the result rather than
        // waiting out a backoff they cannot see.
        runtime.refresh_now(id).await;
        let after_action = schedule_of(&runtime, &id).await;
        assert_eq!(
            after_action.consecutive_failures, 2,
            "refreshing polls immediately, and this instance is still failing"
        );
        assert_eq!(
            after_action.interval(),
            CONNECTOR_POLL_INTERVAL * 4,
            "the failure history survives a refresh: pressing a button is a reason to \
             look now, not evidence that the service is fixed"
        );

        // And the tick still respects the new due time, so a refresh does not
        // leave the instance polling at full frequency.
        runtime.poll_due().await;
        assert_eq!(schedule_of(&runtime, &id).await.consecutive_failures, 2);
    }

    /// The end-to-end diagnostic path, through a fixture pointed at an address
    /// that is guaranteed to refuse a connection.
    #[tokio::test]
    async fn a_down_instance_with_a_network_target_gets_a_diagnosis() {
        let runtime = ConnectorRuntime::new(builtin_registry());
        let id = Uuid::new_v4();
        // Loopback port 1 is reserved and nothing binds it: the connect is
        // refused immediately, with no network and no timeout involved.
        let connector = runtime
            .build(
                DEBUG_TYPE_ID,
                json!({
                    "simulatedHealth": "down",
                    "networkTarget": { "host": "127.0.0.1", "port": 1 }
                }),
            )
            .await
            .expect("the fixture builds");
        runtime.instances.write().await.insert(id, connector);

        runtime.poll_once().await;
        let snapshot = runtime.cached_status(&id).await.expect("status");
        assert_eq!(
            snapshot.status.as_ref().map(|status| status.health),
            Some(HealthState::Down)
        );
        let diagnosis = snapshot
            .diagnosis
            .expect("a Down instance gets an explanation");
        assert!(
            diagnosis.contains("unreachable on port `1`"),
            "should name the port that was tried: {diagnosis}"
        );

        // Debounced: a second poll inside the window reuses the diagnosis
        // rather than opening another connection to a struggling host.
        let before = schedule_of(&runtime, &id).await.last_diagnosed_at;
        runtime.poll_once().await;
        assert_eq!(
            schedule_of(&runtime, &id).await.last_diagnosed_at,
            before,
            "the probe must not re-run on every failed poll"
        );
        assert!(runtime
            .cached_status(&id)
            .await
            .expect("status")
            .diagnosis
            .is_some());

        // Recovery clears it. A sentence about why something is unreachable is
        // worse than no sentence once it is reachable.
        let healthy = runtime
            .build(DEBUG_TYPE_ID, json!({}))
            .await
            .expect("the fixture builds");
        runtime.instances.write().await.insert(id, healthy);
        runtime.poll_once().await;
        assert!(runtime
            .cached_status(&id)
            .await
            .expect("status")
            .diagnosis
            .is_none());
    }

    #[tokio::test]
    async fn an_instance_with_no_network_target_gets_no_diagnosis() {
        // The fixture publishes no target unless configured with one, and a
        // connector with nothing to probe must not be given a made-up
        // explanation.
        let runtime = ConnectorRuntime::new(builtin_registry());
        let id = Uuid::new_v4();
        let connector = runtime
            .build(DEBUG_TYPE_ID, json!({ "simulatedHealth": "down" }))
            .await
            .expect("the fixture builds");
        runtime.instances.write().await.insert(id, connector);

        runtime.poll_once().await;
        let snapshot = runtime.cached_status(&id).await.expect("status");
        assert_eq!(
            snapshot.status.as_ref().map(|status| status.health),
            Some(HealthState::Down)
        );
        assert!(snapshot.diagnosis.is_none());
    }

    #[tokio::test]
    async fn building_reports_an_unknown_type_separately_from_a_refused_config() {
        let runtime = ConnectorRuntime::new(builtin_registry());

        assert!(matches!(
            runtime.build("not-a-type", json!({})).await,
            Err(BuildError::UnknownType(type_id)) if type_id == "not-a-type"
        ));

        assert!(matches!(
            runtime
                .build(DEBUG_TYPE_ID, json!({ "baseLoad": 900 }))
                .await,
            Err(BuildError::Rejected(ConnectorError::InvalidConfig { .. }))
        ));

        assert!(runtime.build(DEBUG_TYPE_ID, json!({})).await.is_ok());
    }

    #[tokio::test]
    async fn instances_can_be_inserted_replaced_and_removed() {
        let runtime = ConnectorRuntime::new(builtin_registry());
        let id = Uuid::new_v4();

        assert!(runtime.get(&id).await.is_none());

        runtime
            .insert(id, runtime.build(DEBUG_TYPE_ID, json!({})).await.unwrap())
            .await;
        assert!(runtime.get(&id).await.is_some());
        assert!(runtime.cached_status(&id).await.is_some());
        assert_eq!(runtime.len().await, 1);

        // Replacing must not leave the old connector reachable.
        runtime
            .insert(
                id,
                runtime
                    .build(DEBUG_TYPE_ID, json!({ "label": "replaced" }))
                    .await
                    .unwrap(),
            )
            .await;
        assert_eq!(runtime.len().await, 1);
        assert!(runtime
            .get(&id)
            .await
            .unwrap()
            .display_fields()
            .iter()
            .any(|field| field.value == "replaced"));

        runtime.remove(&id).await;
        assert!(runtime.get(&id).await.is_none());
        assert!(runtime.cached_status(&id).await.is_none());
        assert_eq!(runtime.len().await, 0);
    }

    #[tokio::test]
    async fn polling_updates_the_cache_and_broadcasts_changes() {
        let runtime = ConnectorRuntime::new(builtin_registry());
        let id = Uuid::new_v4();
        let mut updates = runtime.subscribe_statuses();
        let connector = runtime
            .build(DEBUG_TYPE_ID, json!({ "label": "before" }))
            .await
            .unwrap();

        runtime.insert(id, Arc::clone(&connector)).await;
        let initial = updates.recv().await.expect("insert poll must broadcast");
        assert_eq!(initial.instance_id, id);
        assert!(initial.snapshot.status.is_some());

        connector
            .execute_action("set-label", None, json!({ "label": "after" }))
            .await
            .expect("debug action must succeed");
        runtime.poll_once().await;

        let changed = updates.recv().await.expect("changed poll must broadcast");
        assert_eq!(changed.instance_id, id);
        let details = &changed
            .snapshot
            .status
            .as_ref()
            .expect("successful status")
            .details;
        assert_eq!(
            loom_core::connector::details::get_detail(details, None, "label"),
            Some(&json!("after"))
        );
        assert_eq!(runtime.cached_status(&id).await, Some(changed.snapshot));
    }

    #[tokio::test]
    async fn a_poll_failure_is_cached_instead_of_stopping_the_runtime() {
        let runtime = ConnectorRuntime::new(builtin_registry());
        let id = Uuid::new_v4();
        let connector = runtime
            .build(DEBUG_TYPE_ID, json!({ "failMode": "unreachable" }))
            .await
            .unwrap();

        runtime.insert(id, connector).await;

        let snapshot = runtime.cached_status(&id).await.expect("poll result");
        assert!(snapshot.status.is_none());
        assert!(matches!(
            snapshot.status_error,
            Some(ConnectorError::Unreachable { .. })
        ));
    }

    #[tokio::test]
    async fn a_late_poll_from_a_replaced_connector_cannot_overwrite_the_new_cache() {
        let runtime = ConnectorRuntime::new(builtin_registry());
        let id = Uuid::new_v4();
        runtime
            .insert(
                id,
                runtime
                    .build(
                        DEBUG_TYPE_ID,
                        json!({ "simulatedLatencyMs": 50, "label": "old" }),
                    )
                    .await
                    .unwrap(),
            )
            .await;

        let polling_runtime = runtime.clone();
        let old_poll = tokio::spawn(async move { polling_runtime.poll_once().await });
        tokio::time::sleep(Duration::from_millis(5)).await;
        runtime
            .insert(
                id,
                runtime
                    .build(DEBUG_TYPE_ID, json!({ "label": "new" }))
                    .await
                    .unwrap(),
            )
            .await;
        old_poll.await.expect("poll task must finish");

        let snapshot = runtime.cached_status(&id).await.expect("new snapshot");
        let details = snapshot.status.expect("successful status").details;
        assert_eq!(
            loom_core::connector::details::get_detail(&details, None, "label"),
            Some(&json!("new"))
        );
    }
}
