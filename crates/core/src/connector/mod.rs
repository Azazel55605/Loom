//! The connector contract: how Loom talks to a service it manages.
//!
//! A *connector* is the adapter between Loom and one thing running in a
//! homelab — a media server, a reverse proxy, a hypervisor. It answers two
//! questions and nothing else: *is this service alright?* ([`Connector::status`])
//! and *what can I ask it to do?* ([`Connector::actions`] /
//! [`Connector::execute_action`]). That split is deliberate: Loom is a
//! management platform, so a connector that can only report uptime is only half
//! a connector.
//!
//! Everything in this module is a wire type. The web backend hands these values
//! straight to its HTTP layer and the TypeScript clients deserialize them
//! as-is, so field names are `camelCase` throughout and the shapes are treated
//! as public API rather than internal detail. See
//! `docs/adr/0002-connector-contract-tbd.md` — the *transport* for connector
//! definitions (declarative manifests, sandboxed WASM, or both) is still open,
//! but both candidates need a Rust-side trait to be driven through, and this is
//! it.
//!
//! Note what is absent: no credentials, no permission checks, no notion of
//! *who* is asking. A connector executes what it is told. Deciding whether a
//! caller is allowed to tell it that belongs to `web-backend`, because this
//! crate also gets linked into clients running on the user's own machine.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod debug;

/// An adapter for one manageable service.
///
/// Implementors are held behind `Box<dyn Connector>` in a registry keyed by
/// [`ConnectorMetadata::id`], so the trait is deliberately dyn-compatible:
/// no generic methods, no `Self`-returning methods, and `#[async_trait]`
/// instead of native `async fn` (which is stable on the pinned toolchain but
/// not object-safe). `Send + Sync` is required because the backend keeps that
/// registry in shared state across request tasks.
///
/// Implementations should be cheap to construct and hold their own
/// configuration; the trait has no `configure` step. Instead,
/// [`Connector::config_schema`] describes what configuration the connector
/// expects so that the loader can validate it and the clients can render a
/// form for it without shipping per-connector UI code.
#[async_trait]
pub trait Connector: Send + Sync {
    /// Checks in on the service and reports how it is doing right now.
    ///
    /// This is expected to be called repeatedly on a polling interval by
    /// whoever owns the schedule (never by the connector itself — core starts
    /// no background work). Implementations should be quick and side-effect
    /// free. A service that answers "I am broken" is a successful call
    /// returning [`HealthState::Degraded`] or [`HealthState::Down`]; the `Err`
    /// arm is for when the check itself could not be carried out.
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError>;

    /// Lists the operations this connector is willing to perform.
    ///
    /// Returned as data rather than compiled in so clients can build their
    /// controls from it dynamically. The list may legitimately be empty — a
    /// read-only connector that only reports health is valid — and it may vary
    /// over the connector's lifetime if the remote service's capabilities
    /// depend on its configuration or state.
    async fn actions(&self) -> Vec<ConnectorAction>;

    /// Performs the action named by `action_id`, passing `params` through.
    ///
    /// `params` is validated against the matching
    /// [`ConnectorAction::params_schema`]; an implementation that receives
    /// something it cannot use should say so with
    /// [`ConnectorError::InvalidParams`] rather than guessing. An `action_id`
    /// that is not in [`Connector::actions`] must produce
    /// [`ConnectorError::InvalidAction`].
    ///
    /// A returned [`ActionResult`] with `success: false` means the service
    /// understood the request and declined or failed it — which is different
    /// from an `Err`, meaning Loom could not get the request across at all.
    async fn execute_action(
        &self,
        action_id: &str,
        params: Value,
    ) -> Result<ActionResult, ConnectorError>;

    /// The JSON Schema for the configuration this connector needs.
    ///
    /// Two consumers rely on it: manifest loading, which validates a stored
    /// configuration before a connector is instantiated, and the frontends,
    /// which generate the setup form from it. Publishing a schema is what keeps
    /// "add a connector" from requiring a matching UI change in three clients.
    /// A connector that needs no configuration should return an empty schema
    /// object rather than `null`.
    fn config_schema(&self) -> Value;

    /// Identifying information for this connector: id, label, icon, version.
    ///
    /// Cheap and synchronous on purpose — it is used for registry keys, list
    /// rendering, and log lines, none of which should have to await a service.
    fn metadata(&self) -> ConnectorMetadata;

    /// Resolved values this connector is willing to have shown on the shell.
    ///
    /// Deliberately hand-written by the connector author rather than derived
    /// from [`Connector::config_schema`]: stored configuration is where
    /// credentials live, so anything automatic here would eventually put a
    /// token on a dashboard. Opting a field in is a decision someone makes on
    /// purpose, once, in code.
    ///
    /// Synchronous, so this must not reach out to the service. Values that
    /// require a round trip belong in [`ConnectorStatus::details`].
    fn display_fields(&self) -> Vec<DisplayField>;

    /// The data this instance can bind to a widget.
    ///
    /// Descriptors only — the readings themselves arrive in
    /// [`ConnectorStatus::details`] keyed by [`DataPointDescriptor::id`]. The
    /// split is what lets a saved dashboard survive: a layout stores ids, and
    /// each poll refreshes the values underneath them without the layout being
    /// re-derived.
    ///
    /// May legitimately be empty for a connector that only reports health.
    fn data_points(&self) -> Vec<DataPointDescriptor>;

    /// The widget arrangement this connector ships with.
    ///
    /// Used when a user adds the connector to a dashboard without configuring
    /// anything. It is a starting point the user then owns, not a constraint —
    /// nothing re-applies it after placement.
    fn default_layout(&self) -> WidgetLayout;
}

/// How a service is doing, as of a particular moment.
///
/// Pairs a coarse machine-comparable verdict ([`HealthState`], which is what
/// dashboards colour and alerts fire on) with an open `details` payload for
/// whatever that specific service considers interesting. The timestamp is part
/// of the value rather than the response envelope so a cached or polled status
/// stays honest about its own age.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorStatus {
    /// The coarse verdict clients act on.
    pub health: HealthState,
    /// The current reading for every data point, keyed by id.
    ///
    /// Typed as [`Value`] for serialization flexibility, but the shape is not
    /// free-form and consumers may rely on it: it **MUST** be a JSON object
    /// keyed by `data_point_id`, matching the ids returned by
    /// [`Connector::data_points`]. Each value's shape follows that data point's
    /// declared [`DataPointDescriptor::value_type`]:
    ///
    /// | `value_type` | JSON shape |
    /// | --- | --- |
    /// | [`Number`](DataPointValueType::Number) | a JSON number |
    /// | [`String`](DataPointValueType::String) | a JSON string |
    /// | [`Bool`](DataPointValueType::Bool) | a JSON boolean |
    /// | [`TimeSeries`](DataPointValueType::TimeSeries) | a JSON array of `{ "timestamp": <ISO 8601>, "value": <number> }` objects, oldest first |
    ///
    /// A connector may include extra keys that are not data points — a version
    /// string, a queue depth — and a client that does not recognise one ignores
    /// it. What it may not do is key a data point's value under anything but
    /// that data point's id, because a saved layout stores ids and resolves
    /// them here on every poll. [`ConnectorStatus::data_point_value`] is the
    /// intended way to read one.
    pub details: Value,
    /// When this reading was actually taken, so stale data is visible as stale.
    pub last_checked: DateTime<Utc>,
}

impl ConnectorStatus {
    /// A "nothing to report" status stamped with the current time.
    ///
    /// The common case for a connector whose check succeeded and which has no
    /// extra telemetry to attach.
    pub fn healthy() -> Self {
        Self::new(HealthState::Healthy, Value::Object(Default::default()))
    }

    /// A status with the given verdict and details, stamped with the current
    /// time.
    ///
    /// Use this rather than filling in `last_checked` by hand so that "now"
    /// always means the moment the reading was constructed.
    pub fn new(health: HealthState, details: Value) -> Self {
        Self {
            health,
            details,
            last_checked: Utc::now(),
        }
    }

    /// The current value of one data point, or `None` if this reading has no
    /// entry for it.
    ///
    /// A convenience over the [`ConnectorStatus::details`] object, and the
    /// place the keyed-object contract is enforced in practice: a `details`
    /// payload that is not an object yields `None` for every id rather than
    /// pretending to have found something. Callers still have to interpret the
    /// value against the data point's
    /// [`value_type`](DataPointDescriptor::value_type) — this says what is
    /// there, not what shape it should have been.
    pub fn data_point_value(&self, data_point_id: &str) -> Option<&Value> {
        self.details.as_object()?.get(data_point_id)
    }
}

/// The coarse health verdict for a service.
///
/// Kept to four cases on purpose: clients need to sort, colour, and alert on
/// this, which a free-form string cannot support. Serializes lowercase
/// (`"healthy"`, `"degraded"`, `"down"`, `"unknown"`) to match the TypeScript
/// union on the other side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HealthState {
    /// Reachable and behaving as expected.
    Healthy,
    /// Reachable but impaired — slow, partially failing, or self-reporting a
    /// problem. Worth surfacing, not worth waking someone up for.
    Degraded,
    /// Reachable-but-refusing or not answering at all. Something is broken.
    Down,
    /// No usable reading: never polled yet, or the service exposes nothing we
    /// can interpret. Distinct from [`HealthState::Down`] so a dashboard does
    /// not report an outage it has not actually observed.
    Unknown,
}

/// One operation a connector exposes, described well enough for a client to
/// render a control for it without knowing what the connector is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorAction {
    /// Stable identifier passed back to [`Connector::execute_action`].
    ///
    /// Machine-facing: it belongs in URLs and stored automations, so it should
    /// not change when the label does.
    pub id: String,
    /// Short human-facing name for the button or menu entry.
    pub label: String,
    /// Optional longer explanation, for tooltips and confirmation prompts —
    /// the place to warn that an action is disruptive.
    pub description: Option<String>,
    /// JSON Schema for this action's parameters, driving both client-side form
    /// generation and server-side validation.
    ///
    /// An action that takes no parameters uses an empty object rather than
    /// `null`, so consumers can always treat this as a schema.
    pub params_schema: Value,
}

impl ConnectorAction {
    /// A parameterless action with an id and a label.
    ///
    /// Covers the majority of real actions ("restart", "refresh") without
    /// making every call site spell out an empty schema.
    pub fn simple(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            description: None,
            params_schema: Value::Object(Default::default()),
        }
    }

    /// Attaches a description, for chaining onto [`ConnectorAction::simple`].
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// The outcome of an attempted action.
///
/// Distinct from `Result`: this type describes what the *service* did once the
/// request reached it. `success: false` is a normal, well-formed answer — the
/// service was asked and declined — whereas a [`ConnectorError`] means Loom
/// never got a verdict.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionResult {
    /// Whether the service carried the action out.
    pub success: bool,
    /// Human-readable summary, shown to the user verbatim. Should be
    /// meaningful on its own, since clients are not expected to map it.
    pub message: String,
    /// Optional structured result — a job id, the new state, a listing — for
    /// clients that want more than the message.
    pub payload: Option<Value>,
}

impl ActionResult {
    /// A successful result carrying only a message.
    pub fn ok(message: impl Into<String>) -> Self {
        Self {
            success: true,
            message: message.into(),
            payload: None,
        }
    }

    /// A failed-but-answered result: the service was reached and said no.
    ///
    /// Reach for this instead of [`ConnectorError`] when the failure is the
    /// service's decision rather than a transport problem.
    pub fn failed(message: impl Into<String>) -> Self {
        Self {
            success: false,
            message: message.into(),
            payload: None,
        }
    }

    /// Attaches a structured payload, for chaining onto the constructors above.
    #[must_use]
    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = Some(payload);
        self
    }
}

/// Identity and presentation for a connector.
///
/// This is what a client needs in order to list a connector before it has
/// talked to anything: what to call it, what to draw next to it, and which
/// revision of the connector produced the data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorMetadata {
    /// Stable machine identifier — the registry key and the URL segment.
    /// Lowercase kebab-case by convention (`"mock"`, `"reverse-proxy"`).
    pub id: String,
    /// Display name shown in the UI.
    pub name: String,
    /// Icon *reference*, not image data — a prefixed name the clients resolve
    /// against their own icon sets, so core never ships assets or assumes a
    /// renderer.
    ///
    /// The string, when present, takes one of exactly two forms:
    ///
    /// | Form | Resolves to | Example |
    /// | --- | --- | --- |
    /// | `"brand:<key>"` | A vendored brand SVG, `<key>` matching the vendored file's name without its extension. | `"brand:docker"` |
    /// | `"lucide:<name>"` | One icon from the client's curated generic set, `<name>` matching a `lucide-react` component in **kebab-case**. | `"lucide:hard-drive"` |
    ///
    /// Kebab-case for the `lucide:` form because that is the name lucide's own
    /// catalog and its `dynamicIconImports` map use; PascalCase is a detail of
    /// one binding's component export, and a wire format should not encode
    /// that.
    ///
    /// `None` means "no icon declared" and the client picks its own fallback.
    /// This is not a validated field: an unresolvable reference is a rendering
    /// concern, and a client that cannot find `"brand:whatever"` falls back
    /// rather than failing. Core deliberately does not police it, because core
    /// does not know what any client has vendored.
    ///
    /// See `docs/THIRD_PARTY_ICONS.md` for what is vendored and under which
    /// license.
    pub icon: Option<String>,
    /// Version of the connector implementation itself, independent of the Loom
    /// release, so a connector can be revised without a platform bump.
    pub version: String,
    /// Smallest useful footprint on the dashboard grid, as `(width, height)`
    /// in grid units.
    ///
    /// A floor the placement UI enforces, not a preferred size: a connector
    /// that needs a chart to be readable says so here instead of being shrunk
    /// into illegibility by a user who does not yet know what it draws. `u8`
    /// because a grid that needs more than 255 units in a direction is not a
    /// grid.
    pub min_size: (u8, u8),
}

/// One resolved, currently-true fact about a connector instance that is
/// explicitly safe to put on screen.
///
/// This exists because the obvious alternative is unsafe: deriving what to show
/// from [`Connector::config_schema`] would put whatever is in the stored
/// configuration on the shell, and stored configuration is exactly where API
/// tokens and passwords live. So nothing is ever derived automatically — a
/// connector author writes out the fields they want visible, one by one, and a
/// field that is not written out is not shown. The values are already resolved
/// strings rather than keys into anything, so the clients need no formatting
/// rules and no per-connector display code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplayField {
    /// Short caption, e.g. `"Host"` or `"Version"`.
    pub label: String,
    /// The value as it should appear, already rendered to text.
    pub value: String,
}

impl DisplayField {
    /// A field with a label and a value.
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

/// The shape of a data point's values, so a client knows what it is binding.
///
/// Coarse on purpose: this decides which widgets a data point can legally drive
/// (a `Bool` cannot fill a chart; a `TimeSeries` cannot fill a status dot), and
/// a finer type system here would constrain connector authors without telling
/// the renderer anything more.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DataPointValueType {
    /// A single numeric reading.
    Number,
    /// A single text value.
    String,
    /// A single on/off flag.
    Bool,
    /// An ordered run of recent numeric readings, oldest first.
    TimeSeries,
}

/// One piece of data an instance can expose to a widget.
///
/// A *descriptor*, not a reading: it says what exists and what it means, and
/// the current values arrive separately in
/// [`ConnectorStatus::details`], keyed by [`DataPointDescriptor::id`]. Keeping
/// them apart is what lets a dashboard be laid out once and re-rendered on
/// every poll without re-reading the schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataPointDescriptor {
    /// Stable machine identifier, and the key this data point's value appears
    /// under in [`ConnectorStatus::details`]. Stored in saved layouts, so it
    /// must not change when the label does.
    pub id: String,
    /// Human-facing name for the widget's caption or legend entry.
    pub label: String,
    /// What kind of value this is, which constrains the widgets it can drive.
    pub value_type: DataPointValueType,
    /// Unit suffix (`"%"`, `"MiB"`, `"ms"`), or `None` for a dimensionless
    /// value. A *display* concern only: the value itself is never scaled.
    pub unit: Option<String>,
}

impl DataPointDescriptor {
    /// A descriptor with no unit.
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        value_type: DataPointValueType,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            value_type,
            unit: None,
        }
    }

    /// Attaches a unit, for chaining onto [`DataPointDescriptor::new`].
    #[must_use]
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }
}

/// Which plot a [`DisplayWidgetType::MetricChart`] draws.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartType {
    /// Proportions of a whole.
    Pie,
    /// Discrete categories side by side.
    Bar,
    /// A value over time.
    Line,
}

/// How a data point is drawn.
///
/// A closed enum rather than a free-form string because the clients have to
/// render each case, and an unrecognised widget type is a blank space in a
/// dashboard with no way for the user to work out why. Adding a widget is
/// therefore deliberately a change to this crate and to every renderer, which
/// is the honest cost of the guarantee that everything here draws.
///
/// Read-only: every variant here shows a [`DataPointDescriptor`] and invokes
/// nothing. The controls live in [`ActionWidgetType`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum DisplayWidgetType {
    /// One prominent number with its label.
    StatTile,
    /// A filled bar for a bounded value; `config` supplies `min`/`max`.
    ProgressBar,
    /// A plot of a [`DataPointValueType::TimeSeries`] or of several numbers.
    MetricChart {
        /// Which plot to draw.
        chart_type: ChartType,
    },
    /// A dial for a bounded value; `config` supplies `min`/`max`.
    Gauge,
    /// A coloured dot for a boolean or a health state.
    StatusDot,
    /// A scrolling run of text lines.
    LogStream,
}

/// How an action is offered.
///
/// The counterpart to [`DisplayWidgetType`], and closed for the same reason.
/// Every variant here invokes [`Connector::execute_action`]; which parameters
/// it sends is the renderer's business, guided by the action's
/// [`ConnectorAction::params_schema`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ActionWidgetType {
    /// A button that triggers a parameterless action.
    Button,
    /// A switch that triggers an action taking a boolean.
    Toggle,
    /// A slider that triggers an action taking a number; `config` supplies
    /// `min`/`max`/`step`.
    Slider,
    /// A text input that triggers an action taking a string.
    TextField,
    /// A dropdown that triggers an action taking one of a set of values;
    /// `config` supplies `options`.
    Selector,
}

/// One widget, and the thing it is wired to.
///
/// Split by what the widget binds *to*, not by how it looks: a display widget
/// reads a [`DataPointDescriptor`], a control widget invokes a
/// [`ConnectorAction`], and those are different identifier spaces that happen
/// to both be strings. The earlier flat shape carried one `data_point_id` field
/// for both, which meant a control binding either named a data point that could
/// not be invoked or a live action id that no validator could check — see
/// `docs/adr/0014-widget-binding-model.md`.
///
/// Externally tagged, so each value is a single-key object (`{"display": …}`
/// or `{"action": …}`) — the same shape as [`ConnectorError`] and
/// [`DisplayWidgetType::MetricChart`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum WidgetBinding {
    /// A read-only widget showing one data point.
    Display {
        /// Which [`DataPointDescriptor::id`] this widget shows. Its value
        /// arrives in [`ConnectorStatus::details`] under this same key.
        data_point_id: String,
        /// How to draw it.
        widget_type: DisplayWidgetType,
        /// Widget-specific extras: `min`/`max` for a
        /// [`DisplayWidgetType::Gauge`], and so on.
        ///
        /// Free-form for now, and deliberately so. Every candidate typed
        /// representation would have to enumerate the settings of every widget
        /// in one struct before the widgets themselves have been built against
        /// real connectors, which is the wrong order to design in. A binding
        /// that needs no extras uses an empty object rather than `null`, so
        /// consumers can always treat this as an object.
        config: Value,
    },
    /// A control that invokes one action.
    Action {
        /// Which [`ConnectorAction::id`] this widget invokes, as passed to
        /// [`Connector::execute_action`].
        action_id: String,
        /// How to offer it.
        widget_type: ActionWidgetType,
        /// Widget-specific extras: `options` for an
        /// [`ActionWidgetType::Selector`], `min`/`max`/`step` for a
        /// [`ActionWidgetType::Slider`]. Same free-form contract as the display
        /// arm's `config`.
        config: Value,
    },
}

impl WidgetBinding {
    /// A display binding with no extra configuration.
    pub fn display(data_point_id: impl Into<String>, widget_type: DisplayWidgetType) -> Self {
        Self::Display {
            data_point_id: data_point_id.into(),
            widget_type,
            config: Value::Object(Default::default()),
        }
    }

    /// An action binding with no extra configuration.
    pub fn action(action_id: impl Into<String>, widget_type: ActionWidgetType) -> Self {
        Self::Action {
            action_id: action_id.into(),
            widget_type,
            config: Value::Object(Default::default()),
        }
    }

    /// Attaches widget-specific configuration, for chaining onto
    /// [`WidgetBinding::display`] or [`WidgetBinding::action`].
    #[must_use]
    pub fn with_config(mut self, config: Value) -> Self {
        match &mut self {
            Self::Display { config: slot, .. } | Self::Action { config: slot, .. } => {
                *slot = config;
            }
        }
        self
    }
}

/// The arrangement of widgets a connector ships with.
///
/// A *default*, not a mandate: it is what a user gets when they add the
/// connector and place it without configuring anything, and it is theirs to
/// edit afterwards. Ordering is the connector author's suggested reading order;
/// there are no coordinates here, because where a widget sits on a grid is the
/// dashboard's business and not the connector's.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WidgetLayout {
    /// The widgets, in the order the connector author suggests showing them.
    /// May be empty for a connector that exposes nothing to draw.
    pub bindings: Vec<WidgetBinding>,
}

impl WidgetLayout {
    /// A layout holding exactly these bindings.
    pub fn new(bindings: Vec<WidgetBinding>) -> Self {
        Self { bindings }
    }
}

/// Why a connector could not answer.
///
/// Reserved for failures of the *interaction*, not of the managed service: a
/// service reporting its own bad state is a successful [`ConnectorStatus`], and
/// a service refusing an action is an [`ActionResult`] with `success: false`.
/// Mixing those into this enum would make it impossible for a client to tell
/// "Loom is misconfigured" from "your server is unhappy".
///
/// Three properties are load-bearing beyond `Error`:
///
/// - `Clone`, so a stored error (notably [`debug::DebugConnector`]'s fail mode)
///   can be handed out repeatedly.
/// - `Serialize`/`Deserialize`, so the backend can forward the discriminant to
///   the clients instead of flattening everything to a string. This works
///   because every variant payload is a plain `String`; the derived
///   representation is externally tagged, giving objects like
///   `{"invalidAction":{"actionId":"nope"}}` and `{"internal":"…"}` (variant tags
///   and their fields both `camelCase`). Any future
///   variant that needs a non-serializable payload (an I/O error, a
///   `Box<dyn Error>`) must be reduced to a string here rather than breaking
///   this property.
/// - No `#[source]` anywhere, for the same reason — a chained cause would not
///   survive the wire.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ConnectorError {
    /// The service could not be contacted: refused, timed out, DNS failure.
    /// The user's likely fix is at the infrastructure level, not in Loom.
    #[error("service is unreachable: {reason}")]
    Unreachable {
        /// What went wrong, in terms a user can act on.
        reason: String,
    },

    /// The service was reached but rejected our credentials, so the stored
    /// configuration needs attention. Kept separate from `Unreachable` because
    /// the remedy is completely different.
    #[error("authentication with the service failed: {reason}")]
    AuthFailed {
        /// What the service objected to, with secrets left out.
        reason: String,
    },

    /// The requested action id is not one this connector exposes — usually a
    /// stale client or an automation referring to a removed action.
    #[error("unknown action id: {action_id}")]
    InvalidAction {
        /// The id that was asked for, echoed back for the log line.
        action_id: String,
    },

    /// The action exists but the parameters do not satisfy its schema.
    #[error("invalid parameters for action {action_id}: {reason}")]
    InvalidParams {
        /// The action whose parameters were rejected.
        action_id: String,
        /// Which constraint failed, so the client can point at the field.
        reason: String,
    },

    /// The stored configuration is not something this connector can be built
    /// from — an unknown key, a missing one, a value out of range.
    ///
    /// Distinct from [`ConnectorError::InvalidParams`], which is about the
    /// arguments to one action. This one is about the connector itself, and it
    /// is what a factory returns when it refuses to construct an instance, so
    /// the backend can answer a bad "add connector" request with the
    /// connector's own objection instead of a generic rejection.
    #[error("invalid connector configuration: {reason}")]
    InvalidConfig {
        /// Which part of the configuration was unusable, in terms a user can
        /// act on, with any supplied secret left out.
        reason: String,
    },

    /// Anything else went wrong inside the connector — a bug, an unexpected
    /// response shape, a failed parse. The catch-all exists so implementors are
    /// never tempted to panic; it should not be the answer to a foreseeable
    /// condition.
    #[error("connector failed internally: {0}")]
    Internal(String),
}

impl ConnectorError {
    /// Shorthand for [`ConnectorError::Unreachable`].
    pub fn unreachable(reason: impl Into<String>) -> Self {
        Self::Unreachable {
            reason: reason.into(),
        }
    }

    /// Shorthand for [`ConnectorError::InvalidAction`].
    pub fn invalid_action(action_id: impl Into<String>) -> Self {
        Self::InvalidAction {
            action_id: action_id.into(),
        }
    }

    /// Shorthand for [`ConnectorError::InvalidConfig`].
    pub fn invalid_config(reason: impl Into<String>) -> Self {
        Self::InvalidConfig {
            reason: reason.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use serde_json::json;

    /// A minimal in-test implementation, separate from [`debug::DebugConnector`]
    /// on purpose: it proves the trait alone is enough to write a connector,
    /// with no help from the fixture's machinery.
    struct StubConnector {
        id: &'static str,
        health: HealthState,
    }

    #[async_trait]
    impl Connector for StubConnector {
        async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
            // Keyed by this stub's one data point id, per the
            // `ConnectorStatus::details` contract.
            Ok(ConnectorStatus::new(
                self.health,
                json!({ "reading": 42.0 }),
            ))
        }

        async fn actions(&self) -> Vec<ConnectorAction> {
            vec![ConnectorAction::simple("noop", "Do nothing")]
        }

        async fn execute_action(
            &self,
            action_id: &str,
            _params: Value,
        ) -> Result<ActionResult, ConnectorError> {
            if action_id == "noop" {
                Ok(ActionResult::ok("did nothing"))
            } else {
                Err(ConnectorError::invalid_action(action_id))
            }
        }

        fn config_schema(&self) -> Value {
            json!({ "type": "object", "properties": {} })
        }

        fn metadata(&self) -> ConnectorMetadata {
            ConnectorMetadata {
                id: self.id.to_string(),
                name: "Stub".to_string(),
                icon: None,
                version: "0.0.1".to_string(),
                min_size: (1, 1),
            }
        }

        fn display_fields(&self) -> Vec<DisplayField> {
            vec![DisplayField::new("Kind", "stub")]
        }

        fn data_points(&self) -> Vec<DataPointDescriptor> {
            vec![
                DataPointDescriptor::new("reading", "Reading", DataPointValueType::Number)
                    .with_unit("%"),
            ]
        }

        fn default_layout(&self) -> WidgetLayout {
            WidgetLayout::new(vec![
                WidgetBinding::display("reading", DisplayWidgetType::StatTile),
                WidgetBinding::action("noop", ActionWidgetType::Button),
            ])
        }
    }

    #[tokio::test]
    async fn trait_is_object_safe_and_works_in_a_heterogeneous_collection() {
        let connectors: Vec<Box<dyn Connector>> = vec![
            Box::new(StubConnector {
                id: "stub-a",
                health: HealthState::Healthy,
            }),
            Box::new(StubConnector {
                id: "stub-b",
                health: HealthState::Degraded,
            }),
            Box::new(debug::DebugConnector::default()),
        ];

        assert_eq!(connectors.len(), 3);

        let ids: Vec<String> = connectors.iter().map(|c| c.metadata().id).collect();
        assert_eq!(ids, vec!["stub-a", "stub-b", "debug"]);

        for connector in &connectors {
            let status = connector.status().await.expect("status should succeed");
            assert_ne!(status.health, HealthState::Unknown);
            assert!(connector.config_schema().is_object());
            assert!(!connector.actions().await.is_empty());

            // The presentation half of the trait, exercised through the trait
            // object: these are the methods a dashboard calls, and they have to
            // stay callable without knowing the concrete type.
            let metadata = connector.metadata();
            assert!(metadata.min_size.0 >= 1 && metadata.min_size.1 >= 1);

            assert!(!connector.display_fields().is_empty());

            let data_points = connector.data_points();
            assert!(!data_points.is_empty());

            // `details` is a data-point-keyed object, not a free-form blob:
            // every declared data point has to resolve to a value in the same
            // reading, because that is what a widget binding looks up.
            for point in &data_points {
                assert!(
                    status.data_point_value(&point.id).is_some(),
                    "status details is missing data point {}",
                    point.id
                );
            }

            // Every binding in the shipped layout must name something that
            // actually exists, or the connector ships a dashboard with a hole
            // in it. Which namespace to check is exactly what the enum split
            // buys: a display binding resolves against the data points, an
            // action binding against the action list.
            let point_ids: Vec<&str> = data_points.iter().map(|dp| dp.id.as_str()).collect();
            let actions = connector.actions().await;
            let action_ids: Vec<&str> = actions.iter().map(|a| a.id.as_str()).collect();
            for binding in connector.default_layout().bindings {
                match &binding {
                    WidgetBinding::Display { data_point_id, .. } => assert!(
                        point_ids.contains(&data_point_id.as_str()),
                        "layout binds unknown data point {data_point_id}"
                    ),
                    WidgetBinding::Action { action_id, .. } => assert!(
                        action_ids.contains(&action_id.as_str()),
                        "layout binds unknown action {action_id}"
                    ),
                }
            }
        }

        assert_eq!(
            connectors[0]
                .execute_action("noop", Value::Null)
                .await
                .expect("noop should succeed")
                .message,
            "did nothing"
        );
        assert_eq!(
            connectors[1]
                .execute_action("nope", Value::Null)
                .await
                .expect_err("unknown action should fail"),
            ConnectorError::invalid_action("nope")
        );
    }

    #[test]
    fn connector_status_serializes_with_camel_case_keys() {
        let status = ConnectorStatus {
            health: HealthState::Degraded,
            details: json!({ "queueDepth": 12 }),
            last_checked: Utc.with_ymd_and_hms(2026, 8, 19, 12, 0, 0).unwrap(),
        };

        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(
            value,
            json!({
                "health": "degraded",
                "details": { "queueDepth": 12 },
                "lastChecked": "2026-08-19T12:00:00Z"
            })
        );
        assert_eq!(
            serde_json::from_value::<ConnectorStatus>(value).unwrap(),
            status
        );
    }

    #[test]
    fn health_state_variants_serialize_lowercase() {
        for (state, expected) in [
            (HealthState::Healthy, "healthy"),
            (HealthState::Degraded, "degraded"),
            (HealthState::Down, "down"),
            (HealthState::Unknown, "unknown"),
        ] {
            assert_eq!(serde_json::to_value(state).unwrap(), json!(expected));
        }
    }

    #[test]
    fn connector_action_serializes_with_camel_case_keys() {
        let action = ConnectorAction::simple("restart", "Restart").with_description("Restarts it.");

        let value = serde_json::to_value(&action).unwrap();
        assert_eq!(
            value,
            json!({
                "id": "restart",
                "label": "Restart",
                "description": "Restarts it.",
                "paramsSchema": {}
            })
        );
        assert_eq!(
            serde_json::from_value::<ConnectorAction>(value).unwrap(),
            action
        );
    }

    #[test]
    fn action_result_serializes_with_camel_case_keys() {
        let result = ActionResult::ok("restarted").with_payload(json!({ "jobId": "abc" }));

        let value = serde_json::to_value(&result).unwrap();
        assert_eq!(
            value,
            json!({
                "success": true,
                "message": "restarted",
                "payload": { "jobId": "abc" }
            })
        );
        assert_eq!(
            serde_json::from_value::<ActionResult>(value).unwrap(),
            result
        );
    }

    #[test]
    fn connector_metadata_serializes_with_camel_case_keys() {
        let metadata = ConnectorMetadata {
            id: "debug".to_string(),
            name: "Debug Connector".to_string(),
            icon: Some("lucide:beaker".to_string()),
            version: "1.0.0".to_string(),
            min_size: (2, 2),
        };

        let value = serde_json::to_value(&metadata).unwrap();
        assert_eq!(
            value,
            json!({
                "id": "debug",
                "name": "Debug Connector",
                "icon": "lucide:beaker",
                "version": "1.0.0",
                "minSize": [2, 2]
            })
        );
        assert_eq!(
            serde_json::from_value::<ConnectorMetadata>(value).unwrap(),
            metadata
        );
    }

    #[test]
    fn connector_error_serializes_with_camel_case_tags_and_round_trips() {
        let cases = [
            (
                ConnectorError::unreachable("connection refused"),
                json!({ "unreachable": { "reason": "connection refused" } }),
            ),
            (
                ConnectorError::AuthFailed {
                    reason: "token rejected".to_string(),
                },
                json!({ "authFailed": { "reason": "token rejected" } }),
            ),
            (
                ConnectorError::invalid_action("nope"),
                json!({ "invalidAction": { "actionId": "nope" } }),
            ),
            (
                ConnectorError::InvalidParams {
                    action_id: "restart".to_string(),
                    reason: "missing field `force`".to_string(),
                },
                json!({ "invalidParams": {
                    "actionId": "restart",
                    "reason": "missing field `force`"
                } }),
            ),
            (
                ConnectorError::invalid_config("unknown field `wat`"),
                json!({ "invalidConfig": { "reason": "unknown field `wat`" } }),
            ),
            (
                ConnectorError::Internal("unexpected response shape".to_string()),
                json!({ "internal": "unexpected response shape" }),
            ),
        ];

        for (error, expected) in cases {
            let value = serde_json::to_value(&error).unwrap();
            assert_eq!(value, expected, "unexpected shape for {error:?}");
            assert_eq!(
                serde_json::from_value::<ConnectorError>(value).unwrap(),
                error
            );
        }
    }

    #[test]
    fn connector_error_display_messages_are_useful() {
        assert_eq!(
            ConnectorError::invalid_action("nope").to_string(),
            "unknown action id: nope"
        );
        assert_eq!(
            ConnectorError::Internal("boom".to_string()).to_string(),
            "connector failed internally: boom"
        );
    }

    /// Prints the canonical wire shapes. Not an assertion — it exists so the
    /// exact JSON the clients see can be read off a test run rather than
    /// reconstructed from the type definitions. Run with
    /// `cargo test -p loom-core -- --nocapture print_wire_shapes`.
    #[test]
    fn widget_bindings_serialize_as_externally_tagged_variants() {
        assert_eq!(
            serde_json::to_value(WidgetBinding::display(
                "loadHistory",
                DisplayWidgetType::MetricChart {
                    chart_type: ChartType::Line
                }
            ))
            .unwrap(),
            json!({
                "display": {
                    "dataPointId": "loadHistory",
                    "widgetType": { "metricChart": { "chartType": "line" } },
                    "config": {}
                }
            })
        );
        assert_eq!(
            serde_json::to_value(
                WidgetBinding::action("set-load", ActionWidgetType::Slider)
                    .with_config(json!({ "min": 0, "max": 100 }))
            )
            .unwrap(),
            json!({
                "action": {
                    "actionId": "set-load",
                    "widgetType": "slider",
                    "config": { "min": 0, "max": 100 }
                }
            })
        );
    }

    #[test]
    fn widget_bindings_round_trip_through_json() {
        let layout = WidgetLayout::new(vec![
            WidgetBinding::display("load", DisplayWidgetType::Gauge)
                .with_config(json!({ "min": 0, "max": 100 })),
            WidgetBinding::action("restart", ActionWidgetType::Button),
        ]);
        let encoded = serde_json::to_string(&layout).unwrap();
        assert_eq!(
            serde_json::from_str::<WidgetLayout>(&encoded).unwrap(),
            layout
        );
    }

    #[test]
    fn data_point_value_reads_the_keyed_details_object() {
        let status = ConnectorStatus::new(
            HealthState::Healthy,
            json!({ "load": 12.5, "enabled": true }),
        );
        assert_eq!(status.data_point_value("load"), Some(&json!(12.5)));
        assert_eq!(status.data_point_value("enabled"), Some(&json!(true)));
        assert_eq!(status.data_point_value("missing"), None);

        // A `details` payload that is not an object breaks the contract; the
        // accessor reports "nothing there" rather than inventing a reading.
        let malformed = ConnectorStatus::new(HealthState::Unknown, json!("not an object"));
        assert_eq!(malformed.data_point_value("load"), None);
    }

    #[test]
    fn print_wire_shapes() {
        let last_checked = Utc.with_ymd_and_hms(2026, 8, 19, 12, 0, 0).unwrap();
        let status = ConnectorStatus {
            health: HealthState::Degraded,
            details: json!({ "version": "1.2.3", "queueDepth": 12 }),
            last_checked,
        };
        println!(
            "ConnectorStatus =\n{}",
            serde_json::to_string_pretty(&status).unwrap()
        );
        println!(
            "ConnectorAction =\n{}",
            serde_json::to_string_pretty(
                &ConnectorAction::simple("restart", "Restart")
                    .with_description("Restarts the service.")
            )
            .unwrap()
        );
        println!(
            "ActionResult =\n{}",
            serde_json::to_string_pretty(
                &ActionResult::ok("restart requested").with_payload(json!({ "jobId": "abc123" }))
            )
            .unwrap()
        );
        println!(
            "ConnectorMetadata =\n{}",
            serde_json::to_string_pretty(&ConnectorMetadata {
                id: "debug".to_string(),
                name: "Debug Connector".to_string(),
                icon: Some("lucide:beaker".to_string()),
                version: "1.0.0".to_string(),
                min_size: (2, 2),
            })
            .unwrap()
        );
        println!(
            "WidgetLayout =\n{}",
            serde_json::to_string_pretty(&WidgetLayout::new(vec![
                WidgetBinding::display("load", DisplayWidgetType::StatTile),
                WidgetBinding::display(
                    "loadHistory",
                    DisplayWidgetType::MetricChart {
                        chart_type: ChartType::Line
                    }
                ),
                WidgetBinding::display("load", DisplayWidgetType::Gauge)
                    .with_config(json!({ "min": 0, "max": 100 })),
                WidgetBinding::action("set-enabled", ActionWidgetType::Toggle),
            ]))
            .unwrap()
        );
        println!(
            "DataPointDescriptor =\n{}",
            serde_json::to_string_pretty(
                &DataPointDescriptor::new("load", "Load", DataPointValueType::Number)
                    .with_unit("%")
            )
            .unwrap()
        );
        println!(
            "DisplayField =\n{}",
            serde_json::to_string_pretty(&DisplayField::new("Host", "debug.invalid")).unwrap()
        );
        for error in [
            ConnectorError::unreachable("connection refused"),
            ConnectorError::AuthFailed {
                reason: "token rejected".to_string(),
            },
            ConnectorError::invalid_action("nope"),
            ConnectorError::InvalidParams {
                action_id: "restart".to_string(),
                reason: "missing field `force`".to_string(),
            },
            ConnectorError::invalid_config("unknown field `wat`"),
            ConnectorError::Internal("unexpected response shape".to_string()),
        ] {
            println!(
                "ConnectorError =\n{}",
                serde_json::to_string_pretty(&error).unwrap()
            );
        }
    }
}
