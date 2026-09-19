//! The live connectors this instance currently has.
//!
//! One entry per row in `connector_instances`, constructed at startup and kept
//! for the process's lifetime. An entry is either [`InstanceState::Live`] — a
//! built [`Connector`] — or [`InstanceState::Pending`], a row whose factory
//! failed and which is waiting to be built again. The map exists because a
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
//!
//! # Construction is a failure mode like any other
//!
//! Building a connector contacts its service — a Docker daemon is pinged, a
//! Music Assistant socket is opened, a Pi-hole session is authenticated — so
//! construction fails for exactly the reasons a poll fails, and just as
//! temporarily. A row whose factory fails therefore becomes a `Pending` entry
//! rather than being dropped: it keeps a place in the map, reports Down with
//! the real error, and is retried by the poller on the same backoff schedule a
//! live-but-failing connector earns. There is deliberately no second retry
//! system for construction — [`ConnectorRuntime::poll_due`] drives both, so
//! one instance cannot be "retried" by machinery the other does not share.
//!
//! Before this, a factory failure at startup logged a line and excluded the
//! row from the map, and nothing polled what was not in the map: the instance
//! stayed dark until somebody re-saved it by hand, even after its service came
//! back. See `docs/adr/0018-connector-offline-handling.md`.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use chrono::{DateTime, Utc};
use loom_core::connector::{
    Connector, ConnectorError, ConnectorStatus, HealthState, NetworkTarget,
};
use serde::Serialize;
use serde_json::Value;
use sqlx::SqlitePool;
use tokio::sync::{broadcast, watch, RwLock};
use tokio::task::{JoinHandle, JoinSet};
use tokio::time::{self, MissedTickBehavior};
use uuid::Uuid;

use super::config_secrets::{decrypt_sensitive_fields, ConfigEncryptionKey};
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

/// How long one instance's factory may run before it is treated as failed.
///
/// Construction opens real connections, and a service that is half-up can
/// accept a socket and then never answer. Without a bound, one such instance
/// would hold up every other instance's construction and delay the HTTP
/// listener binding behind it — which is how a single sick dependency used to
/// turn into a server that looked hung at startup. Twelve seconds is longer
/// than any healthy handshake and short enough that a stuck one is simply a
/// failed attempt, retried on the ordinary schedule like any other.
pub const CONSTRUCTION_TIMEOUT: Duration = Duration::from_secs(12);

/// Recent TCP-connect failures remain relevant to the shared-outage advisory
/// for this long. A rolling window avoids declaring a network incident from
/// failures that happened hours apart while still tolerating backed-off polls.
pub const NETWORK_ADVISORY_WINDOW: Duration = Duration::from_secs(120);

/// Distinct unreachable hosts required before a local network problem becomes
/// more plausible than several unrelated service failures.
pub const NETWORK_ADVISORY_THRESHOLD: usize = 3;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkAdvisory {
    pub active: bool,
    pub affected_host_count: usize,
}

#[derive(Debug, Clone, Copy)]
struct NetworkFailureObservation {
    address: IpAddr,
    observed_at: time::Instant,
}

#[derive(Debug, Default)]
struct NetworkOutageTracker {
    observations: HashMap<Uuid, NetworkFailureObservation>,
}

impl NetworkOutageTracker {
    fn record(&mut self, id: Uuid, address: Option<IpAddr>, now: time::Instant) {
        match address {
            Some(address) => {
                self.observations.insert(
                    id,
                    NetworkFailureObservation {
                        address,
                        observed_at: now,
                    },
                );
            }
            None => {
                self.observations.remove(&id);
            }
        }
        self.prune(now);
    }

    fn remove(&mut self, id: &Uuid, now: time::Instant) {
        self.observations.remove(id);
        self.prune(now);
    }

    fn prune(&mut self, now: time::Instant) {
        self.observations.retain(|_, observation| {
            now.duration_since(observation.observed_at) < NETWORK_ADVISORY_WINDOW
        });
    }

    fn advisory(&self) -> NetworkAdvisory {
        let affected_host_count = self
            .observations
            .values()
            .map(|observation| observation.address)
            .collect::<std::collections::HashSet<_>>()
            .len();
        NetworkAdvisory {
            active: affected_host_count >= NETWORK_ADVISORY_THRESHOLD,
            affected_host_count,
        }
    }
}

enum DiagnosticProbe {
    Skipped,
    Completed(Option<diagnostics::NetworkDiagnosis>),
}

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

/// An instance whose connector could not be built, kept so it can be retried.
///
/// Holds everything the next attempt needs, so a retry does not have to go back
/// to the database: the type id to look the factory up with, the decrypted
/// configuration to hand it, and what went wrong last time. The configuration
/// is held in plaintext because that is the only form a factory accepts — the
/// same form a live connector already holds internally, so this is not a new
/// exposure.
pub struct PendingInstance {
    connector_type: String,
    config: Value,
    last_error: ConnectorError,
}

/// One entry in the instance map: either built, or waiting to be built.
///
/// An enum rather than "present or absent", because absence cannot be retried
/// and cannot be reported. A row that failed to build is a thing this process
/// knows about and has an opinion on, and both of those need somewhere to live.
#[derive(Clone)]
enum InstanceState {
    Live(Arc<dyn Connector>),
    Pending(Arc<PendingInstance>),
}

/// Why [`ConnectorRuntime::ensure_live`] could not produce a connector.
pub enum EnsureError {
    /// This process has no entry for the id at all.
    Unknown,
    /// An attempt was made just now, and this is what it said.
    Construction(ConnectorError),
}

/// The status a pending instance reports.
///
/// Down rather than "no reading", and the distinction is not cosmetic: "no
/// reading" means Loom does not know, whereas a factory that has just failed is
/// positive knowledge that the service is not usable. The real error travels
/// alongside in `status_error`, so the tile says Down and says why.
fn pending_status() -> ConnectorStatus {
    ConnectorStatus::new(HealthState::Down, Value::Object(Default::default()))
}

/// The error a client is shown for an instance that has no connector behind it.
///
/// Keeps the established "was not loaded" phrasing, which clients already
/// render, and adds what was previously only ever written to the log: the
/// factory's own objection.
fn not_loaded_error(error: &ConnectorError) -> ConnectorError {
    ConnectorError::Internal(format!("this instance was not loaded: {error}"))
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
    instances: Arc<RwLock<HashMap<Uuid, InstanceState>>>,
    statuses: Arc<RwLock<HashMap<Uuid, ConnectorStatusSnapshot>>>,
    /// Per-instance backoff and debounce bookkeeping. A separate map from
    /// `statuses` because none of it is ever sent to a client, and folding it
    /// into the serialized type would be one `#[serde(skip)]` per field plus a
    /// standing invitation to leak one.
    schedules: Arc<RwLock<HashMap<Uuid, PollSchedule>>>,
    status_updates: broadcast::Sender<ConnectorStatusUpdate>,
    network_failures: Arc<RwLock<NetworkOutageTracker>>,
    network_advisory: watch::Sender<NetworkAdvisory>,
    /// Overridable so a test can watch the safety net fire without waiting two
    /// minutes for it. Never changed in production.
    pending_timeout: Duration,
    /// Overridable for the same reason as `pending_timeout`: a test that proves
    /// a hung factory cannot hold up the others should not take twelve seconds
    /// to say so. Never changed in production.
    construction_timeout: Duration,
}

impl ConnectorRuntime {
    /// An empty runtime over `types`.
    pub fn new(types: ConnectorTypeRegistry) -> Self {
        let (status_updates, _) = broadcast::channel(256);
        let (network_advisory, _) = watch::channel(NetworkAdvisory::default());
        Self {
            types,
            instances: Arc::new(RwLock::new(HashMap::new())),
            statuses: Arc::new(RwLock::new(HashMap::new())),
            schedules: Arc::new(RwLock::new(HashMap::new())),
            status_updates,
            network_failures: Arc::new(RwLock::new(NetworkOutageTracker::default())),
            network_advisory,
            pending_timeout: PENDING_OPERATION_TIMEOUT,
            construction_timeout: CONSTRUCTION_TIMEOUT,
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

    /// Shortens the per-instance construction bound, for tests only.
    #[cfg(test)]
    pub fn with_construction_timeout(mut self, timeout: Duration) -> Self {
        self.construction_timeout = timeout;
        self
    }

    /// Builds a runtime and populates it from `connector_instances`.
    ///
    /// Every row ends up in the map. A row whose factory fails is inserted as
    /// [`InstanceState::Pending`] carrying the real error and a due-now poll
    /// schedule, so the poller retries it exactly as it retries a connector
    /// that is built but failing. Only a row that cannot be *addressed* at all
    /// is skipped — an unparseable id, configuration that is not JSON, a type
    /// this build does not register, configuration that will not decrypt —
    /// because none of those become true later and retrying them forever would
    /// be a loop with no exit. Those rows survive on disk, remain listed, and
    /// can be fixed or deleted through the API. See
    /// `docs/adr/0004-zero-config-startup.md` for why startup fails as rarely
    /// as possible.
    ///
    /// Construction runs concurrently, one task per row, each bounded by
    /// [`CONSTRUCTION_TIMEOUT`]. Sequential construction under a single write
    /// lock meant total startup time was the *sum* of every connector's
    /// handshake, and one unreachable service delayed every other connector and
    /// the HTTP listener behind it. The lock is now taken per instance, for the
    /// insert alone, never across a factory call.
    pub async fn load(
        pool: &SqlitePool,
        types: ConnectorTypeRegistry,
        config_encryption_key: &ConfigEncryptionKey,
    ) -> Result<Self, sqlx::Error> {
        Self::new(types)
            .load_into(pool, config_encryption_key)
            .await
    }

    /// [`ConnectorRuntime::load`] onto an already-configured runtime, so a test
    /// can shorten the construction bound before rows are read.
    async fn load_into(
        self,
        pool: &SqlitePool,
        config_encryption_key: &ConfigEncryptionKey,
    ) -> Result<Self, sqlx::Error> {
        let rows = sqlx::query_as::<_, (String, String, String)>(
            "SELECT id, connector_type, config FROM connector_instances",
        )
        .fetch_all(pool)
        .await?;

        let mut builds = JoinSet::new();
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

            // Decryption happens here rather than in the task because it needs
            // the registration's schema, which is borrowed from `self`.
            let config = {
                let Some(registration) = self.registration(&connector_type) else {
                    tracing::warn!(
                        instance = %id,
                        connector_type,
                        "skipping connector instance of a type this build does not register"
                    );
                    continue;
                };
                match decrypt_sensitive_fields(&config, &registration.schema, config_encryption_key)
                {
                    Ok(config) => config,
                    Err(error) => {
                        tracing::warn!(
                            instance = %id,
                            %error,
                            "skipping connector instance whose sensitive config cannot be decrypted"
                        );
                        continue;
                    }
                }
            };

            let runtime = self.clone();
            builds.spawn(async move {
                runtime.load_one(uuid, connector_type, config).await;
            });
        }

        while let Some(result) = builds.join_next().await {
            if let Err(error) = result {
                tracing::error!(%error, "connector construction task failed");
            }
        }

        let (live, pending) = self.counts().await;
        tracing::info!(live, pending, "loaded connector instances");

        Ok(self)
    }

    /// Builds one row into the map, as `Live` or as `Pending`.
    async fn load_one(&self, id: Uuid, connector_type: String, config: Value) {
        match self.construct(&connector_type, &config).await {
            Ok(connector) => {
                self.instances
                    .write()
                    .await
                    .insert(id, InstanceState::Live(connector));
            }
            Err(error) => {
                // Deliberately not "skipping": the row is in the map, it is
                // reported to clients as Down with this error, and the poller
                // will build it again without anyone asking.
                tracing::warn!(
                    instance = %id,
                    connector_type = %connector_type,
                    %error,
                    "connector instance could not be built yet; it is pending and will be \
                     retried automatically on the poll schedule"
                );
                self.insert_pending(
                    id,
                    PendingInstance {
                        connector_type,
                        config,
                        last_error: error,
                    },
                )
                .await;
            }
        }
    }

    /// One bounded construction attempt, with both failure kinds flattened into
    /// the connector's own error type.
    ///
    /// `BuildError::UnknownType` cannot happen on a retry — the registration was
    /// checked before the entry was created — but it is mapped rather than
    /// unwrapped, because a panic here would take down the poller.
    async fn construct(
        &self,
        type_id: &str,
        config: &Value,
    ) -> Result<Arc<dyn Connector>, ConnectorError> {
        match time::timeout(
            self.construction_timeout,
            self.build(type_id, config.clone()),
        )
        .await
        {
            Ok(Ok(connector)) => Ok(connector),
            Ok(Err(BuildError::Rejected(error))) => Err(error),
            Ok(Err(BuildError::UnknownType(type_id))) => Err(ConnectorError::Internal(format!(
                "no connector of type `{type_id}` is registered in this build"
            ))),
            Err(_) => Err(ConnectorError::unreachable(format!(
                "the connector did not finish connecting within {} seconds",
                self.construction_timeout.as_secs().max(1)
            ))),
        }
    }

    /// How many entries are built, and how many are waiting to be.
    async fn counts(&self) -> (usize, usize) {
        let instances = self.instances.read().await;
        let pending = instances
            .values()
            .filter(|state| matches!(state, InstanceState::Pending(_)))
            .count();
        (instances.len() - pending, pending)
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

    /// Constructs the throwaway connector used by the setup capability check.
    pub async fn build_for_connection_test(
        &self,
        type_id: &str,
        config: Value,
    ) -> Result<Arc<dyn Connector>, BuildError> {
        let registration = self
            .registration(type_id)
            .ok_or_else(|| BuildError::UnknownType(type_id.to_owned()))?;
        let factory = registration
            .connection_test_factory
            .unwrap_or(registration.factory);

        factory(config)
            .await
            .map(Arc::from)
            .map_err(BuildError::Rejected)
    }

    /// Inserts or replaces the live connector for `id` and schedules a poll.
    ///
    /// Status collection must not be awaited by the create/update request. A
    /// Docker instance can expose dozens of containers, and its poll includes
    /// real remote I/O; making persistence wait for that work turns a valid
    /// connector into a client-side "network error" even though the row and
    /// connection were accepted successfully. The process poller picks this up
    /// on its next one-second tick and publishes the resulting snapshot.
    pub async fn insert(&self, id: Uuid, connector: Arc<dyn Connector>) {
        self.instances
            .write()
            .await
            .insert(id, InstanceState::Live(connector));
        self.statuses.write().await.remove(&id);
        self.schedules
            .write()
            .await
            .insert(id, PollSchedule::due_now());
        self.record_network_failure(id, None).await;
    }

    /// Records an instance that could not be built, due for a retry now.
    ///
    /// The mirror of [`ConnectorRuntime::insert`] for the failing case, and
    /// deliberately the same shape: an entry in the map, a seeded status, and a
    /// due-now schedule. A fresh schedule rather than a backed-off one because
    /// this is the *first* attempt's outcome — the backoff is earned by the
    /// retries that follow, through the same path a failing poll uses.
    async fn insert_pending(&self, id: Uuid, pending: PendingInstance) {
        let reported = not_loaded_error(&pending.last_error);
        self.instances
            .write()
            .await
            .insert(id, InstanceState::Pending(Arc::new(pending)));
        self.schedules
            .write()
            .await
            .insert(id, PollSchedule::due_now());
        self.update_snapshot(id, |snapshot| {
            snapshot.status = Some(pending_status());
            snapshot.status_error = Some(reported);
        })
        .await;
    }

    /// Drops the live connector for `id`.
    pub async fn remove(&self, id: &Uuid) {
        self.instances.write().await.remove(id);
        self.statuses.write().await.remove(id);
        self.schedules.write().await.remove(id);
        self.remove_network_failure(id).await;
    }

    /// The live connector for `id`, if there is one.
    ///
    /// Returns a clone of the `Arc` and releases the lock, so the caller can
    /// await on it freely.
    pub async fn get(&self, id: &Uuid) -> Option<Arc<dyn Connector>> {
        match self.instances.read().await.get(id) {
            Some(InstanceState::Live(connector)) => Some(Arc::clone(connector)),
            Some(InstanceState::Pending(_)) | None => None,
        }
    }

    /// The live connector for `id`, building it first if it is still pending.
    ///
    /// For anything a person just asked for. [`ConnectorRuntime::get`] answers
    /// "is there one right now", which is what rendering wants; this answers
    /// "get me one", which is what pressing a button wants — and a button press
    /// is the strongest possible signal that somebody is waiting, so it does not
    /// wait for the backoff to come around. A failed attempt still records
    /// itself through the ordinary path, so pressing repeatedly cannot reset the
    /// backoff a connector has earned.
    pub async fn ensure_live(&self, id: &Uuid) -> Result<Arc<dyn Connector>, EnsureError> {
        let state = self.instances.read().await.get(id).cloned();
        match state {
            Some(InstanceState::Live(connector)) => Ok(connector),
            Some(InstanceState::Pending(pending)) => {
                self.retry_construction(*id, Arc::clone(&pending)).await;
                match self.instances.read().await.get(id) {
                    Some(InstanceState::Live(connector)) => Ok(Arc::clone(connector)),
                    Some(InstanceState::Pending(current)) => {
                        Err(EnsureError::Construction(current.last_error.clone()))
                    }
                    None => Err(EnsureError::Unknown),
                }
            }
            None => Err(EnsureError::Unknown),
        }
    }

    /// How many entries there are, built or pending.
    ///
    /// Used by tests; listing goes through the database, which is the ordering
    /// authority, and the startup log reports `counts()` instead so that live
    /// and pending instances are distinguishable in it.
    #[cfg(test)]
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

    /// Brings an instance's next poll forward to now without blocking the
    /// action response on that poll's remote I/O.
    ///
    /// Called after an action, because an action is the strongest possible
    /// signal that the state is about to change and that somebody is watching.
    /// Without this, restarting a container that had backed off to two minutes
    /// would leave the dashboard stale for two minutes at the exact moment its
    /// user is looking at it.
    pub async fn refresh_now(&self, id: Uuid) {
        let mut schedules = self.schedules.write().await;
        let schedule = schedules.entry(id).or_insert_with(PollSchedule::due_now);
        // Only the due time. The failure history is deliberately kept: if the
        // eventual poll fails too, the instance goes straight back to the
        // interval it had earned rather than starting its backoff over.
        // Pressing a button is a reason to look soon, not evidence that the
        // service is fixed.
        schedule.next_due = time::Instant::now();
    }

    /// Subscribe to status changes after the current cache snapshot.
    pub fn subscribe_statuses(&self) -> broadcast::Receiver<ConnectorStatusUpdate> {
        self.status_updates.subscribe()
    }

    /// Subscribe to the retained system-wide network advisory state.
    pub fn subscribe_network_advisory(&self) -> watch::Receiver<NetworkAdvisory> {
        self.network_advisory.subscribe()
    }

    /// Immediately polls one instance, bypassing its current backoff.
    ///
    /// The ordinary outcome path remains authoritative: success resets the
    /// schedule to the base cadence, while a failed retry records another
    /// failure and keeps backing off. Cache updates and WebSocket pushes also
    /// pass through the same single path used by scheduled polls.
    pub async fn reconnect(&self, id: Uuid) -> Option<ConnectorStatusSnapshot> {
        let state = self.instances.read().await.get(&id).cloned()?;
        match state {
            InstanceState::Live(connector) => self.poll_connector(id, connector).await,
            // A pending instance retried here is the whole point of the button
            // for somebody who has just fixed the service it could not reach.
            InstanceState::Pending(pending) => self.retry_construction(id, pending).await,
        }
        self.cached_status(&id).await
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
    #[cfg(test)]
    pub async fn poll_once(&self) {
        let instances: Vec<(Uuid, InstanceState)> = self
            .instances
            .read()
            .await
            .iter()
            .map(|(id, state)| (*id, state.clone()))
            .collect();
        self.poll_all(instances).await;
    }

    /// Poll only the instances whose next-due time has arrived.
    ///
    /// This is what makes backoff mean anything: a connector that has been
    /// failing for a while simply is not in this set most of the time.
    pub async fn poll_due(&self) {
        let now = time::Instant::now();

        let due: Vec<(Uuid, InstanceState)> = {
            let instances = self.instances.read().await;
            let mut schedules = self.schedules.write().await;
            instances
                .iter()
                .filter(|(id, _)| {
                    // An instance with no schedule yet has never been polled by
                    // the loop, so it is due on this very tick — hence
                    // `due_at(now)` rather than `due_now()`, which would read a
                    // later clock and defer it by one tick for no reason.
                    let schedule = schedules
                        .entry(**id)
                        .or_insert_with(|| PollSchedule::due_at(now));
                    if schedule.next_due > now {
                        return false;
                    }
                    // Claimed for this pass. The due time only moved when the
                    // attempt *finished* before, so anything outlasting a tick
                    // — a connector timing out, a construction retry waiting on
                    // a dead host — was dispatched again every second until it
                    // came back, piling concurrent connection attempts onto the
                    // one service least able to take them. The outcome
                    // overwrites this with the interval it has earned.
                    schedule.next_due = now + schedule.interval();
                    true
                })
                .map(|(id, state)| (*id, state.clone()))
                .collect()
        };

        self.prune_expired_operations().await;
        if due.is_empty() {
            self.prune_network_failures().await;
            return;
        }
        self.poll_all(due).await;
        self.prune_network_failures().await;
    }

    /// Works the given instances concurrently, one task each.
    ///
    /// What "working" one means depends on what it is: a built connector is
    /// polled, and one that is still pending gets another construction attempt.
    /// Both outcomes land in [`ConnectorRuntime::record_poll_outcome`], so a
    /// pending instance backs off, gets diagnosed, and feeds the shared-outage
    /// advisory on exactly the terms a live-but-Down one does.
    async fn poll_all(&self, instances: Vec<(Uuid, InstanceState)>) {
        let mut polls = JoinSet::new();
        for (id, state) in instances {
            let runtime = self.clone();
            polls.spawn(async move {
                match state {
                    InstanceState::Live(connector) => runtime.poll_connector(id, connector).await,
                    InstanceState::Pending(pending) => {
                        runtime.retry_construction(id, pending).await
                    }
                }
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
        let is_current = matches!(
            self.instances.read().await.get(&id),
            Some(InstanceState::Live(current)) if Arc::ptr_eq(current, &connector)
        );
        if !is_current {
            return;
        }

        self.record_poll_outcome(id, outcome, connector.network_target())
            .await;
    }

    /// Tries once more to build a pending instance.
    ///
    /// Success is the same event as a create or an update completing: the entry
    /// becomes `Live`, the "was not loaded" snapshot is dropped, and the
    /// schedule returns to the base cadence. The instance is then polled
    /// immediately rather than on the next tick, so recovery shows up as a real
    /// reading instead of an empty one.
    ///
    /// Failure is the same event as a poll failing, and goes down the same path
    /// — one backoff curve, one diagnosis debounce, one broadcast — because two
    /// implementations of "keep trying, but less often" is precisely how one of
    /// them ends up never firing.
    async fn retry_construction(&self, id: Uuid, pending: Arc<PendingInstance>) {
        let attempt = self
            .construct(&pending.connector_type, &pending.config)
            .await;

        // Whether this entry is still the one that was retried. An update or a
        // delete landing mid-attempt wins: its result is newer than this one.
        let replaced = |instances: &HashMap<Uuid, InstanceState>| {
            !matches!(
                instances.get(&id),
                Some(InstanceState::Pending(current)) if Arc::ptr_eq(current, &pending)
            )
        };

        match attempt {
            Ok(connector) => {
                {
                    let mut instances = self.instances.write().await;
                    if replaced(&instances) {
                        return;
                    }
                    instances.insert(id, InstanceState::Live(Arc::clone(&connector)));
                }
                self.statuses.write().await.remove(&id);
                self.schedules
                    .write()
                    .await
                    .insert(id, PollSchedule::due_now());
                self.record_network_failure(id, None).await;
                tracing::info!(
                    instance = %id,
                    connector_type = %pending.connector_type,
                    "a pending connector instance was built successfully and is now live"
                );
                self.poll_connector(id, connector).await;
            }
            Err(error) => {
                let reported = not_loaded_error(&error);
                let target = self.pending_network_target(&pending).await;
                {
                    let mut instances = self.instances.write().await;
                    if replaced(&instances) {
                        return;
                    }
                    instances.insert(
                        id,
                        InstanceState::Pending(Arc::new(PendingInstance {
                            connector_type: pending.connector_type.clone(),
                            config: pending.config.clone(),
                            last_error: error,
                        })),
                    );
                }
                self.record_poll_outcome(
                    id,
                    ConnectorStatusSnapshot::from_poll(Some(pending_status()), Some(reported)),
                    target,
                )
                .await;
            }
        }
    }

    /// Where a pending instance *would have* connected, for the diagnosis.
    ///
    /// Asked of the type's **no-I/O** connection-test constructor, which exists
    /// so a connector can be built without contacting anything. Never the
    /// ordinary factory: contacting the service is the thing that just failed,
    /// and doing it twice per retry would double the load on a struggling host.
    /// A type without such a constructor simply gets no network diagnosis,
    /// exactly like a live connector that publishes no target.
    async fn pending_network_target(&self, pending: &PendingInstance) -> Option<NetworkTarget> {
        let factory = self
            .registration(&pending.connector_type)?
            .connection_test_factory?;
        // Bounded anyway. "No I/O" is a property of each registration that this
        // code cannot verify, and a diagnosis must never outlast the failure it
        // is explaining.
        time::timeout(diagnostics::PROBE_TIMEOUT, factory(pending.config.clone()))
            .await
            .ok()?
            .ok()?
            .network_target()
    }

    /// Records what an attempt produced: backoff, diagnosis, advisory, publish.
    ///
    /// The single tail shared by a status poll and a construction retry. Every
    /// decision about *how often to try again* and *what to say about it* is
    /// made here and nowhere else.
    async fn record_poll_outcome(
        &self,
        id: Uuid,
        outcome: ConnectorStatusSnapshot,
        target: Option<NetworkTarget>,
    ) {
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
            self.diagnose_if_due(id, target).await
        } else {
            DiagnosticProbe::Completed(None)
        };

        if let DiagnosticProbe::Completed(result) = &diagnosis {
            self.record_network_failure(
                id,
                result
                    .as_ref()
                    .and_then(diagnostics::NetworkDiagnosis::tcp_unreachable_address),
            )
            .await;
        }

        self.update_snapshot(id, |snapshot| {
            snapshot.status = outcome.status;
            snapshot.status_error = outcome.status_error;
            if failing {
                // An existing diagnosis survives a poll that was not due for a
                // fresh probe; a recovery clears it, because a sentence about
                // why something is unreachable is worse than nothing once it is
                // reachable.
                if let DiagnosticProbe::Completed(result) = diagnosis {
                    snapshot.diagnosis = result.map(|diagnosis| diagnosis.message);
                }
            } else {
                snapshot.diagnosis = None;
            }
        })
        .await;
    }

    /// Probes an instance's network target, unless it was probed recently.
    ///
    /// `Skipped` means the debounce blocked a real probe and existing evidence
    /// must remain intact. `Completed(None)` means there is no target worth
    /// probing and clears any observation left by an older connector config.
    async fn diagnose_if_due(&self, id: Uuid, target: Option<NetworkTarget>) -> DiagnosticProbe {
        let Some(target) = target else {
            return DiagnosticProbe::Completed(None);
        };

        {
            let mut schedules = self.schedules.write().await;
            let schedule = schedules.entry(id).or_insert_with(PollSchedule::due_now);
            let now = time::Instant::now();
            if schedule
                .last_diagnosed_at
                .is_some_and(|last| now.duration_since(last) < DIAGNOSIS_INTERVAL)
            {
                return DiagnosticProbe::Skipped;
            }
            schedule.last_diagnosed_at = Some(now);
        }

        DiagnosticProbe::Completed(diagnostics::diagnose(&target).await)
    }

    async fn record_network_failure(&self, id: Uuid, address: Option<IpAddr>) {
        let advisory = {
            let mut tracker = self.network_failures.write().await;
            tracker.record(id, address, time::Instant::now());
            tracker.advisory()
        };
        self.publish_network_advisory(advisory);
    }

    async fn remove_network_failure(&self, id: &Uuid) {
        let advisory = {
            let mut tracker = self.network_failures.write().await;
            tracker.remove(id, time::Instant::now());
            tracker.advisory()
        };
        self.publish_network_advisory(advisory);
    }

    async fn prune_network_failures(&self) {
        let advisory = {
            let mut tracker = self.network_failures.write().await;
            tracker.prune(time::Instant::now());
            tracker.advisory()
        };
        self.publish_network_advisory(advisory);
    }

    fn publish_network_advisory(&self, advisory: NetworkAdvisory) {
        if *self.network_advisory.borrow() != advisory {
            self.network_advisory.send_replace(advisory);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{LazyLock, Mutex};

    use super::*;
    use crate::connectors::registry::{builtin_registry, ConnectorTypeRegistration};
    use loom_core::connector::debug::{DebugConnector, TYPE_ID as DEBUG_TYPE_ID};
    use serde_json::json;

    /// How many construction attempts each fixture instance has seen, keyed by
    /// the `slot` in its stored configuration.
    ///
    /// Keyed rather than a single counter because a [`ConnectorFactory`] is a
    /// plain `fn` pointer that can capture nothing, so the state has to be
    /// global — and these tests run in one process at the same time. The slot
    /// gives each test its own tally.
    static FLAKY_ATTEMPTS: LazyLock<Mutex<HashMap<String, u64>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    const FLAKY_TYPE_ID: &str = "flaky-test-connector";
    const HANGING_TYPE_ID: &str = "hanging-test-connector";

    /// Configuration for the flaky fixture: fail `failures` times in this slot,
    /// then succeed.
    fn flaky_config(slot: &str, failures: u64) -> Value {
        json!({ "slot": slot, "failures": failures })
    }

    /// A registry of two fixtures that model the two ways construction goes
    /// wrong: a factory that refuses for a while and then works, and one that
    /// never comes back at all.
    fn flaky_registry() -> ConnectorTypeRegistry {
        fn registration(
            type_id: &'static str,
            factory: crate::connectors::registry::ConnectorFactory,
        ) -> ConnectorTypeRegistration {
            ConnectorTypeRegistration {
                type_id,
                display_name: "Test Connector",
                icon: None,
                factory,
                connection_test_factory: None,
                schema: json!({ "type": "object" }),
                setup_guide: None,
                discoverable_type: None,
                discovery_target_field: None,
            }
        }

        let mut types = HashMap::new();
        types.insert(
            FLAKY_TYPE_ID,
            registration(FLAKY_TYPE_ID, |config| {
                Box::pin(async move {
                    let slot = config["slot"].as_str().unwrap_or_default().to_owned();
                    let failures = config["failures"].as_u64().unwrap_or(0);
                    let attempt = {
                        let mut attempts =
                            FLAKY_ATTEMPTS.lock().expect("the tally is not poisoned");
                        let seen = attempts.entry(slot).or_insert(0);
                        *seen += 1;
                        *seen
                    };
                    if attempt <= failures {
                        return Err(ConnectorError::unreachable("the service is not up yet"));
                    }
                    DebugConnector::from_config_value(json!({}))
                        .map(|connector| Box::new(connector) as Box<dyn Connector>)
                })
            }),
        );
        types.insert(
            HANGING_TYPE_ID,
            registration(HANGING_TYPE_ID, |_config| {
                Box::pin(async move {
                    // A service that accepts the socket and then says nothing:
                    // the case a timeout exists for, since it never errors.
                    std::future::pending::<()>().await;
                    unreachable!("the construction timeout fires first")
                })
            }),
        );
        Arc::new(types)
    }

    /// An in-memory database holding just the columns `load` reads.
    async fn instances_pool(rows: &[(Uuid, &str, Value)]) -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:")
            .await
            .expect("an in-memory database");
        sqlx::query(
            "CREATE TABLE connector_instances (id TEXT PRIMARY KEY, connector_type TEXT NOT \
             NULL, config TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("the table is created");

        for (id, connector_type, config) in rows {
            sqlx::query(
                "INSERT INTO connector_instances (id, connector_type, config) VALUES (?, ?, ?)",
            )
            .bind(id.to_string())
            .bind(connector_type)
            .bind(config.to_string())
            .execute(&pool)
            .await
            .expect("the row is inserted");
        }

        pool
    }

    async fn is_pending(runtime: &ConnectorRuntime, id: &Uuid) -> bool {
        matches!(
            runtime.instances.read().await.get(id),
            Some(InstanceState::Pending(_))
        )
    }

    /// Any key at all: these fixtures store nothing sensitive, so decryption is
    /// a pass-through and the value only has to be well-formed.
    fn test_key() -> ConfigEncryptionKey {
        ConfigEncryptionKey::clone_from_slice(&[7u8; 32])
    }

    /// A connector whose polls always fail, for exercising backoff without
    /// waiting on anything real.
    async fn failing_instance(runtime: &ConnectorRuntime) -> Uuid {
        let id = Uuid::new_v4();
        let connector = runtime
            .build(DEBUG_TYPE_ID, json!({ "failMode": "unreachable" }))
            .await
            .expect("the fixture builds");
        runtime
            .instances
            .write()
            .await
            .insert(id, InstanceState::Live(connector));
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
        runtime.poll_once().await;

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
        runtime
            .instances
            .write()
            .await
            .insert(id, InstanceState::Live(healthy));
        runtime.poll_once().await;

        let recovered = schedule_of(&runtime, &id).await;
        assert_eq!(recovered.consecutive_failures, 0);
        assert_eq!(recovered.interval(), CONNECTOR_POLL_INTERVAL);
    }

    #[tokio::test]
    async fn a_scheduled_poll_recovers_from_down_resets_backoff_and_pushes_status() {
        let runtime = ConnectorRuntime::new(builtin_registry());
        let id = failing_instance(&runtime).await;
        runtime.poll_once().await;

        for _ in 0..20 {
            runtime.poll_once().await;
        }
        assert_eq!(
            schedule_of(&runtime, &id).await.interval(),
            CONNECTOR_POLL_MAX_INTERVAL
        );

        let healthy = runtime
            .build(DEBUG_TYPE_ID, json!({}))
            .await
            .expect("the fixture builds");
        runtime
            .instances
            .write()
            .await
            .insert(id, InstanceState::Live(healthy));
        runtime
            .schedules
            .write()
            .await
            .get_mut(&id)
            .expect("schedule")
            .next_due = time::Instant::now();
        let mut updates = runtime.subscribe_statuses();

        runtime.poll_due().await;

        let recovered = runtime.cached_status(&id).await.expect("recovered status");
        assert_eq!(
            recovered.status.as_ref().map(|status| status.health),
            Some(HealthState::Healthy)
        );
        assert_eq!(schedule_of(&runtime, &id).await.consecutive_failures, 0);
        assert_eq!(
            schedule_of(&runtime, &id).await.interval(),
            CONNECTOR_POLL_INTERVAL
        );
        let pushed = updates.recv().await.expect("recovery is pushed");
        assert_eq!(pushed.instance_id, id);
        assert_eq!(
            pushed.snapshot.status.map(|status| status.health),
            Some(HealthState::Healthy)
        );
    }

    #[tokio::test]
    async fn manual_reconnect_bypasses_backoff_and_resets_it_after_success() {
        let runtime = ConnectorRuntime::new(builtin_registry());
        let id = failing_instance(&runtime).await;
        for _ in 0..20 {
            runtime.poll_once().await;
        }
        let due_before = schedule_of(&runtime, &id).await.next_due;

        let healthy = runtime
            .build(DEBUG_TYPE_ID, json!({}))
            .await
            .expect("the fixture builds");
        runtime
            .instances
            .write()
            .await
            .insert(id, InstanceState::Live(healthy));
        let mut updates = runtime.subscribe_statuses();

        let snapshot = runtime.reconnect(id).await.expect("live instance");

        assert_eq!(
            snapshot.status.as_ref().map(|status| status.health),
            Some(HealthState::Healthy)
        );
        let schedule = schedule_of(&runtime, &id).await;
        assert_eq!(schedule.consecutive_failures, 0);
        assert_eq!(schedule.interval(), CONNECTOR_POLL_INTERVAL);
        assert!(
            schedule.next_due < due_before,
            "manual retry bypassed the old due time"
        );
        assert_eq!(
            updates
                .recv()
                .await
                .expect("manual recovery is pushed")
                .snapshot
                .status
                .map(|status| status.health),
            Some(HealthState::Healthy)
        );
    }

    #[test]
    fn network_advisory_requires_three_distinct_recent_hosts() {
        let now = time::Instant::now();
        let mut tracker = NetworkOutageTracker::default();
        let shared: IpAddr = "192.0.2.10".parse().unwrap();
        let second: IpAddr = "192.0.2.11".parse().unwrap();
        let third: IpAddr = "192.0.2.12".parse().unwrap();

        tracker.record(Uuid::new_v4(), Some(shared), now);
        tracker.record(Uuid::new_v4(), Some(shared), now);
        assert_eq!(tracker.advisory().affected_host_count, 1);
        assert!(!tracker.advisory().active);

        tracker.record(Uuid::new_v4(), Some(second), now);
        assert!(!tracker.advisory().active);
        tracker.record(Uuid::new_v4(), Some(third), now);
        assert_eq!(
            tracker.advisory(),
            NetworkAdvisory {
                active: true,
                affected_host_count: 3,
            }
        );
    }

    #[test]
    fn network_advisory_expires_old_failures_and_clears_recovered_instances() {
        let start = time::Instant::now();
        let mut tracker = NetworkOutageTracker::default();
        let ids = [Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4()];
        for (index, id) in ids.into_iter().enumerate() {
            tracker.record(
                id,
                Some(format!("192.0.2.{}", index + 1).parse().unwrap()),
                start,
            );
        }
        assert!(tracker.advisory().active);

        tracker.record(ids[0], None, start + Duration::from_secs(1));
        assert!(!tracker.advisory().active);
        assert_eq!(tracker.advisory().affected_host_count, 2);

        tracker.prune(start + NETWORK_ADVISORY_WINDOW);
        assert_eq!(tracker.advisory(), NetworkAdvisory::default());
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

        // An action brings the next poll forward without making the action's
        // HTTP response wait for remote status I/O.
        runtime.refresh_now(id).await;
        assert_eq!(
            schedule_of(&runtime, &id).await.consecutive_failures,
            1,
            "scheduling a refresh must not itself perform the poll"
        );
        runtime.poll_due().await;
        let after_action = schedule_of(&runtime, &id).await;
        assert_eq!(
            after_action.consecutive_failures, 2,
            "the next poller tick performs the scheduled refresh"
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
        runtime
            .instances
            .write()
            .await
            .insert(id, InstanceState::Live(connector));

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
        runtime
            .instances
            .write()
            .await
            .insert(id, InstanceState::Live(healthy));
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
        runtime
            .instances
            .write()
            .await
            .insert(id, InstanceState::Live(connector));

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
        assert!(
            runtime.cached_status(&id).await.is_none(),
            "insert must not block the request path on an initial status poll"
        );
        runtime.poll_once().await;
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
        runtime.poll_once().await;
        let initial = updates.recv().await.expect("first poll must broadcast");
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
        runtime.poll_once().await;

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

        assert!(
            runtime.cached_status(&id).await.is_none(),
            "replacement clears the stale snapshot until its background poll"
        );
        runtime.poll_once().await;

        let snapshot = runtime.cached_status(&id).await.expect("new snapshot");
        let details = snapshot.status.expect("successful status").details;
        assert_eq!(
            loom_core::connector::details::get_detail(&details, None, "label"),
            Some(&json!("new"))
        );
    }

    /// The bug this whole state exists for: a factory that fails at startup
    /// must leave a retryable instance behind, and the ordinary poller must
    /// recover it with nobody touching anything.
    #[tokio::test]
    async fn a_construction_failure_becomes_a_pending_instance_the_poller_recovers() {
        let id = Uuid::new_v4();
        let pool = instances_pool(&[(id, FLAKY_TYPE_ID, flaky_config("recovers", 1))]).await;
        let runtime = ConnectorRuntime::new(flaky_registry())
            .load_into(&pool, &test_key())
            .await
            .expect("the runtime loads");

        // Not dropped: the row is in the map, reported Down rather than as an
        // absence, and carrying the factory's own objection.
        assert_eq!(runtime.len().await, 1);
        assert!(is_pending(&runtime, &id).await);
        assert!(runtime.get(&id).await.is_none());
        let snapshot = runtime.cached_status(&id).await.expect("a seeded status");
        assert_eq!(
            snapshot.status.as_ref().map(|status| status.health),
            Some(HealthState::Down)
        );
        let reported = snapshot
            .status_error
            .expect("the construction error")
            .to_string();
        assert!(
            reported.contains("was not loaded") && reported.contains("the service is not up yet"),
            "the seeded error should name the real cause: {reported}"
        );
        assert_eq!(runtime.counts().await, (0, 1));

        // Due immediately, so one ordinary poller pass is all recovery takes.
        runtime.poll_due().await;

        assert!(!is_pending(&runtime, &id).await);
        assert!(runtime.get(&id).await.is_some());
        assert_eq!(runtime.counts().await, (1, 0));
        let recovered = runtime.cached_status(&id).await.expect("a polled status");
        assert_eq!(
            recovered.status.as_ref().map(|status| status.health),
            Some(HealthState::Healthy),
            "a promoted instance is polled immediately rather than left empty"
        );
        assert!(recovered.status_error.is_none());
    }

    /// A pending instance that keeps failing must back off exactly as a live
    /// instance that keeps failing does — the same curve, from the same code.
    #[tokio::test]
    async fn a_pending_instance_that_keeps_failing_backs_off_like_a_failing_poll() {
        let id = Uuid::new_v4();
        let pool =
            instances_pool(&[(id, FLAKY_TYPE_ID, flaky_config("always-fails", u64::MAX))]).await;
        let runtime = ConnectorRuntime::new(flaky_registry())
            .load_into(&pool, &test_key())
            .await
            .expect("the runtime loads");

        runtime.poll_due().await;
        let after_one = schedule_of(&runtime, &id).await;
        assert_eq!(after_one.consecutive_failures, 1);
        assert_eq!(after_one.interval(), CONNECTOR_POLL_INTERVAL * 2);

        // Still pending, still due later, still carrying the latest error.
        assert!(is_pending(&runtime, &id).await);
        assert!(after_one.next_due > time::Instant::now());
    }

    /// One factory that never returns must not hold up the others, and must not
    /// hold up the process: `load` is what the HTTP listener waits behind.
    #[tokio::test]
    async fn a_hanging_factory_does_not_block_the_other_instances_from_loading() {
        let hanging = Uuid::new_v4();
        let healthy = Uuid::new_v4();
        let pool = instances_pool(&[
            (hanging, HANGING_TYPE_ID, json!({})),
            (healthy, FLAKY_TYPE_ID, flaky_config("beside-a-hang", 0)),
        ])
        .await;

        let started = time::Instant::now();
        let runtime = ConnectorRuntime::new(flaky_registry())
            .with_construction_timeout(Duration::from_millis(200))
            .load_into(&pool, &test_key())
            .await
            .expect("the runtime loads");
        let elapsed = started.elapsed();

        assert!(
            runtime.get(&healthy).await.is_some(),
            "a reachable connector is built regardless of what the others are doing"
        );
        assert!(is_pending(&runtime, &hanging).await);
        assert!(
            elapsed < Duration::from_secs(2),
            "load must be bounded by the construction timeout, not by the hung factory: {elapsed:?}"
        );

        // The hung one is a pending instance like any other, not a special case.
        let snapshot = runtime
            .cached_status(&hanging)
            .await
            .expect("a seeded status");
        assert!(snapshot
            .status_error
            .expect("the timeout error")
            .to_string()
            .contains("did not finish connecting"));
    }

    /// Pressing Reconnect on an instance that is pending must be a real
    /// attempt, not a refusal — and must succeed the moment the service is up.
    #[tokio::test]
    async fn reconnect_retries_construction_for_a_pending_instance() {
        let id = Uuid::new_v4();
        let pool = instances_pool(&[(id, FLAKY_TYPE_ID, flaky_config("reconnect", 1))]).await;
        let runtime = ConnectorRuntime::new(flaky_registry())
            .load_into(&pool, &test_key())
            .await
            .expect("the runtime loads");
        assert!(is_pending(&runtime, &id).await);

        let snapshot = runtime
            .reconnect(id)
            .await
            .expect("a pending instance answers reconnect rather than refusing it");

        assert_eq!(
            snapshot.status.as_ref().map(|status| status.health),
            Some(HealthState::Healthy)
        );
        assert!(runtime.get(&id).await.is_some());
    }

    /// The route helper behind actions, sub-targets and discovery: a pending
    /// instance is built on demand, and its failure is reported as the
    /// connector's own objection rather than as "not loaded".
    #[tokio::test]
    async fn ensure_live_builds_a_pending_instance_on_demand() {
        let id = Uuid::new_v4();
        let pool = instances_pool(&[(id, FLAKY_TYPE_ID, flaky_config("ensure-live", 2))]).await;
        let runtime = ConnectorRuntime::new(flaky_registry())
            .load_into(&pool, &test_key())
            .await
            .expect("the runtime loads");

        // Second attempt: still failing, and the caller is told why.
        match runtime.ensure_live(&id).await {
            Err(EnsureError::Construction(error)) => {
                assert!(error.to_string().contains("the service is not up yet"));
            }
            Err(EnsureError::Unknown) => panic!("the instance is known, just not built"),
            Ok(_) => panic!("the fixture is configured to fail twice"),
        }

        // Third attempt: the service is up, so the caller simply gets it.
        assert!(runtime.ensure_live(&id).await.is_ok());
        assert!(runtime.get(&id).await.is_some());

        // An id this process holds nothing for stays distinguishable.
        assert!(matches!(
            runtime.ensure_live(&Uuid::new_v4()).await,
            Err(EnsureError::Unknown)
        ));
    }

    /// A row of a type this build does not register is still not retried: there
    /// is nothing to retry with, and the list response says so itself.
    #[tokio::test]
    async fn an_unregistered_type_is_skipped_rather_than_left_pending() {
        let id = Uuid::new_v4();
        let pool = instances_pool(&[(id, "no-such-connector-type", json!({}))]).await;
        let runtime = ConnectorRuntime::new(flaky_registry())
            .load_into(&pool, &test_key())
            .await
            .expect("the runtime loads");

        assert_eq!(runtime.len().await, 0);
        assert!(runtime.cached_status(&id).await.is_none());
    }
}
