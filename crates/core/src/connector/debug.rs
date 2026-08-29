//! A configurable fake connector, kept in the tree on purpose.
//!
//! **This is a permanent development and testing fixture, not example code.**
//! It is not scaffolding to be deleted once "real" connectors exist, and it must
//! not be removed on the grounds that it talks to nothing. Its whole job is to
//! talk to nothing: it lets the clients — web-frontend today, the desktop and
//! mobile apps later — be built and tested against realistic loading, error,
//! and success states without anyone needing live infrastructure. A contributor
//! with a laptop and no homelab can still work on Loom's UI, and Loom's tests
//! can assert on connector behaviour without depending on a service being up.
//!
//! It now carries seven jobs rather than one:
//!
//! 1. **Auth and shell development**, its original purpose — a connector that
//!    is reliably there to be listed, permission-checked, and acted on.
//! 2. **Widget rendering development.** It exposes one data point of every
//!    [`DataPointValueType`] and ships a [`default_layout`](Connector::default_layout)
//!    spreading them across the display widgets *and* wiring its actions to the
//!    control widgets, so both halves of every renderer can be built and
//!    reviewed against something that moves.
//! 3. **Dashboard placement testing.** It declares a `min_size` and a real
//!    layout, which is what a placement UI needs in order to be exercised.
//! 4. **End-to-end tests**, present and future — it is the one connector type
//!    that behaves identically on every machine.
//! 5. **Discovery, setup-guide, and capability-check reference behaviour.**
//!    Its self-referential discovery yields more valid debug configurations,
//!    its two guide variants exercise static and toggle-driven setup paths,
//!    and its live connection test reports both unconditional and conditional
//!    capabilities, so every layer can be
//!    exercised before a real integration exists.
//! 6. **Resource-browser reference behaviour.** It publishes two fake
//!    browsable kinds — `widgets` and `gadgets` — whose columns cover every
//!    [`ColumnValueType`] and whose actions cover both scopes (one row action
//!    needing a `resourceId`, one kind action needing nothing), so a table
//!    renderer and the endpoints behind it can be built and tested before any
//!    real connector browses anything.
//! 7. **Audit-log and update-check reference behaviour.** Its `recalibrate`
//!    action is both disruptive *and* snapshot-bearing, so the platform's
//!    pre-action snapshot can be proven end to end against a reading the
//!    action then destroys, and its update check answers whatever
//!    [`DebugConnectorConfig::simulated_update_available`] says, so both a
//!    clean instance and an out-of-date one are renderable.
//!
//! Everything interesting is set at construction through
//! [`DebugConnectorConfig`]:
//!
//! - [`DebugConnectorConfig::simulated_status`] — the reading `status()` bases
//!   its answer on, so every [`HealthState`](super::HealthState) can be
//!   rendered and reviewed.
//! - [`DebugConnectorConfig::simulated_latency_ms`] — an artificial delay before
//!   every response, so spinners and skeletons are exercised instead of being
//!   skipped by an instant local answer.
//! - [`DebugConnectorConfig::fail_mode`] — a stored [`ConnectorError`] returned
//!   in place of success, so error paths are reachable on demand.
//! - [`DebugConnectorConfig::base_load`], [`DebugConnectorConfig::label`] and
//!   [`DebugConnectorConfig::enabled`] — the starting values of the simulated
//!   data points.
//!
//! ```
//! use loom_core::connector::debug::{DebugConnector, DebugConnectorConfig};
//! use loom_core::connector::{Connector, ConnectorError};
//!
//! # async fn example() {
//! let flaky = DebugConnector::new(DebugConnectorConfig {
//!     simulated_latency_ms: 250,
//!     fail_mode: Some(ConnectorError::unreachable("simulated outage")),
//!     ..DebugConnectorConfig::default()
//! });
//! assert!(flaky.status().await.is_err());
//! # }
//! ```

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{
    details::set_detail, ActionResult, ActionWidgetType, CapabilityRequirement, CapabilityStatus,
    ChartType, ColumnDescriptor, ColumnValueType, ConnectionTestResult, Connector, ConnectorAction,
    ConnectorError, ConnectorMetadata, ConnectorStatus, DataPointDescriptor, DataPointValueType,
    DiscoveredResource, DisplayField, DisplayWidgetType, HealthState, NetworkTarget, ResourceItem,
    ResourceKindDescriptor, SetupGuide, SetupGuideToggle, SetupGuideVariant, SubTarget,
    UpdateCheckResult, WidgetBinding, WidgetLayout,
};

/// The connector type id this fixture registers under.
pub const TYPE_ID: &str = "debug";

/// The action id for the simulated restart. Parameterless.
pub const ACTION_RESTART: &str = "restart";

/// The action id for the simulated reachability check. Parameterless.
pub const ACTION_PING: &str = "ping";

/// The action id that flips the simulated on/off state. Takes `{"enabled": bool}`.
pub const ACTION_SET_ENABLED: &str = "set-enabled";

/// The action id that moves the simulated load. Takes `{"value": number}`.
pub const ACTION_SET_LOAD: &str = "set-load";

/// The action id that rewrites the simulated label. Takes `{"label": string}`.
pub const ACTION_SET_LABEL: &str = "set-label";

/// The resource kind id for the fake browsable "widgets" table.
pub const RESOURCE_KIND_WIDGETS: &str = "widgets";

/// The resource kind id for the fake browsable "gadgets" table.
pub const RESOURCE_KIND_GADGETS: &str = "gadgets";

/// The row-scoped resource action. Takes `{"resourceId": string}`.
pub const ACTION_RECYCLE: &str = "recycle";

/// The kind-scoped resource action. Parameterless.
pub const ACTION_CLEANUP_ALL: &str = "cleanupAll";

/// The action id for the simulated recalibration. Parameterless, disruptive,
/// and the fixture's demonstration of the pre-action snapshot: it overwrites
/// [`DATA_POINT_LOAD`], having first declared that data point as worth
/// recording.
pub const ACTION_RECALIBRATE: &str = "recalibrate";

/// The data point id for the oscillating numeric reading.
pub const DATA_POINT_LOAD: &str = "load";

/// The data point id for the simulated version/label text.
pub const DATA_POINT_LABEL: &str = "label";

/// The data point id for the simulated on/off flag.
pub const DATA_POINT_ENABLED: &str = "enabled";

/// The data point id for the rolling buffer of recent load readings.
pub const DATA_POINT_LOAD_HISTORY: &str = "loadHistory";

/// How many readings [`DATA_POINT_LOAD_HISTORY`] keeps.
///
/// Bounded because this buffer lives in memory for the process's whole life and
/// is serialized into every status response; an unbounded history would grow a
/// response body without limit for a fixture nobody is monitoring.
pub const HISTORY_CAPACITY: usize = 50;

/// The data point id for the rolling buffer of fake log lines.
pub const DATA_POINT_LOG: &str = "log";

/// How many lines [`DATA_POINT_LOG`] keeps.
///
/// Small on purpose. This one is not a chart series but a scrolling text pane,
/// and the fixture's job is to give `LogStream` something to scroll — not to
/// simulate log retention, which is a real connector's problem.
pub const LOG_CAPACITY: usize = 10;

/// Stable fake targets used to exercise target-aware clients without a service.
pub const FIXTURE_TARGETS: [&str; 2] = ["fixture-a", "fixture-b"];

/// The fake hostname shown in [`Connector::display_fields`].
///
/// `.invalid` is reserved by RFC 2606 and never resolves, so this cannot be
/// mistaken for — or accidentally become — a real address.
const FAKE_HOST: &str = "debug.invalid";

/// The load [`ACTION_RECALIBRATE`] resets to. A fixed value, so a test can
/// assert that the reading really was overwritten and that the snapshot on the
/// log entry is therefore the only copy of the old one.
const RECALIBRATED_LOAD: f64 = 50.0;

/// The fake "latest version" the simulated update check reports. Obviously not
/// a real registry reference.
const SIMULATED_LATEST_REF: &str = "debug-fixture:2.0.0";

/// How a [`DebugConnector`] should behave.
///
/// A plain struct rather than a builder: every field is independent and has a
/// sensible default, so `..DebugConnectorConfig::default()` in a struct literal
/// is already the ergonomic form and a builder would only add indirection.
/// The default is the boring happy path — healthy, instant, never failing —
/// which is what most tests want.
#[derive(Debug, Clone, PartialEq)]
pub struct DebugConnectorConfig {
    /// The status [`Connector::status`] bases its answer on. Defaults to
    /// [`ConnectorStatus::healthy`]; set it to a degraded or down reading to
    /// see how a client renders one.
    ///
    /// The health and any details are carried through; the simulated data point
    /// values are merged in on top, and the timestamp is restamped, so a poll
    /// reports when it actually happened.
    pub simulated_status: ConnectorStatus,

    /// Milliseconds to wait before answering any call. Defaults to `0`.
    ///
    /// The point is to make asynchronicity visible: with a real service the
    /// network supplies the delay, and without one a loading state can appear
    /// to work while actually never being rendered.
    pub simulated_latency_ms: u64,

    /// When `Some`, every call fails with this error instead of succeeding.
    /// Defaults to `None`.
    ///
    /// The error is cloned out on each call rather than consumed, so one
    /// configured connector can be asked repeatedly and behave the same way —
    /// which is why [`ConnectorError`] is `Clone`.
    pub fail_mode: Option<ConnectorError>,

    /// The centre the simulated load oscillates around, in percent.
    /// Defaults to `42.0`. Must be within `0.0..=100.0`.
    pub base_load: f64,

    /// The starting value of the simulated text data point.
    pub label: String,

    /// The starting value of the simulated boolean data point.
    pub enabled: bool,

    /// What [`Connector::network_target`] should report. Defaults to `None`.
    ///
    /// The fixture reaches nothing, so it has no real endpoint — which is
    /// precisely why this is configurable. The platform's network diagnostic
    /// (DNS lookup, then TCP connect) has three outcomes a client must render,
    /// and reaching all three otherwise needs a real host that is broken in a
    /// specific way. Point this at a name that does not resolve, or at an
    /// address with nothing listening, and the diagnostic under test produces
    /// the matching diagnosis on any machine.
    ///
    /// Same job as [`DebugConnectorConfig::fail_mode`], one layer further out:
    /// `fail_mode` makes the connector fail, this makes the *network under* it
    /// fail.
    pub network_target: Option<NetworkTarget>,

    /// What [`Connector::check_for_updates`] should pretend to find. Defaults
    /// to `false`.
    ///
    /// The fixture manages nothing and therefore has no registry to compare
    /// against, so both answers have to be configurable for either to be
    /// testable. A client's "update available" badge and its absence are
    /// equally worth being able to render on a machine with no homelab.
    pub simulated_update_available: bool,
}

impl Default for DebugConnectorConfig {
    fn default() -> Self {
        Self {
            simulated_status: ConnectorStatus::healthy(),
            simulated_latency_ms: 0,
            fail_mode: None,
            base_load: 42.0,
            label: "debug-fixture".to_string(),
            enabled: true,
            network_target: None,
            simulated_update_available: false,
        }
    }
}

/// The part of the fixture that moves.
///
/// Behind a `Mutex` because [`Connector::status`] takes `&self` — a connector
/// is shared, immutably, across every request task that touches it — and a
/// data point that never changes gives a chart nothing to draw. The critical
/// sections are a handful of arithmetic operations with no `await` inside them.
/// One entry in the [`DATA_POINT_LOAD_HISTORY`] buffer.
///
/// Serialized straight into `status().details` as the `{ "timestamp", "value" }`
/// object the [`DataPointValueType::TimeSeries`] contract requires
/// ([`ConnectorStatus::details`]); the field names are the wire names, so this
/// struct is the shape rather than a source for one.
#[derive(Debug, Clone, Serialize)]
struct HistorySample {
    /// When the reading was taken, RFC 3339 on the wire.
    timestamp: DateTime<Utc>,
    /// The reading itself, rounded the same way the scalar data point is.
    value: f64,
}

#[derive(Debug)]
struct SimulatedState {
    /// Number of `status()` calls so far; drives the oscillation.
    tick: u64,
    /// The most recent load reading.
    load: f64,
    /// The last [`HISTORY_CAPACITY`] readings, oldest first.
    history: VecDeque<HistorySample>,
    /// Current value of the boolean data point.
    enabled: bool,
    /// Current value of the text data point.
    label: String,
    /// The last [`LOG_CAPACITY`] fake log lines, oldest first.
    log: VecDeque<String>,
}

impl SimulatedState {
    /// Appends a reading to the bounded history, evicting the oldest first.
    ///
    /// One place rather than two, because both `status()` and the `set-load`
    /// action push into this buffer and a capacity check that only one of them
    /// performed would let the fixture grow without limit.
    fn record(&mut self, value: f64) {
        if self.history.len() == HISTORY_CAPACITY {
            self.history.pop_front();
        }
        self.history.push_back(HistorySample {
            timestamp: Utc::now(),
            value: round2(value),
        });
    }

    /// Appends one fake log line, evicting the oldest past [`LOG_CAPACITY`].
    ///
    /// The content is deliberately about the fixture itself — its tick counter
    /// and its own simulated numbers. Nothing here imitates the log format of
    /// any real service, because a plausible-looking line from a service Loom
    /// does not have is a line someone will eventually try to debug.
    fn append_log(&mut self, value: f64) {
        let level = match self.tick % 4 {
            0 => "INFO",
            1 => "DEBUG",
            2 => "INFO",
            _ => "WARN",
        };
        let line = format!(
            "{} {level:<5} simulated tick {} — load {:.1}%, {}",
            Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ"),
            self.tick,
            value,
            if self.enabled { "enabled" } else { "disabled" }
        );

        if self.log.len() == LOG_CAPACITY {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }
}

/// A [`Connector`] that simulates a service instead of contacting one.
///
/// See the [module documentation](self) for why this exists permanently. Cheap
/// to construct and to clone, holds no resources, and starts no tasks. Cloning
/// shares the simulated state, so two clones of one fixture agree about what
/// they are pretending to be.
#[derive(Debug, Clone)]
pub struct DebugConnector {
    config: DebugConnectorConfig,
    state: Arc<Mutex<SimulatedState>>,
}

impl Default for DebugConnector {
    fn default() -> Self {
        Self::new(DebugConnectorConfig::default())
    }
}

impl DebugConnector {
    /// Builds a fixture with the given behaviour.
    pub fn new(config: DebugConnectorConfig) -> Self {
        let state = SimulatedState {
            tick: 0,
            load: config.base_load,
            history: VecDeque::with_capacity(HISTORY_CAPACITY),
            log: VecDeque::with_capacity(LOG_CAPACITY),
            enabled: config.enabled,
            label: config.label.clone(),
        };

        Self {
            config,
            state: Arc::new(Mutex::new(state)),
        }
    }

    /// Builds a fixture that always fails with `error`.
    ///
    /// The shorthand for the most common non-default case: pointing a client at
    /// something broken to check its error handling.
    pub fn failing(error: ConnectorError) -> Self {
        Self::new(DebugConnectorConfig {
            fail_mode: Some(error),
            ..DebugConnectorConfig::default()
        })
    }

    /// Builds a healthy fixture that takes `ms` milliseconds to answer, for
    /// exercising loading states.
    pub fn with_latency(ms: u64) -> Self {
        Self::new(DebugConnectorConfig {
            simulated_latency_ms: ms,
            ..DebugConnectorConfig::default()
        })
    }

    /// Read-only view of the configured behaviour, so tests can assert against
    /// what they asked for without keeping a second copy.
    pub fn config(&self) -> &DebugConnectorConfig {
        &self.config
    }

    /// Builds a fixture from a stored JSON configuration.
    ///
    /// This is the entry point the backend's connector-type registry calls: a
    /// stored instance is a type id plus an opaque JSON blob, and turning that
    /// blob into a live connector is the connector's own job. Validation lives
    /// here, next to [`Connector::config_schema`], rather than in the backend —
    /// the backend has no way to know what `baseLoad` means, and a shape check
    /// against the published schema would not catch an out-of-range value
    /// anyway.
    ///
    /// `null` and `{}` both mean "no configuration", which is what an
    /// "add connector" form submits before anything is filled in. Unknown keys
    /// are rejected rather than ignored, so a typo in a field name is reported
    /// instead of silently doing nothing.
    pub fn from_config_value(config: Value) -> Result<Self, ConnectorError> {
        let raw: RawConfig = match config {
            Value::Null => RawConfig::default(),
            other => serde_json::from_value(other)
                .map_err(|error| ConnectorError::invalid_config(error.to_string()))?,
        };

        let base_load = raw.base_load.unwrap_or(42.0);
        if !(0.0..=100.0).contains(&base_load) {
            return Err(ConnectorError::invalid_config(format!(
                "baseLoad must be between 0 and 100, got {base_load}"
            )));
        }

        let label = raw.label.unwrap_or_else(|| "debug-fixture".to_string());
        if label.trim().is_empty() {
            return Err(ConnectorError::invalid_config("label must not be empty"));
        }

        Ok(Self::new(DebugConnectorConfig {
            simulated_status: ConnectorStatus::new(
                raw.simulated_health.unwrap_or(HealthState::Healthy),
                Value::Object(Map::new()),
            ),
            simulated_latency_ms: raw.simulated_latency_ms,
            fail_mode: raw.fail_mode.map(FailModeConfig::into_error),
            base_load,
            label,
            enabled: raw.enabled.unwrap_or(true),
            network_target: raw.network_target,
            simulated_update_available: raw.simulated_update_available,
        }))
    }

    /// Applies the configured artificial delay.
    ///
    /// `tokio::time::sleep` is the only reason core depends on tokio: it needs
    /// a timer driven by the caller's existing runtime. Nothing here spawns a
    /// task or builds a runtime, so the library stays passive.
    async fn simulate_latency(&self) {
        if self.config.simulated_latency_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(
                self.config.simulated_latency_ms,
            ))
            .await;
        }
    }

    /// Delays, then short-circuits with the configured failure if there is one.
    async fn gate(&self) -> Result<(), ConnectorError> {
        self.simulate_latency().await;
        match &self.config.fail_mode {
            Some(error) => Err(error.clone()),
            None => Ok(()),
        }
    }

    /// Advances the simulation by one tick and returns the current values,
    /// already shaped as the `details` payload keyed by data point id.
    fn advance(&self) -> Value {
        let mut state = self.lock();

        state.tick = state.tick.wrapping_add(1);
        let load = simulated_load(self.config.base_load, state.tick);
        state.load = load;

        state.record(load);
        state.append_log(load);

        // One entry per data point, keyed by its own id and shaped by its
        // declared value type — the `ConnectorStatus::details` contract, which
        // this fixture exists partly to demonstrate.
        json!({
            DATA_POINT_LOAD: round2(state.load),
            DATA_POINT_LABEL: state.label,
            DATA_POINT_ENABLED: state.enabled,
            DATA_POINT_LOAD_HISTORY: state.history.iter().collect::<Vec<&HistorySample>>(),
            // Newline-joined rather than an array: the data point's declared
            // value type is `String`, and `LogStream` is the widget that splits
            // it back into lines. A JSON array here would be a second, undeclared
            // shape for the same value type.
            DATA_POINT_LOG: state
                .log
                .iter()
                .cloned()
                .collect::<Vec<String>>()
                .join("\n"),
        })
    }

    /// Takes the state lock, recovering from a poisoned one.
    ///
    /// A panic inside one of these critical sections would leave nothing worse
    /// than a stale simulated number, and a fixture that refuses to answer
    /// afterwards would take the development server down with it. So the guard
    /// is recovered rather than propagated.
    fn lock(&self) -> std::sync::MutexGuard<'_, SimulatedState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The stored-configuration shape, matching [`Connector::config_schema`].
///
/// `deny_unknown_fields` is what turns a mistyped key into a 400 instead of a
/// setting that silently does nothing.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawConfig {
    #[serde(default)]
    simulated_latency_ms: u64,
    #[serde(default)]
    simulated_health: Option<HealthState>,
    #[serde(default)]
    fail_mode: Option<FailModeConfig>,
    #[serde(default)]
    base_load: Option<f64>,
    #[serde(default)]
    label: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    network_target: Option<NetworkTarget>,
    #[serde(default)]
    simulated_update_available: bool,
}

/// Which failure the fixture should simulate, as it appears in stored config.
///
/// A closed set rather than a free-form [`ConnectorError`]: the configuration
/// is written by a human in a form, and the point is to reach each error path,
/// not to compose arbitrary error values.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
enum FailModeConfig {
    /// Simulates a service that cannot be contacted.
    Unreachable,
    /// Simulates a service that rejects Loom's stored credentials.
    AuthFailed,
    /// Simulates a bug inside the connector.
    Internal,
}

impl FailModeConfig {
    fn into_error(self) -> ConnectorError {
        match self {
            Self::Unreachable => ConnectorError::unreachable("simulated outage"),
            Self::AuthFailed => ConnectorError::AuthFailed {
                reason: "simulated credential rejection".to_string(),
            },
            Self::Internal => ConnectorError::Internal("simulated internal failure".to_string()),
        }
    }
}

/// The load reading for a given tick.
///
/// Deterministic — a sine wave plus a wrapping-LCG wobble seeded from the tick
/// itself. Deliberately not `rand`: core would gain a dependency purely to make
/// a fixture unreproducible, and a test that cannot predict the fixture's
/// output cannot assert on it. The wobble exists so a line chart shows
/// something that looks like telemetry rather than a perfect curve.
fn simulated_load(base: f64, tick: u64) -> f64 {
    let wave = (tick as f64 * 0.35).sin() * 12.0;

    let scrambled = tick
        .wrapping_mul(6_364_136_223_846_793_005)
        .wrapping_add(1_442_695_040_888_963_407);
    let unit = ((scrambled >> 33) as f64) / ((1u64 << 31) as f64);
    let wobble = (unit - 0.5) * 4.0;

    (base + wave + wobble).clamp(0.0, 100.0)
}

/// Rounds to two decimals so the wire values stay readable.
fn round2(value: f64) -> f64 {
    (value * 100.0).round() / 100.0
}

/// Reads a required boolean parameter, or explains why it could not.
fn bool_param(action_id: &str, params: &Value, field: &str) -> Result<bool, ConnectorError> {
    params
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| ConnectorError::InvalidParams {
            action_id: action_id.to_string(),
            reason: format!("expected a boolean `{field}`"),
        })
}

fn debug_capabilities(reachable: bool, actions_enabled: bool) -> Vec<CapabilityStatus> {
    vec![
        CapabilityStatus {
            key: "read-status".to_owned(),
            label: "Read status".to_owned(),
            available: reachable,
            note: (!reachable).then(|| "Requires a reachable connector.".to_owned()),
        },
        CapabilityStatus {
            key: "view-widgets".to_owned(),
            label: "View widgets".to_owned(),
            available: reachable,
            note: (!reachable).then(|| "Requires a reachable connector.".to_owned()),
        },
        CapabilityStatus {
            key: "perform-actions".to_owned(),
            label: "Perform actions".to_owned(),
            available: reachable && actions_enabled,
            note: if !reachable {
                Some("Requires a reachable connector.".to_owned())
            } else if !actions_enabled {
                Some("Unavailable while the debug fixture's enabled flag is off.".to_owned())
            } else {
                None
            },
        },
    ]
}

#[async_trait]
impl Connector for DebugConnector {
    /// Advances the simulation and reports the configured health alongside the
    /// current data point values.
    ///
    /// The values are merged **into** the configured `details` rather than
    /// replacing them, so a test that configures a details payload still sees
    /// it, and `last_checked` is restamped so a polled reading is honest about
    /// its own age.
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
        self.gate().await?;

        let mut details = Value::Object(Map::new());
        if let Value::Object(configured) = self.config.simulated_status.details.clone() {
            for (id, value) in configured {
                set_detail(&mut details, None, &id, value);
            }
        }
        let simulated = self.advance();
        if let Value::Object(values) = &simulated {
            for (id, value) in values {
                set_detail(&mut details, None, id, value.clone());
            }
        }

        // Two fake addressable views deliberately expose different point sets.
        // Besides exercising nested status, this lets backend tests prove that
        // a placement cannot bind a point belonging to another target.
        for (index, target) in FIXTURE_TARGETS.iter().enumerate() {
            let load = simulated
                .get(DATA_POINT_LOAD)
                .and_then(Value::as_f64)
                .unwrap_or_default();
            if index == 0 {
                set_detail(&mut details, Some(target), DATA_POINT_LOAD, json!(load));
            } else {
                set_detail(
                    &mut details,
                    Some(target),
                    DATA_POINT_LABEL,
                    json!(self.lock().label.clone()),
                );
            }
            set_detail(
                &mut details,
                Some(target),
                DATA_POINT_ENABLED,
                json!(index == 0),
            );
        }

        Ok(ConnectorStatus::new(
            self.config.simulated_status.health,
            details,
        ))
    }

    async fn test_connection(&self) -> ConnectionTestResult {
        if let Err(error) = self.gate().await {
            return ConnectionTestResult {
                reachable: false,
                capabilities: debug_capabilities(false, false),
                message: Some(error.to_string()),
            };
        }

        let reachable = matches!(
            self.config.simulated_status.health,
            HealthState::Healthy | HealthState::Degraded
        );
        ConnectionTestResult {
            reachable,
            capabilities: debug_capabilities(reachable, self.config.enabled),
            message: (!reachable).then(|| {
                format!(
                    "The debug fixture reports {:?} health.",
                    self.config.simulated_status.health
                )
            }),
        }
    }

    /// Returns the canned actions, or an empty list in fail mode.
    ///
    /// [`Connector::actions`] has no error channel, and the honest translation
    /// of "this connector is currently broken" into a `Vec` is "it can do
    /// nothing right now" — which is also exactly what a client should render
    /// for an unreachable service. Reporting actions that would immediately
    /// fail, or panicking, would both be worse.
    ///
    /// The five cover the interaction shapes a client has to render: two
    /// parameterless buttons, a boolean toggle, a numeric slider, and a text
    /// field.
    async fn actions(&self) -> Vec<ConnectorAction> {
        if self.gate().await.is_err() {
            return Vec::new();
        }

        vec![
            // Disruptive, so the fixture can exercise the "Performing: …"
            // overlay without a real service having to be taken away. The
            // whole point of this connector is that every state a client has
            // to render is reachable from a laptop with no homelab.
            ConnectorAction::simple(ACTION_RESTART, "Restart")
                .with_description("Pretends to restart the simulated service.")
                .disruptive(),
            ConnectorAction::simple(ACTION_PING, "Ping")
                .with_description("Pretends to check that the simulated service answers."),
            // The fixture's proof of the audit log's snapshot mechanism, and
            // the reason it overwrites a reading rather than nudging it: a
            // snapshot is only worth anything when the value it recorded can
            // no longer be recovered from the connector afterwards.
            ConnectorAction::simple(ACTION_RECALIBRATE, "Recalibrate")
                .with_description(
                    "Pretends to recalibrate the simulated service, resetting its load.",
                )
                .disruptive()
                .snapshotting([DATA_POINT_LOAD]),
            ConnectorAction {
                id: ACTION_SET_ENABLED.to_string(),
                target_id: None,
                label: "Enabled".to_string(),
                description: Some("Flips the simulated on/off state.".to_string()),
                params_schema: json!({
                    "type": "object",
                    "properties": {
                        "enabled": { "type": "boolean" }
                    },
                    "required": ["enabled"],
                    "additionalProperties": false
                }),
                is_disruptive: false,
                snapshot_data_point_ids: Vec::new(),
            },
            ConnectorAction {
                id: ACTION_SET_LOAD.to_string(),
                target_id: None,
                label: "Load".to_string(),
                description: Some(
                    "Moves the centre the simulated load oscillates around.".to_string(),
                ),
                params_schema: json!({
                    "type": "object",
                    "properties": {
                        "value": { "type": "number", "minimum": 0, "maximum": 100 }
                    },
                    "required": ["value"],
                    "additionalProperties": false
                }),
                is_disruptive: false,
                snapshot_data_point_ids: Vec::new(),
            },
            ConnectorAction {
                id: ACTION_SET_LABEL.to_string(),
                target_id: None,
                label: "Label".to_string(),
                description: Some("Rewrites the simulated label text.".to_string()),
                params_schema: json!({
                    "type": "object",
                    "properties": {
                        "label": { "type": "string", "minLength": 1 }
                    },
                    "required": ["label"],
                    "additionalProperties": false
                }),
                is_disruptive: false,
                snapshot_data_point_ids: Vec::new(),
            },
        ]
        .into_iter()
        .chain(FIXTURE_TARGETS.into_iter().flat_map(|target| {
            [
                ConnectorAction::simple(ACTION_PING, "Ping fixture").for_target(target),
                ConnectorAction::simple(ACTION_RESTART, "Restart fixture")
                    .with_description("Pretends to restart this simulated sub-target.")
                    .disruptive()
                    .for_target(target),
            ]
        }))
        .collect()
    }

    async fn execute_action(
        &self,
        action_id: &str,
        _target_id: Option<&str>,
        params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        self.gate().await?;

        match action_id {
            ACTION_RESTART => Ok(ActionResult::ok("Simulated service restarted.")
                .with_payload(json!({ "restarted": true, "params": params }))),

            ACTION_PING => Ok(ActionResult::ok("Simulated service answered the ping.")
                .with_payload(json!({ "pong": true }))),

            ACTION_RECALIBRATE => {
                // Deliberately destructive of the previous reading: the
                // snapshot on the log entry is then the only remaining record
                // of what the load was, which is exactly the property a
                // rollback would depend on.
                let previous = {
                    let mut state = self.lock();
                    let previous = state.load;
                    state.load = RECALIBRATED_LOAD;
                    state.record(RECALIBRATED_LOAD);
                    previous
                };

                Ok(
                    ActionResult::ok("Simulated service recalibrated.").with_payload(json!({
                        "previousLoad": round2(previous),
                        DATA_POINT_LOAD: RECALIBRATED_LOAD,
                    })),
                )
            }

            ACTION_SET_ENABLED => {
                let enabled = bool_param(action_id, &params, "enabled")?;
                self.lock().enabled = enabled;
                Ok(ActionResult::ok(if enabled {
                    "Simulated service enabled."
                } else {
                    "Simulated service disabled."
                })
                .with_payload(json!({ DATA_POINT_ENABLED: enabled })))
            }

            ACTION_SET_LOAD => {
                let value = params.get("value").and_then(Value::as_f64).ok_or_else(|| {
                    ConnectorError::InvalidParams {
                        action_id: action_id.to_string(),
                        reason: "expected a number `value`".to_string(),
                    }
                })?;
                if !(0.0..=100.0).contains(&value) {
                    return Err(ConnectorError::InvalidParams {
                        action_id: action_id.to_string(),
                        reason: format!("`value` must be between 0 and 100, got {value}"),
                    });
                }

                {
                    let mut state = self.lock();
                    state.load = value;
                    state.record(value);
                }

                Ok(ActionResult::ok(format!("Simulated load set to {value}."))
                    .with_payload(json!({ DATA_POINT_LOAD: round2(value) })))
            }

            ACTION_SET_LABEL => {
                let label = params
                    .get("label")
                    .and_then(Value::as_str)
                    .filter(|label| !label.trim().is_empty())
                    .ok_or_else(|| ConnectorError::InvalidParams {
                        action_id: action_id.to_string(),
                        reason: "expected a non-empty string `label`".to_string(),
                    })?
                    .to_string();

                self.lock().label = label.clone();

                Ok(ActionResult::ok("Simulated label updated.")
                    .with_payload(json!({ DATA_POINT_LABEL: label })))
            }

            ACTION_RECYCLE => {
                // A row action without a row is a client bug, not a resource
                // that happens to be missing: refusing it here is what keeps
                // the `resourceId` convention enforceable rather than
                // advisory.
                let resource_id = params
                    .get("resourceId")
                    .and_then(Value::as_str)
                    .filter(|id| !id.trim().is_empty())
                    .ok_or_else(|| ConnectorError::InvalidParams {
                        action_id: action_id.to_string(),
                        reason: "expected a non-empty string `resourceId`".to_string(),
                    })?
                    .to_string();

                Ok(
                    ActionResult::ok(format!("Simulated resource {resource_id} recycled."))
                        .with_payload(json!({ "recycled": resource_id })),
                )
            }

            ACTION_CLEANUP_ALL => Ok(ActionResult::ok("Simulated resources cleaned up.")
                .with_payload(json!({ "cleaned": 3 }))),

            unknown => Err(ConnectorError::invalid_action(unknown)),
        }
    }

    fn config_schema(&self) -> Value {
        json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": "Debug connector configuration",
            "description": "The debug connector contacts nothing, so its configuration only \
                            controls how it pretends to behave.",
            "type": "object",
            "properties": {
                "simulatedLatencyMs": {
                    "type": "integer",
                    "minimum": 0,
                    "default": 0,
                    "description": "Artificial delay before each response, in milliseconds."
                },
                "simulatedHealth": {
                    "type": "string",
                    "enum": ["healthy", "degraded", "down", "unknown"],
                    "default": "healthy",
                    "description": "The health state the fixture reports."
                },
                "failMode": {
                    "type": "string",
                    "enum": ["unreachable", "authFailed", "internal"],
                    "description": "Makes every call fail with this error instead of succeeding."
                },
                "baseLoad": {
                    "type": "number",
                    "minimum": 0,
                    "maximum": 100,
                    "default": 42,
                    "description": "The centre the simulated load oscillates around, in percent."
                },
                "label": {
                    "type": "string",
                    "minLength": 1,
                    "default": "debug-fixture",
                    "description": "Starting value of the simulated label data point."
                },
                "enabled": {
                    "type": "boolean",
                    "default": true,
                    "description": "Starting value of the simulated on/off data point."
                },
                "networkTarget": {
                    "type": "object",
                    "properties": {
                        "host": { "type": "string", "minLength": 1 },
                        "port": { "type": "integer", "minimum": 1, "maximum": 65535 }
                    },
                    "required": ["host"],
                    "additionalProperties": false,
                    "description": "Endpoint the platform's network diagnostic should probe when \
                                    this fixture reports Down. The fixture contacts nothing, so \
                                    this exists to make each diagnosis reachable: a name that does \
                                    not resolve, or an address with nothing listening."
                },
                "simulatedUpdateAvailable": {
                    "type": "boolean",
                    "default": false,
                    "description": "What the fixture's update check should report. The fixture \
                                    has no registry to compare against, so both answers are \
                                    configurable and both are therefore renderable."
                }
            },
            "additionalProperties": false
        })
    }

    fn supports_update_checking(&self) -> bool {
        true
    }

    /// Reports whatever the configuration asked it to report.
    ///
    /// Gated on latency and fail mode like every other entry point, so an
    /// update check has the same loading and failure states to render as
    /// anything else — a check against a real registry is a network call, and
    /// a fixture that always answered instantly and successfully would let a
    /// client ship without handling either.
    async fn check_for_updates(
        &self,
        _target_id: Option<&str>,
    ) -> Result<UpdateCheckResult, ConnectorError> {
        self.gate().await?;

        Ok(if self.config.simulated_update_available {
            UpdateCheckResult::available(SIMULATED_LATEST_REF)
        } else {
            UpdateCheckResult::up_to_date()
        })
    }

    fn discoverable_type(&self) -> Option<String> {
        Some(TYPE_ID.to_owned())
    }

    fn supports_sub_targets(&self) -> bool {
        true
    }

    async fn list_sub_targets(&self) -> Result<Vec<SubTarget>, ConnectorError> {
        self.gate().await?;
        Ok(FIXTURE_TARGETS
            .into_iter()
            .map(|id| SubTarget::new(id, id))
            .collect())
    }

    /// Two fake browsable kinds, covering every [`ColumnValueType`] and both
    /// action scopes.
    ///
    /// `widgets` carries a row action and a kind action; `gadgets` carries only
    /// a row action, so a client cannot get away with assuming every kind has
    /// both. Between them the columns exercise all five cell formats, which is
    /// what a table renderer has to be built against.
    ///
    /// Unconditional, including in fail mode: this is a descriptor method like
    /// [`Connector::data_points`], and what a connector *can* browse does not
    /// stop being true because the service is unreachable. The listing is where
    /// the failure shows up.
    ///
    /// Unconditional in `target_id` too. This connector's fixture targets are
    /// all the same sort of thing, so it has nothing to vary — which is the
    /// ordinary case, and the one the argument must stay cheap for.
    fn resource_kinds(&self, _target_id: Option<&str>) -> Vec<ResourceKindDescriptor> {
        let recycle = ConnectorAction {
            id: ACTION_RECYCLE.to_owned(),
            target_id: None,
            label: "Recycle".to_owned(),
            description: Some("Pretends to recycle one simulated resource.".to_owned()),
            params_schema: json!({
                "type": "object",
                "properties": {
                    "resourceId": { "type": "string", "minLength": 1 }
                },
                "required": ["resourceId"],
                "additionalProperties": false
            }),
            is_disruptive: false,
            snapshot_data_point_ids: Vec::new(),
        };

        vec![
            ResourceKindDescriptor::new(
                RESOURCE_KIND_WIDGETS,
                "Widgets",
                vec![
                    ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
                    ColumnDescriptor::new("size", "Size", ColumnValueType::Bytes),
                    ColumnDescriptor::new("createdAt", "Created", ColumnValueType::Timestamp),
                ],
            )
            .with_row_actions(vec![recycle.clone()])
            .with_kind_actions(vec![ConnectorAction::simple(
                ACTION_CLEANUP_ALL,
                "Clean up all",
            )
            .with_description("Pretends to clean up every simulated widget at once.")]),
            ResourceKindDescriptor::new(
                RESOURCE_KIND_GADGETS,
                "Gadgets",
                vec![
                    ColumnDescriptor::new("serial", "Serial", ColumnValueType::Text),
                    ColumnDescriptor::new("torque", "Torque", ColumnValueType::Number),
                    ColumnDescriptor::new("active", "Active", ColumnValueType::Bool),
                ],
            )
            .with_row_actions(vec![recycle]),
        ]
    }

    /// Canned rows for the two fixture kinds, and an empty list for anything
    /// else.
    ///
    /// The empty answer for an unknown kind is deliberate and matches the
    /// trait's default: "no such kind here" is not a failure of the listing,
    /// and a caller that needs to tell the two apart checks
    /// [`Connector::resource_kinds`], which is the authoritative list. Latency
    /// and fail mode apply exactly as they do everywhere else in this fixture,
    /// so a table's loading and error states are reachable without a service.
    ///
    /// The rows are fixed values rather than derived from the simulation: a
    /// table is something a test asserts on cell by cell, and readings that
    /// moved between two calls would make that impossible.
    async fn list_resource_items(
        &self,
        kind: &str,
        _target_id: Option<&str>,
    ) -> Result<Vec<ResourceItem>, ConnectorError> {
        self.gate().await?;

        Ok(match kind {
            RESOURCE_KIND_WIDGETS => vec![
                ResourceItem::new("widget-1")
                    .with_field("name", "alpha-widget")
                    .with_field("size", 1_048_576)
                    .with_field("createdAt", "2026-01-04T09:15:00Z"),
                ResourceItem::new("widget-2")
                    .with_field("name", "beta-widget")
                    .with_field("size", 734_003_200)
                    .with_field("createdAt", "2026-02-17T22:40:00Z"),
                ResourceItem::new("widget-3")
                    .with_field("name", "gamma-widget")
                    .with_field("size", 512)
                    .with_field("createdAt", "2026-03-01T06:00:00Z"),
            ],
            RESOURCE_KIND_GADGETS => vec![
                ResourceItem::new("gadget-1")
                    .with_field("serial", "SN-0001")
                    .with_field("torque", 42.5)
                    .with_field("active", true),
                ResourceItem::new("gadget-2")
                    .with_field("serial", "SN-0002")
                    .with_field("torque", 7.0)
                    .with_field("active", false),
            ],
            _ => Vec::new(),
        })
    }

    async fn discover(&self) -> Result<Vec<DiscoveredResource>, ConnectorError> {
        self.gate().await?;

        Ok(vec![
            DiscoveredResource {
                suggested_name: "Discovered Debug Fixture 1".to_owned(),
                target_connector_type: TYPE_ID.to_owned(),
                config: json!({
                    "simulatedHealth": "healthy",
                    "baseLoad": 24,
                    "label": "discovered-alpha",
                    "enabled": true
                }),
                target_field_value: None,
            },
            DiscoveredResource {
                suggested_name: "Discovered Debug Fixture 2".to_owned(),
                target_connector_type: TYPE_ID.to_owned(),
                config: json!({
                    "simulatedHealth": "degraded",
                    "simulatedLatencyMs": 150,
                    "baseLoad": 68,
                    "label": "discovered-beta",
                    "enabled": false
                }),
                target_field_value: None,
            },
            DiscoveredResource {
                suggested_name: "Discovered Debug Fixture 3".to_owned(),
                target_connector_type: TYPE_ID.to_owned(),
                config: json!({
                    "simulatedHealth": "unknown",
                    "baseLoad": 5,
                    "label": "discovered-gamma",
                    "enabled": true
                }),
                target_field_value: None,
            },
        ])
    }

    fn setup_guide(&self) -> Option<SetupGuide> {
        Some(SetupGuide {
            variants: vec![
                SetupGuideVariant {
                    id: "simple".to_owned(),
                    label: "Simple".to_owned(),
                    description: "Uses the live connection test for capability detail.".to_owned(),
                    template: "No setup needed — this is an internal test fixture.".to_owned(),
                    toggles: Vec::new(),
                    capability_requirements: Vec::new(),
                },
                SetupGuideVariant {
                    id: "configurable".to_owned(),
                    label: "Configurable".to_owned(),
                    description: "Exercises UI-only setup toggles and declarative capabilities."
                        .to_owned(),
                    template: "Debug setup for {{label}}\nLOOM_DEBUG_WIDGETS={{LOOM_DEBUG_WIDGETS}}\nLOOM_DEBUG_ACTIONS={{LOOM_DEBUG_ACTIONS}}"
                        .to_owned(),
                    toggles: vec![
                        SetupGuideToggle {
                            key: "enableWidgets".to_owned(),
                            env_var: "LOOM_DEBUG_WIDGETS".to_owned(),
                            label: "Enable widgets".to_owned(),
                            description: "Includes read-only widget support in the example setup."
                                .to_owned(),
                            default: true,
                            recommended: true,
                        },
                        SetupGuideToggle {
                            key: "enableActions".to_owned(),
                            env_var: "LOOM_DEBUG_ACTIONS".to_owned(),
                            label: "Enable actions".to_owned(),
                            description: "Includes mutating action support in the example setup."
                                .to_owned(),
                            default: false,
                            recommended: false,
                        },
                    ],
                    capability_requirements: vec![
                        CapabilityRequirement {
                            capability_key: "view-widgets".to_owned(),
                            label: "View widgets".to_owned(),
                            required_toggle_keys: vec!["enableWidgets".to_owned()],
                        },
                        CapabilityRequirement {
                            capability_key: "perform-actions".to_owned(),
                            label: "Perform actions".to_owned(),
                            required_toggle_keys: vec!["enableActions".to_owned()],
                        },
                    ],
                },
            ],
        })
    }

    fn metadata(&self) -> ConnectorMetadata {
        ConnectorMetadata {
            id: TYPE_ID.to_string(),
            name: "Debug Connector".to_string(),
            // `lucide:` rather than `brand:` — the fixture is not a product
            // and has no logo to vendor. A bug is what a debug connector is.
            // See `ConnectorMetadata::icon` for the two reference forms.
            icon: Some("lucide:bug".to_string()),
            version: env!("CARGO_PKG_VERSION").to_string(),
            // Two by two: enough for the stat tile and the status dot beside a
            // chart that is still readable. Small, because the fixture should
            // not be the thing that decides how big a dashboard grid has to be.
            min_size: (2, 2),
        }
    }

    /// Illustrative, obviously-fake values.
    ///
    /// `debug.invalid` is a reserved non-resolving name, and the rest name the
    /// fixture rather than any service, so nothing here can be mistaken for a
    /// real deployment detail.
    fn display_fields(&self) -> Vec<DisplayField> {
        let label = self.lock().label.clone();

        vec![
            DisplayField::new("Host", FAKE_HOST),
            DisplayField::new("Connector version", env!("CARGO_PKG_VERSION")),
            DisplayField::new("Label", label),
            DisplayField::new("Mode", "Simulated — contacts nothing"),
        ]
    }

    /// One data point of every [`DataPointValueType`], so a renderer can be
    /// built against all four without a real service.
    fn data_points(&self) -> Vec<DataPointDescriptor> {
        let host = vec![
            DataPointDescriptor::new(DATA_POINT_LOAD, "Load", DataPointValueType::Number)
                .with_unit("%"),
            DataPointDescriptor::new(DATA_POINT_LABEL, "Label", DataPointValueType::String),
            DataPointDescriptor::new(DATA_POINT_ENABLED, "Enabled", DataPointValueType::Bool),
            DataPointDescriptor::new(
                DATA_POINT_LOAD_HISTORY,
                "Load history",
                DataPointValueType::TimeSeries,
            )
            .with_unit("%"),
            DataPointDescriptor::new(
                DATA_POINT_LOG,
                "Recent activity",
                DataPointValueType::String,
            ),
        ];
        host.into_iter()
            .chain([
                DataPointDescriptor::new(DATA_POINT_LOAD, "Load", DataPointValueType::Number)
                    .with_unit("%")
                    .for_target(FIXTURE_TARGETS[0]),
                DataPointDescriptor::new(DATA_POINT_ENABLED, "Enabled", DataPointValueType::Bool)
                    .for_target(FIXTURE_TARGETS[0]),
                DataPointDescriptor::new(DATA_POINT_LABEL, "Label", DataPointValueType::String)
                    .for_target(FIXTURE_TARGETS[1]),
                DataPointDescriptor::new(DATA_POINT_ENABLED, "Enabled", DataPointValueType::Bool)
                    .for_target(FIXTURE_TARGETS[1]),
            ])
            .collect()
    }

    /// A spread across both binding kinds rather than the minimum that
    /// compiles.
    ///
    /// The load appears three times on purpose — as a tile, a gauge, and a bar
    /// — because the fixture's job includes letting three different renderers
    /// be compared side by side against the same moving number.
    /// Whatever the configuration asked for — see
    /// [`DebugConnectorConfig::network_target`].
    fn network_target(&self) -> Option<NetworkTarget> {
        self.config.network_target.clone()
    }

    fn default_layout(&self) -> WidgetLayout {
        WidgetLayout::new(vec![
            WidgetBinding::display(DATA_POINT_LOAD, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_ENABLED, DisplayWidgetType::StatusDot),
            WidgetBinding::display(
                DATA_POINT_LOAD_HISTORY,
                DisplayWidgetType::MetricChart {
                    chart_type: ChartType::Line,
                },
            ),
            WidgetBinding::display(DATA_POINT_LOAD, DisplayWidgetType::Gauge)
                .with_config(json!({ "min": 0, "max": 100 })),
            WidgetBinding::display(DATA_POINT_LOAD, DisplayWidgetType::ProgressBar)
                .with_config(json!({ "min": 0, "max": 100 })),
            WidgetBinding::display(DATA_POINT_LABEL, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_LOG, DisplayWidgetType::LogStream),
            // Both binding kinds on purpose: a layout that only ever displayed
            // would not exercise the half of the renderer that has to send an
            // action, which is the gap the flat binding shape used to hide.
            WidgetBinding::action(ACTION_SET_ENABLED, ActionWidgetType::Toggle),
            WidgetBinding::action(ACTION_SET_LOAD, ActionWidgetType::Slider)
                .with_config(json!({ "min": 0, "max": 100, "step": 1 })),
            WidgetBinding::action(ACTION_SET_LABEL, ActionWidgetType::TextField),
            WidgetBinding::action(ACTION_RESTART, ActionWidgetType::Button),
        ])
    }

    fn default_layout_for(&self, target_id: Option<&str>) -> WidgetLayout {
        match target_id {
            None => self.default_layout(),
            Some("fixture-a") => WidgetLayout::new(vec![
                WidgetBinding::display(DATA_POINT_LOAD, DisplayWidgetType::StatTile),
                WidgetBinding::display(DATA_POINT_ENABLED, DisplayWidgetType::StatusDot),
                WidgetBinding::action(ACTION_PING, ActionWidgetType::Button),
            ]),
            Some(_) => WidgetLayout::new(vec![
                WidgetBinding::display(DATA_POINT_LABEL, DisplayWidgetType::StatTile),
                WidgetBinding::display(DATA_POINT_ENABLED, DisplayWidgetType::StatusDot),
                WidgetBinding::action(ACTION_RESTART, ActionWidgetType::Button),
            ]),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::time::Instant;

    #[tokio::test]
    async fn default_fixture_is_healthy_and_lists_every_action() {
        let connector = DebugConnector::default();

        let status = connector.status().await.expect("default must succeed");
        assert_eq!(status.health, HealthState::Healthy);

        let ids: Vec<String> = connector
            .actions()
            .await
            .into_iter()
            .filter(|action| action.target_id.is_none())
            .map(|action| action.id)
            .collect();
        assert_eq!(
            ids,
            vec![
                ACTION_RESTART,
                ACTION_PING,
                ACTION_RECALIBRATE,
                ACTION_SET_ENABLED,
                ACTION_SET_LOAD,
                ACTION_SET_LABEL
            ]
        );

        assert_eq!(connector.metadata().id, TYPE_ID);
        assert_eq!(connector.metadata().min_size, (2, 2));
        assert!(connector.config_schema().is_object());
    }

    #[tokio::test]
    async fn discovery_returns_valid_debug_connector_suggestions() {
        let connector = DebugConnector::default();
        assert_eq!(connector.discoverable_type().as_deref(), Some(TYPE_ID));

        let resources = connector.discover().await.expect("discovery must succeed");
        assert_eq!(resources.len(), 3);
        for resource in resources {
            assert_eq!(resource.target_connector_type, TYPE_ID);
            assert_eq!(resource.target_field_value, None);
            assert!(resource
                .suggested_name
                .starts_with("Discovered Debug Fixture"));
            DebugConnector::from_config_value(resource.config)
                .expect("every discovered config must satisfy the real parser");
        }
    }

    #[test]
    fn setup_guide_exposes_static_and_toggle_driven_variants() {
        let connector = DebugConnector::default();
        let guide = connector.setup_guide().expect("debug publishes setup help");

        assert_eq!(guide.variants.len(), 2);
        let simple = &guide.variants[0];
        assert_eq!(simple.id, "simple");
        assert!(simple.toggles.is_empty());
        assert!(simple.capability_requirements.is_empty());

        let configurable = &guide.variants[1];
        assert_eq!(configurable.id, "configurable");
        assert_eq!(configurable.toggles.len(), 2);
        assert!(configurable.template.contains("{{label}}"));
        assert!(configurable.template.contains("{{LOOM_DEBUG_WIDGETS}}"));
        assert!(configurable.template.contains("{{LOOM_DEBUG_ACTIONS}}"));
        assert_eq!(configurable.capability_requirements.len(), 2);
        assert_eq!(
            configurable.capability_requirements[1].required_toggle_keys,
            ["enableActions"]
        );
        assert!(connector.config_schema()["properties"]["label"].is_object());
    }

    #[tokio::test]
    async fn connection_test_reports_reachability_and_conditional_capabilities() {
        let enabled = DebugConnector::default().test_connection().await;
        assert!(enabled.reachable);
        assert_eq!(enabled.message, None);
        assert!(enabled.capabilities.iter().all(|status| status.available));

        let disabled = DebugConnector::new(DebugConnectorConfig {
            enabled: false,
            ..DebugConnectorConfig::default()
        })
        .test_connection()
        .await;
        assert!(disabled.reachable);
        assert!(disabled
            .capabilities
            .iter()
            .find(|status| status.key == "read-status")
            .is_some_and(|status| status.available));
        let actions = disabled
            .capabilities
            .iter()
            .find(|status| status.key == "perform-actions")
            .expect("action capability");
        assert!(!actions.available);
        assert!(actions
            .note
            .as_deref()
            .is_some_and(|note| note.contains("enabled")));
    }

    #[tokio::test]
    async fn connection_test_respects_health_fail_mode_and_latency() {
        let down = DebugConnector::new(DebugConnectorConfig {
            simulated_status: ConnectorStatus::new(HealthState::Down, json!({})),
            ..DebugConnectorConfig::default()
        })
        .test_connection()
        .await;
        assert!(!down.reachable);
        assert!(down.capabilities.iter().all(|status| !status.available));
        assert!(down
            .message
            .as_deref()
            .is_some_and(|message| message.contains("Down")));

        let failing = DebugConnector::failing(ConnectorError::unreachable("simulated outage"));
        let failure = failing.test_connection().await;
        assert!(!failure.reachable);
        assert!(failure.capabilities.iter().all(|status| !status.available));
        assert!(failure
            .message
            .as_deref()
            .is_some_and(|message| message.contains("simulated outage")));

        let delayed = DebugConnector::with_latency(20);
        let started = Instant::now();
        assert!(delayed.test_connection().await.reachable);
        assert!(started.elapsed().as_millis() >= 20);
    }

    #[test]
    fn the_icon_uses_a_prefixed_reference_a_client_can_resolve() {
        // Core cannot check that the *target* exists — only a client knows what
        // it has vendored. What it can check is that the reference is one of
        // the two documented forms, because an unprefixed name like `"bug"` is
        // silently unresolvable everywhere rather than loudly wrong anywhere.
        let icon = DebugConnector::default()
            .metadata()
            .icon
            .expect("the fixture declares an icon, as the convention's example");

        let (scheme, name) = icon
            .split_once(':')
            .unwrap_or_else(|| panic!("{icon} is not a prefixed icon reference"));
        assert!(
            matches!(scheme, "brand" | "lucide"),
            "{scheme} is not one of the two documented icon schemes"
        );
        assert!(!name.is_empty(), "{icon} names nothing after its prefix");
        assert_eq!(
            name,
            name.to_lowercase(),
            "icon references are kebab-case, never PascalCase"
        );
    }

    #[tokio::test]
    async fn configured_health_and_details_survive_the_simulated_values() {
        let connector = DebugConnector::new(DebugConnectorConfig {
            simulated_status: ConnectorStatus::new(
                HealthState::Degraded,
                json!({ "queueDepth": 7 }),
            ),
            ..DebugConnectorConfig::default()
        });

        let status = connector.status().await.unwrap();
        assert_eq!(status.health, HealthState::Degraded);
        // The configured detail is still there...
        assert_eq!(status.data_point_value("queueDepth"), Some(&json!(7)));
        // ...alongside every data point's current value, keyed by its own id
        // and shaped by its declared value type.
        for descriptor in connector.data_points() {
            let value = status
                .data_point_value_for(descriptor.target_id.as_deref(), &descriptor.id)
                .unwrap_or_else(|| {
                    panic!("status details is missing data point {}", descriptor.id)
                });
            match descriptor.value_type {
                DataPointValueType::Number => assert!(
                    value.is_number(),
                    "{} must be a JSON number, got {value}",
                    descriptor.id
                ),
                DataPointValueType::String => assert!(
                    value.is_string(),
                    "{} must be a JSON string, got {value}",
                    descriptor.id
                ),
                DataPointValueType::Bool => assert!(
                    value.is_boolean(),
                    "{} must be a JSON boolean, got {value}",
                    descriptor.id
                ),
                DataPointValueType::TimeSeries => {
                    let samples = value
                        .as_array()
                        .unwrap_or_else(|| panic!("{} must be a JSON array", descriptor.id));
                    assert!(!samples.is_empty());
                    for sample in samples {
                        assert!(
                            sample["value"].is_number(),
                            "a time series sample needs a numeric `value`, got {sample}"
                        );
                        let timestamp = sample["timestamp"]
                            .as_str()
                            .expect("a time series sample needs a string `timestamp`");
                        DateTime::parse_from_rfc3339(timestamp)
                            .expect("the sample timestamp must be ISO 8601");
                    }
                }
            }
        }
    }

    #[tokio::test]
    async fn data_points_cover_every_value_type_and_have_unique_ids() {
        let points = DebugConnector::default().data_points();
        assert_eq!(points.len(), 9);

        let ids: HashSet<(&str, Option<&str>)> = points
            .iter()
            .map(|p| (p.id.as_str(), p.target_id.as_deref()))
            .collect();
        assert_eq!(ids.len(), points.len(), "data point ids must be unique");

        let types: HashSet<DataPointValueType> = points.iter().map(|p| p.value_type).collect();
        assert_eq!(
            types.len(),
            4,
            "the fixture must exercise all four value types"
        );

        assert!(points.iter().all(|p| !p.label.is_empty()));
    }

    #[tokio::test]
    async fn the_log_data_point_is_newline_joined_and_stays_capped() {
        let connector = DebugConnector::default();

        let lines = |details: &Value| -> Vec<String> {
            super::super::details::get_detail(details, None, DATA_POINT_LOG)
                .expect("the log detail must exist")
                .as_str()
                .expect("the log data point must be a JSON string")
                .lines()
                .map(str::to_string)
                .collect()
        };

        let first = connector.status().await.unwrap().details;
        assert_eq!(lines(&first).len(), 1);

        for _ in 0..(LOG_CAPACITY + 5) {
            connector.status().await.unwrap();
        }
        let later = connector.status().await.unwrap().details;
        let later_lines = lines(&later);
        assert_eq!(later_lines.len(), LOG_CAPACITY);

        // Newest last, so a pane scrolled to the bottom shows the latest line.
        assert_ne!(later_lines.first(), later_lines.last());
        assert!(later_lines
            .iter()
            .all(|line| line.contains("simulated tick")));

        // Nothing here may look like it came from a real service.
        assert!(!later.to_string().contains(".com"));
    }

    #[tokio::test]
    async fn the_default_layout_only_binds_things_that_exist() {
        let connector = DebugConnector::default();
        let point_ids: HashSet<String> = connector
            .data_points()
            .into_iter()
            .map(|point| point.id)
            .collect();
        let action_ids: HashSet<String> = connector
            .actions()
            .await
            .into_iter()
            .map(|action| action.id)
            .collect();

        let layout = connector.default_layout();
        assert_eq!(layout.bindings.len(), 11);

        for binding in &layout.bindings {
            match binding {
                WidgetBinding::Display {
                    data_point_id,
                    config,
                    ..
                } => {
                    assert!(
                        point_ids.contains(data_point_id),
                        "layout binds unknown data point {data_point_id}"
                    );
                    assert!(config.is_object(), "config must always be an object");
                }
                WidgetBinding::Action {
                    action_id, config, ..
                } => {
                    assert!(
                        action_ids.contains(action_id),
                        "layout binds unknown action {action_id}"
                    );
                    assert!(config.is_object(), "config must always be an object");
                }
            }
        }
    }

    #[tokio::test]
    async fn the_default_layout_exercises_both_binding_kinds() {
        let layout = DebugConnector::default().default_layout();

        // The three display shapes a renderer has to get right first.
        assert!(layout.bindings.contains(&WidgetBinding::display(
            DATA_POINT_LOAD,
            DisplayWidgetType::StatTile
        )));
        assert!(layout.bindings.contains(&WidgetBinding::display(
            DATA_POINT_ENABLED,
            DisplayWidgetType::StatusDot
        )));
        assert!(layout.bindings.contains(&WidgetBinding::display(
            DATA_POINT_LOAD_HISTORY,
            DisplayWidgetType::MetricChart {
                chart_type: ChartType::Line
            }
        )));
        assert!(layout.bindings.contains(&WidgetBinding::display(
            DATA_POINT_LOG,
            DisplayWidgetType::LogStream
        )));

        // ...and at least one control, so the action half of the renderer has
        // something to be built against.
        let actions: Vec<&WidgetBinding> = layout
            .bindings
            .iter()
            .filter(|binding| matches!(binding, WidgetBinding::Action { .. }))
            .collect();
        assert!(
            !actions.is_empty(),
            "the fixture must ship at least one action binding"
        );
        assert!(actions.contains(&&WidgetBinding::action(
            ACTION_SET_ENABLED,
            ActionWidgetType::Toggle
        )));

        // The bounded widgets must carry the bounds they need to draw.
        for binding in &layout.bindings {
            let config = match binding {
                WidgetBinding::Display {
                    widget_type: DisplayWidgetType::Gauge | DisplayWidgetType::ProgressBar,
                    config,
                    ..
                } => config,
                WidgetBinding::Action {
                    widget_type: ActionWidgetType::Slider,
                    config,
                    ..
                } => config,
                _ => continue,
            };
            assert_eq!(config["min"], json!(0));
            assert_eq!(config["max"], json!(100));
        }
    }

    #[tokio::test]
    async fn display_fields_are_present_and_obviously_fake() {
        let fields = DebugConnector::default().display_fields();
        assert!(!fields.is_empty());
        assert!(fields
            .iter()
            .all(|f| !f.label.is_empty() && !f.value.is_empty()));

        let host = fields
            .iter()
            .find(|f| f.label == "Host")
            .expect("a host field");
        assert_eq!(host.value, FAKE_HOST);
        assert!(
            host.value.ends_with(".invalid"),
            "the fake host must stay in a reserved, non-resolving namespace"
        );
    }

    #[tokio::test]
    async fn the_load_time_series_grows_and_stays_capped() {
        let connector = DebugConnector::default();

        for _ in 0..3 {
            connector.status().await.unwrap();
        }
        let status = connector.status().await.unwrap();
        let series = status
            .data_point_value(DATA_POINT_LOAD_HISTORY)
            .and_then(Value::as_array)
            .expect("a time series");
        assert_eq!(series.len(), 4);

        // Oldest first, so a chart can plot the array in order without sorting.
        let timestamps: Vec<DateTime<Utc>> = series
            .iter()
            .map(|sample| {
                DateTime::parse_from_rfc3339(sample["timestamp"].as_str().expect("a timestamp"))
                    .expect("ISO 8601")
                    .with_timezone(&Utc)
            })
            .collect();
        assert!(timestamps.windows(2).all(|pair| pair[0] <= pair[1]));

        for _ in 0..(HISTORY_CAPACITY + 10) {
            connector.status().await.unwrap();
        }
        let series = connector
            .status()
            .await
            .unwrap()
            .data_point_value(DATA_POINT_LOAD_HISTORY)
            .expect("load history detail")
            .as_array()
            .expect("a time series")
            .len();
        assert_eq!(series, HISTORY_CAPACITY);
    }

    #[tokio::test]
    async fn the_load_reading_moves_between_polls_and_stays_in_range() {
        let connector = DebugConnector::default();

        let mut seen = HashSet::new();
        for _ in 0..10 {
            let load = connector
                .status()
                .await
                .unwrap()
                .data_point_value(DATA_POINT_LOAD)
                .expect("load detail")
                .as_f64()
                .expect("a number");
            assert!(
                (0.0..=100.0).contains(&load),
                "load escaped its range: {load}"
            );
            seen.insert(load.to_bits());
        }

        assert!(
            seen.len() > 1,
            "a chart needs the value to actually change between polls"
        );
    }

    #[tokio::test]
    async fn resource_kinds_describe_two_browsable_tables() {
        let connector = DebugConnector::default();
        let kinds = connector.resource_kinds(None);

        let ids: Vec<&str> = kinds.iter().map(|kind| kind.kind.as_str()).collect();
        assert_eq!(ids, vec![RESOURCE_KIND_WIDGETS, RESOURCE_KIND_GADGETS]);

        let widgets = &kinds[0];
        assert_eq!(widgets.label, "Widgets");
        assert_eq!(
            widgets
                .columns
                .iter()
                .map(|column| (column.key.as_str(), column.value_type))
                .collect::<Vec<_>>(),
            vec![
                ("name", ColumnValueType::Text),
                ("size", ColumnValueType::Bytes),
                ("createdAt", ColumnValueType::Timestamp),
            ]
        );
        assert_eq!(
            widgets
                .row_actions
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            vec![ACTION_RECYCLE]
        );
        assert_eq!(
            widgets
                .kind_actions
                .iter()
                .map(|action| action.id.as_str())
                .collect::<Vec<_>>(),
            vec![ACTION_CLEANUP_ALL]
        );

        // The second kind deliberately has no kind actions: a client that
        // assumed every kind offers both scopes would render a phantom button.
        let gadgets = &kinds[1];
        assert!(gadgets.kind_actions.is_empty());
        assert_eq!(
            gadgets
                .columns
                .iter()
                .map(|column| column.value_type)
                .collect::<Vec<_>>(),
            vec![
                ColumnValueType::Text,
                ColumnValueType::Number,
                ColumnValueType::Bool,
            ]
        );

        // Between the two, every cell format a renderer has to handle is
        // reachable from the fixture alone.
        let covered: std::collections::HashSet<ColumnValueType> = kinds
            .iter()
            .flat_map(|kind| kind.columns.iter().map(|column| column.value_type))
            .collect();
        for value_type in [
            ColumnValueType::Text,
            ColumnValueType::Number,
            ColumnValueType::Bool,
            ColumnValueType::Timestamp,
            ColumnValueType::Bytes,
        ] {
            assert!(
                covered.contains(&value_type),
                "{value_type:?} is unexercised"
            );
        }

        // Resource-kind descriptors are not gated on health: what a connector
        // can browse stays true while its service is unreachable.
        assert_eq!(
            DebugConnector::failing(ConnectorError::unreachable("simulated"))
                .resource_kinds(None)
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn resource_items_are_canned_rows_keyed_by_their_columns() {
        let connector = DebugConnector::default();

        let widgets = connector
            .list_resource_items(RESOURCE_KIND_WIDGETS, None)
            .await
            .expect("listing widgets");
        assert_eq!(widgets.len(), 3);
        assert_eq!(widgets[0].id, "widget-1");
        assert_eq!(widgets[0].fields["name"], json!("alpha-widget"));
        assert_eq!(widgets[0].fields["size"], json!(1_048_576));
        assert_eq!(
            widgets[0].fields["createdAt"],
            json!("2026-01-04T09:15:00Z")
        );

        // Every row fills every declared column, or a table renderer would be
        // built against holes the fixture invented rather than against data.
        let kinds = connector.resource_kinds(None);
        for kind in &kinds {
            let rows = connector
                .list_resource_items(&kind.kind, None)
                .await
                .expect("listing a declared kind");
            assert!(!rows.is_empty(), "{} has no rows", kind.kind);
            for row in &rows {
                assert!(!row.id.is_empty());
                for column in &kind.columns {
                    assert!(
                        row.fields.contains_key(&column.key),
                        "row {} is missing column {}",
                        row.id,
                        column.key
                    );
                }
            }
        }

        // A kind nobody declared is an empty table, not a failure — the same
        // answer the trait's default gives.
        assert_eq!(
            connector
                .list_resource_items("nonexistent", None)
                .await
                .expect("an unknown kind is not an error"),
            Vec::new()
        );
    }

    #[tokio::test]
    async fn listing_resources_respects_latency_and_fail_mode() {
        let start = std::time::Instant::now();
        assert!(!DebugConnector::with_latency(60)
            .list_resource_items(RESOURCE_KIND_WIDGETS, None)
            .await
            .expect("listing should still succeed")
            .is_empty());
        assert!(start.elapsed() >= std::time::Duration::from_millis(60));

        let failing = DebugConnector::failing(ConnectorError::unreachable("simulated outage"));
        assert_eq!(
            failing
                .list_resource_items(RESOURCE_KIND_WIDGETS, None)
                .await
                .expect_err("fail mode must reach the listing too"),
            ConnectorError::unreachable("simulated outage")
        );
    }

    #[tokio::test]
    async fn resource_actions_cover_both_scopes_and_enforce_the_row_id() {
        let connector = DebugConnector::default();

        let recycled = connector
            .execute_action(ACTION_RECYCLE, None, json!({ "resourceId": "widget-2" }))
            .await
            .expect("recycle should succeed");
        assert!(recycled.success);
        assert_eq!(
            recycled.payload,
            Some(json!({ "recycled": "widget-2" })),
            "the row action must echo the row it acted on"
        );

        // A kind action addresses no row, so it must not demand one.
        let cleaned = connector
            .execute_action(ACTION_CLEANUP_ALL, None, Value::Null)
            .await
            .expect("cleanup should succeed");
        assert!(cleaned.success);

        // A row action without a row is refused rather than guessed at.
        for params in [Value::Null, json!({}), json!({ "resourceId": "  " })] {
            assert_eq!(
                connector
                    .execute_action(ACTION_RECYCLE, None, params.clone())
                    .await
                    .expect_err("a row action needs its row"),
                ConnectorError::InvalidParams {
                    action_id: ACTION_RECYCLE.to_string(),
                    reason: "expected a non-empty string `resourceId`".to_string(),
                },
                "params {params} should have been refused"
            );
        }
    }

    #[tokio::test]
    async fn the_recalibrate_action_declares_a_snapshot_and_then_destroys_the_reading() {
        let connector = DebugConnector::default();

        let recalibrate = connector
            .actions()
            .await
            .into_iter()
            .find(|action| action.id == ACTION_RECALIBRATE && action.target_id.is_none())
            .expect("the fixture must advertise recalibrate");
        assert_eq!(
            recalibrate.snapshot_data_point_ids,
            vec![DATA_POINT_LOAD.to_string()],
            "the snapshot list is what tells the platform which reading to record"
        );
        // Disruptive as well as snapshot-bearing: this one action exercises
        // both of the platform behaviours a descriptor can ask for.
        assert!(recalibrate.is_disruptive);

        // What the connector currently reports is what a pre-action snapshot
        // would capture. Read through `status()` rather than set through
        // `set-load`, because the fixture re-derives its load on every poll —
        // which is also why the old value is genuinely unrecoverable
        // afterwards, and why recording it is worth doing.
        let before = connector
            .status()
            .await
            .expect("status")
            .data_point_value(DATA_POINT_LOAD)
            .cloned()
            .expect("the load reading");

        let result = connector
            .execute_action(ACTION_RECALIBRATE, None, Value::Null)
            .await
            .expect("recalibrate should succeed");
        assert!(result.success);
        let payload = result
            .payload
            .expect("recalibrate reports what it replaced");
        assert_eq!(
            payload["previousLoad"], before,
            "the action overwrote exactly the value the snapshot would have recorded"
        );
        assert_eq!(payload[DATA_POINT_LOAD], json!(RECALIBRATED_LOAD));
    }

    #[tokio::test]
    async fn the_update_check_reports_whatever_the_configuration_asked_for() {
        // Supported unconditionally: the capability is a property of the
        // connector, not of what it currently finds.
        assert!(DebugConnector::default().supports_update_checking());

        assert_eq!(
            DebugConnector::default()
                .check_for_updates(None)
                .await
                .expect("the default fixture is up to date"),
            UpdateCheckResult::up_to_date()
        );

        let outdated = DebugConnector::new(DebugConnectorConfig {
            simulated_update_available: true,
            ..DebugConnectorConfig::default()
        });
        let found = outdated
            .check_for_updates(None)
            .await
            .expect("an update should be reported");
        assert!(found.available);
        assert_eq!(found.latest_ref.as_deref(), Some(SIMULATED_LATEST_REF));

        // Sub-target scoping is accepted; the fixture answers the same way for
        // every target, which is enough to prove the argument reaches it.
        assert_eq!(
            outdated
                .check_for_updates(Some(FIXTURE_TARGETS[0]))
                .await
                .expect("a targeted check should succeed"),
            found
        );

        // An update check is a network call in every real connector, so the
        // fixture's failure and latency knobs have to reach it too.
        assert_eq!(
            DebugConnector::failing(ConnectorError::unreachable("registry is down"))
                .check_for_updates(None)
                .await
                .expect_err("fail mode must reach the update check"),
            ConnectorError::unreachable("registry is down")
        );
        let start = std::time::Instant::now();
        DebugConnector::with_latency(60)
            .check_for_updates(None)
            .await
            .expect("a slow check still succeeds");
        assert!(start.elapsed() >= std::time::Duration::from_millis(60));
    }

    #[test]
    fn the_update_check_config_knob_is_read_from_stored_configuration() {
        let connector = DebugConnector::from_config_value(json!({
            "simulatedUpdateAvailable": true,
        }))
        .expect("a valid configuration");
        assert!(connector.config().simulated_update_available);

        // Absent means "up to date", which is the answer a fixture nobody has
        // configured should give.
        assert!(
            !DebugConnector::from_config_value(json!({}))
                .expect("an empty configuration")
                .config()
                .simulated_update_available
        );
    }

    #[tokio::test]
    async fn canned_actions_succeed_and_echo_their_params() {
        let connector = DebugConnector::default();

        let restart = connector
            .execute_action(ACTION_RESTART, None, json!({ "force": true }))
            .await
            .unwrap();
        assert!(restart.success);
        assert_eq!(restart.payload.unwrap()["params"], json!({ "force": true }));

        let ping = connector
            .execute_action(ACTION_PING, None, Value::Null)
            .await
            .unwrap();
        assert!(ping.success);
    }

    #[tokio::test]
    async fn parameterized_actions_change_what_status_reports() {
        let connector = DebugConnector::default();

        connector
            .execute_action(ACTION_SET_ENABLED, None, json!({ "enabled": false }))
            .await
            .unwrap();
        connector
            .execute_action(ACTION_SET_LABEL, None, json!({ "label": "renamed" }))
            .await
            .unwrap();

        let status = connector.status().await.unwrap();
        assert_eq!(
            status.data_point_value(DATA_POINT_ENABLED),
            Some(&json!(false))
        );
        assert_eq!(
            status.data_point_value(DATA_POINT_LABEL),
            Some(&json!("renamed"))
        );

        // The label is a display field too, so the shell shows the new value
        // without waiting for a poll.
        assert!(connector
            .display_fields()
            .iter()
            .any(|field| field.value == "renamed"));

        let result = connector
            .execute_action(ACTION_SET_LOAD, None, json!({ "value": 12.5 }))
            .await
            .unwrap();
        assert_eq!(result.payload.unwrap()[DATA_POINT_LOAD], json!(12.5));
    }

    #[tokio::test]
    async fn parameterized_actions_reject_bad_params() {
        let connector = DebugConnector::default();

        for (action, params, field) in [
            (ACTION_SET_ENABLED, json!({}), "enabled"),
            (ACTION_SET_ENABLED, json!({ "enabled": "yes" }), "enabled"),
            (ACTION_SET_LOAD, json!({ "value": "lots" }), "value"),
            (ACTION_SET_LABEL, json!({ "label": "  " }), "label"),
        ] {
            let error = connector
                .execute_action(action, None, params.clone())
                .await
                .expect_err("bad params must be refused");
            match error {
                ConnectorError::InvalidParams { action_id, reason } => {
                    assert_eq!(action_id, action);
                    assert!(reason.contains(field), "unhelpful reason: {reason}");
                }
                other => panic!("expected InvalidParams, got {other:?}"),
            }
        }

        // Out of range is a different failure from the wrong type, and both are
        // the connector's to detect.
        let error = connector
            .execute_action(ACTION_SET_LOAD, None, json!({ "value": 900 }))
            .await
            .expect_err("out of range must be refused");
        assert!(matches!(error, ConnectorError::InvalidParams { .. }));
    }

    #[tokio::test]
    async fn unknown_action_id_is_rejected() {
        let error = DebugConnector::default()
            .execute_action("not-a-real-action", None, Value::Null)
            .await
            .expect_err("unknown ids must not silently succeed");

        assert_eq!(error, ConnectorError::invalid_action("not-a-real-action"));
    }

    #[tokio::test]
    async fn latency_is_respected() {
        // Asserted loosely: the timer guarantees a lower bound, not an exact
        // duration, and a busy CI machine can always take longer.
        let connector = DebugConnector::with_latency(50);

        let started = Instant::now();
        connector.status().await.unwrap();
        assert!(
            started.elapsed().as_millis() >= 50,
            "status returned after {:?}, expected at least 50ms",
            started.elapsed()
        );

        let started = Instant::now();
        connector.actions().await;
        assert!(started.elapsed().as_millis() >= 50);

        let started = Instant::now();
        connector
            .execute_action(ACTION_PING, None, Value::Null)
            .await
            .unwrap();
        assert!(started.elapsed().as_millis() >= 50);
    }

    #[tokio::test]
    async fn fail_mode_applies_to_every_entry_point() {
        let configured = ConnectorError::AuthFailed {
            reason: "simulated bad token".to_string(),
        };
        let connector = DebugConnector::failing(configured.clone());

        assert_eq!(connector.status().await.unwrap_err(), configured);
        assert_eq!(
            connector
                .execute_action(ACTION_RESTART, None, Value::Null)
                .await
                .unwrap_err(),
            configured
        );
        // Even a *valid* action id fails, and an invalid one reports the fail
        // mode rather than `InvalidAction` — the connector never got far enough
        // to look the id up.
        assert_eq!(
            connector
                .execute_action("not-a-real-action", None, Value::Null)
                .await
                .unwrap_err(),
            configured
        );
        // Parameter validation is also downstream of the gate, so a broken
        // connector reports being broken rather than complaining about params.
        assert_eq!(
            connector
                .execute_action(ACTION_SET_ENABLED, None, json!({}))
                .await
                .unwrap_err(),
            configured
        );
        // `actions()` cannot return an error, so it degrades to "nothing is
        // available"; see the impl for why.
        assert!(connector.actions().await.is_empty());
    }

    #[tokio::test]
    async fn fail_mode_is_reusable_across_calls() {
        let connector = DebugConnector::failing(ConnectorError::unreachable("simulated outage"));

        for _ in 0..3 {
            assert_eq!(
                connector.status().await.unwrap_err(),
                ConnectorError::unreachable("simulated outage")
            );
        }
    }

    #[test]
    fn config_default_is_the_happy_path() {
        let config = DebugConnectorConfig::default();
        assert_eq!(config.simulated_status.health, HealthState::Healthy);
        assert_eq!(config.simulated_latency_ms, 0);
        assert_eq!(config.fail_mode, None);
        // Compared field-by-field: `simulated_status.last_checked` is stamped
        // with the construction time, so two defaults are never `==`.
        let from_connector = DebugConnector::default();
        assert_eq!(
            from_connector.config().simulated_status.health,
            config.simulated_status.health
        );
        assert_eq!(from_connector.config().simulated_latency_ms, 0);
        assert_eq!(from_connector.config().fail_mode, None);
    }

    #[test]
    fn an_empty_config_value_builds_the_default_fixture() {
        for empty in [Value::Null, json!({})] {
            let connector =
                DebugConnector::from_config_value(empty).expect("no configuration is valid");
            assert_eq!(connector.config().base_load, 42.0);
            assert_eq!(connector.config().simulated_latency_ms, 0);
            assert_eq!(connector.config().fail_mode, None);
        }
    }

    #[test]
    fn a_full_config_value_is_applied() {
        let connector = DebugConnector::from_config_value(json!({
            "simulatedLatencyMs": 25,
            "simulatedHealth": "degraded",
            "failMode": "unreachable",
            "baseLoad": 7.5,
            "label": "custom",
            "enabled": false
        }))
        .expect("a well-formed configuration");

        let config = connector.config();
        assert_eq!(config.simulated_latency_ms, 25);
        assert_eq!(config.simulated_status.health, HealthState::Degraded);
        assert_eq!(
            config.fail_mode,
            Some(ConnectorError::unreachable("simulated outage"))
        );
        assert_eq!(config.base_load, 7.5);
        assert_eq!(config.label, "custom");
        assert!(!config.enabled);
    }

    #[test]
    fn a_bad_config_value_is_refused_with_a_reason() {
        let cases = [
            (json!({ "baseLoad": 900 }), "baseLoad"),
            (json!({ "label": "" }), "label"),
            (json!({ "notAField": 1 }), "notAField"),
            // serde's own message, which names the offending value rather than
            // the field — enough for a user to find it, and not worth
            // reconstructing a path for.
            (json!({ "simulatedHealth": "sideways" }), "sideways"),
            (json!({ "simulatedLatencyMs": -1 }), "-1"),
            (json!([1, 2, 3]), ""),
        ];

        for (config, expected_mention) in cases {
            let error = DebugConnector::from_config_value(config.clone())
                .err()
                .unwrap_or_else(|| panic!("{config} must be refused"));
            match error {
                ConnectorError::InvalidConfig { reason } => assert!(
                    reason.contains(expected_mention),
                    "reason {reason:?} does not mention {expected_mention:?}"
                ),
                other => panic!("expected InvalidConfig, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn clones_share_the_simulated_state() {
        // The runtime hands out `Arc<dyn Connector>`, and a fixture whose state
        // forked per clone would report different values to two requests.
        let connector = DebugConnector::default();
        let clone = connector.clone();

        connector
            .execute_action(ACTION_SET_LABEL, None, json!({ "label": "shared" }))
            .await
            .unwrap();

        assert_eq!(
            clone
                .status()
                .await
                .unwrap()
                .data_point_value(DATA_POINT_LABEL),
            Some(&json!("shared"))
        );
    }

    #[tokio::test]
    async fn sub_targets_and_their_default_layouts_are_distinct() {
        let connector = DebugConnector::default();
        assert!(connector.supports_sub_targets());
        assert_eq!(
            connector.list_sub_targets().await.unwrap(),
            vec![
                SubTarget::new("fixture-a", "fixture-a"),
                SubTarget::new("fixture-b", "fixture-b"),
            ]
        );
        // A connector with nothing to distinguish leaves every target the
        // default kind, which is what `SubTarget::new` gives it.
        assert!(connector
            .list_sub_targets()
            .await
            .unwrap()
            .iter()
            .all(|target| target.kind == crate::connector::SUB_TARGET_KIND_DEFAULT));

        let host = connector.default_layout_for(None);
        let fixture_a = connector.default_layout_for(Some("fixture-a"));
        let fixture_b = connector.default_layout_for(Some("fixture-b"));
        assert_ne!(host, fixture_a);
        assert_ne!(fixture_a, fixture_b);
        assert_eq!(fixture_a.bindings.len(), 3);
    }
}
