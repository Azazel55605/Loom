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
use std::collections::HashMap;

pub mod debug;
pub mod details;

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

    /// Performs a lightweight reachability and capability check during setup.
    ///
    /// This is distinct from the recurring [`Connector::status`] poll: setup
    /// clients call it explicitly for a candidate configuration before an
    /// instance exists. The default preserves useful behaviour for connectors
    /// that do not publish capability detail by mapping the ordinary health
    /// check to reachability and returning no capability rows.
    async fn test_connection(&self) -> ConnectionTestResult {
        match self.status().await {
            Ok(status) => ConnectionTestResult {
                reachable: matches!(status.health, HealthState::Healthy | HealthState::Degraded),
                capabilities: Vec::new(),
                message: None,
            },
            Err(error) => ConnectionTestResult {
                reachable: false,
                capabilities: Vec::new(),
                message: Some(error.to_string()),
            },
        }
    }

    /// Lists the operations this connector is willing to perform.
    ///
    /// Returned as data rather than compiled in so clients can build their
    /// controls from it dynamically. The list may legitimately be empty — a
    /// read-only connector that only reports health is valid — and it may vary
    /// over the connector's lifetime if the remote service's capabilities
    /// depend on its configuration or state.
    async fn actions(&self) -> Vec<ConnectorAction>;

    /// Performs the action named by `action_id` for the optional sub-target,
    /// passing `params` through.
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
        target_id: Option<&str>,
        params: Value,
    ) -> Result<ActionResult, ConnectorError>;

    /// Whether this connector exposes addressable views below the instance.
    fn supports_sub_targets(&self) -> bool {
        false
    }

    /// Cheaply enumerates addressable views within this connector instance.
    ///
    /// This returns names/labels only: detailed readings still come from
    /// [`Connector::status`]. It is deliberately distinct from
    /// [`Connector::discover`], which proposes configurations for creating
    /// whole new connector instances. `target_id: None` names the connector's
    /// host/aggregate action; `Some(id)` must match the action descriptor. A
    /// sub-target remains inside this
    /// instance and shares its connection and permission boundary.
    async fn list_sub_targets(&self) -> Result<Vec<SubTarget>, ConnectorError> {
        Ok(Vec::new())
    }

    /// The kinds of resource this connector can list as a browsable table.
    ///
    /// Descriptors only — the rows themselves come from
    /// [`Connector::list_resource_items`], the same split as
    /// [`Connector::data_points`] and [`ConnectorStatus::details`]. A client
    /// renders a table from the columns without knowing what the connector is,
    /// and offers the declared actions beside it.
    ///
    /// Empty by default, and legitimately empty for most connectors: a resource
    /// kind is for things a service has *many* of and a user browses through —
    /// images, volumes, backups — not for the service's own readings, which are
    /// data points. See `docs/adr/0021-connector-resource-browser.md`.
    ///
    /// Cheap and synchronous, like the other descriptor methods: a client asks
    /// what can be browsed before it asks for any rows.
    ///
    /// # Why this takes a target
    ///
    /// `target_id` is which view is being looked at — `None` for the instance
    /// as a whole, otherwise a [`SubTarget::id`] — and it exists so a kind can
    /// be **absent** rather than merely empty. [`ApplicableTarget`] already
    /// lets a descriptor say *where* it belongs, and that is enough while every
    /// target of a connector is the same sort of thing. It stops being enough
    /// when they are not: Docker's stacks and its containers are both
    /// sub-targets, and "the containers in this stack" is a table one of them
    /// has and the other does not. `TargetOnly` cannot express that, because a
    /// container is a target too.
    ///
    /// A connector that does not care ignores the argument, which is what the
    /// default body and every existing implementation do.
    fn resource_kinds(&self, target_id: Option<&str>) -> Vec<ResourceKindDescriptor> {
        let _ = target_id;
        Vec::new()
    }

    /// Lists the current rows of one resource kind.
    ///
    /// `kind` matches a [`ResourceKindDescriptor::kind`] this connector
    /// published; `target_id` scopes the listing to a sub-target
    /// ([`Connector::list_sub_targets`]) when the connector has them, and
    /// `None` means the instance as a whole.
    ///
    /// An unrecognised `kind` yields an **empty list, not an error** — that is
    /// what the default implementation returns for every kind, and a connector
    /// that overrides this should behave the same way, so "this connector does
    /// not have that kind" reads identically whether or not it browses anything
    /// at all. Callers that need to distinguish the two compare against
    /// [`Connector::resource_kinds`], which is the authoritative list. The
    /// `Err` arm stays reserved for what it means everywhere else in this
    /// trait: the listing could not be carried out.
    async fn list_resource_items(
        &self,
        _kind: &str,
        _target_id: Option<&str>,
    ) -> Result<Vec<ResourceItem>, ConnectorError> {
        Ok(Vec::new())
    }

    /// Whether this connector can say if the thing it manages is out of date.
    ///
    /// Declared separately from [`Connector::check_for_updates`] so a client
    /// can decide whether to offer the control at all, rather than offering it
    /// everywhere and discovering per instance that the answer is always "no
    /// update". The two must agree: a connector returning `true` here is
    /// expected to override the check.
    fn supports_update_checking(&self) -> bool {
        false
    }

    /// Asks whether a newer version of the managed thing exists.
    ///
    /// `target_id` scopes the question to one sub-target — for a connector with
    /// many, "is this one out of date?" is the question people actually ask —
    /// and `None` asks about the instance as a whole.
    ///
    /// Read-only and non-committal: this reports what is available and never
    /// applies anything. Whatever *acts* on the answer is an ordinary
    /// [`ConnectorAction`], which is what puts it behind `connectors.control`
    /// and into the audit log, where an upgrade belongs.
    ///
    /// The default answers "nothing available" rather than erroring, so a
    /// caller that asks a connector which does not support checking gets a
    /// usable answer instead of an exception to handle. The `Err` arm keeps its
    /// usual meaning: the check itself could not be carried out — the registry
    /// was unreachable, the credentials were refused.
    async fn check_for_updates(
        &self,
        _target_id: Option<&str>,
    ) -> Result<UpdateCheckResult, ConnectorError> {
        Ok(UpdateCheckResult {
            available: false,
            latest_ref: None,
        })
    }

    /// The JSON Schema for the configuration this connector needs.
    ///
    /// Two consumers rely on it: manifest loading, which validates a stored
    /// configuration before a connector is instantiated, and the frontends,
    /// which generate the setup form from it. Publishing a schema is what keeps
    /// "add a connector" from requiring a matching UI change in three clients.
    /// A connector that needs no configuration should return an empty schema
    /// object rather than `null`.
    fn config_schema(&self) -> Value;

    /// The connector type this live instance can discover child resources for.
    ///
    /// Discovery is instance-scoped because it commonly needs an already
    /// configured connection. It proposes whole new connector instances and is
    /// deliberately distinct from [`Connector::list_sub_targets`], which names
    /// addressable views inside this same instance. `None` means this connector
    /// does not support discovery.
    fn discoverable_type(&self) -> Option<String> {
        None
    }

    /// Configuration field a discovery result can fill during setup.
    ///
    /// This is independent of [`Connector::discoverable_type`]: a connector
    /// built from a complete configuration may have nothing further to
    /// discover while its type can still use discovery to fill one field in a
    /// candidate configuration.
    fn discovery_target_field(&self) -> Option<String> {
        None
    }

    /// Finds resources reachable through this configured connector.
    ///
    /// Implementations return suggested configurations; they never create
    /// connector instances themselves. The backend remains responsible for
    /// authorization and persistence. Connectors opt in by overriding this
    /// method together with [`Connector::discoverable_type`]. This is not
    /// sub-target enumeration: discovery proposes whole new connector
    /// instances, while [`Connector::list_sub_targets`] names addressable views
    /// that remain within this instance.
    async fn discover(&self) -> Result<Vec<DiscoveredResource>, ConnectorError> {
        Ok(Vec::new())
    }

    /// Descriptive, client-rendered ways to configure this connector type.
    ///
    /// Each variant is an independent setup path. Its template may contain
    /// placeholders for schema fields and for its UI-only toggles. Toggles are
    /// never part of the connector configuration and are never persisted;
    /// clients use them only to render live setup text and derive declarative
    /// capability availability. A capability requirement uses AND-only logic:
    /// every listed toggle key must be enabled. This deliberate v1 constraint
    /// should be expanded only when a real connector requires OR logic.
    /// Core performs no substitution or requirement evaluation.
    fn setup_guide(&self) -> Option<SetupGuide> {
        None
    }

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

    /// The starting widget arrangement for one connector-level or sub-target
    /// view. Connectors without sub-targets inherit their existing layout.
    fn default_layout_for(&self, _target_id: Option<&str>) -> WidgetLayout {
        self.default_layout()
    }

    /// The host and port a *network-level* probe should try when this connector
    /// stops answering, if probing one would mean anything.
    ///
    /// Purely descriptive: core neither resolves nor connects to it. This
    /// answers "if I am broken, which endpoint is worth checking?", and the
    /// platform decides whether and how to check, because reaching out to a
    /// network is exactly the sort of thing core does not do.
    ///
    /// `None` means "no useful probe", and that is the honest answer more often
    /// than it looks: a connector reaching a Unix socket, an in-process
    /// fixture, or a service behind a path on a host it shares has no host and
    /// port whose reachability would tell a user anything they did not already
    /// know.
    ///
    /// Returning a target buys one concrete thing — the difference between
    /// "your DNS name does not resolve", "the host is not answering on that
    /// port", and "the host is fine, the service is not", which are three
    /// different afternoons.
    fn network_target(&self) -> Option<NetworkTarget> {
        None
    }
}

/// The answer to "is what this connector manages out of date?".
///
/// Deliberately two fields and no more. `available` is what a client renders a
/// badge from, and `latest_ref` is the connector's own name for what it found —
/// an image digest, a release tag, a version string — carried as opaque text
/// because Loom has no business parsing another ecosystem's version scheme. A
/// structured "current vs. latest" comparison would require every connector to
/// agree on what a version *is*, which they do not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheckResult {
    /// Whether something newer than what is running exists.
    pub available: bool,
    /// What the newer thing is called, in the managed system's own terms, or
    /// `None` when nothing newer was found or the connector cannot name it.
    pub latest_ref: Option<String>,
}

impl UpdateCheckResult {
    /// "Nothing newer" — the answer most checks give most of the time.
    pub fn up_to_date() -> Self {
        Self {
            available: false,
            latest_ref: None,
        }
    }

    /// "Something newer exists", named in the managed system's own terms.
    pub fn available(latest_ref: impl Into<String>) -> Self {
        Self {
            available: true,
            latest_ref: Some(latest_ref.into()),
        }
    }
}

/// Where to aim a network-level reachability probe.
///
/// A host and optionally a port, not a URL: the probe is a DNS lookup and a TCP
/// connect, so a scheme and a path would be noise, and carrying them would
/// invite someone to make an HTTP request out of it and call the answer
/// reachability.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkTarget {
    /// Hostname or literal IP address. A hostname is resolved first, which is
    /// what separates "DNS is wrong" from "the host is down".
    pub host: String,
    /// TCP port to attempt. `None` means the connector knows a host but no
    /// port worth trying — the probe then stops after DNS, because a connect
    /// needs somewhere to connect to.
    pub port: Option<u16>,
}

impl NetworkTarget {
    /// A target with both halves known — the case a full probe can run on.
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port: Some(port),
        }
    }
}

/// One resource found by a live connector's discovery pass.
///
/// This is a proposal, not a persisted instance. A client may present or edit
/// it before sending the configuration through the ordinary instance-creation
/// endpoint, where the target connector's factory validates it normally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredResource {
    /// Human-facing starting name for the eventual connector instance.
    pub suggested_name: String,
    /// Registry type id that should validate and construct `config`.
    pub target_connector_type: String,
    /// Suggested configuration in the target connector's schema shape.
    pub config: Value,
    /// Value that may be assigned directly to the source connector's
    /// [`Connector::discovery_target_field`].
    #[serde(default)]
    pub target_field_value: Option<Value>,
}

/// One capability reported by a candidate connection check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityStatus {
    /// Stable machine key used to match declarative and live capability rows.
    pub key: String,
    /// Human-facing capability name.
    pub label: String,
    /// Whether this candidate can currently provide the capability.
    pub available: bool,
    /// Optional explanation, especially useful when unavailable.
    pub note: Option<String>,
}

/// Result of explicitly testing a candidate connector configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionTestResult {
    /// Whether the candidate service can be contacted usefully.
    pub reachable: bool,
    /// Fine-grained live capability results, empty when a connector only
    /// supports the trait's default reachability check.
    pub capabilities: Vec<CapabilityStatus>,
    /// Optional connector-authored summary or failure explanation.
    pub message: Option<String>,
}

/// One UI-only switch offered by a setup-guide variant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupGuideToggle {
    /// Stable key used by templates and capability requirements.
    pub key: String,
    /// Environment variable represented in the rendered setup instructions.
    pub env_var: String,
    /// Human-facing toggle name.
    pub label: String,
    /// Explanation of what enabling the toggle changes in the setup.
    pub description: String,
    /// Initial UI state.
    pub default: bool,
    /// Whether the connector author recommends enabling it.
    pub recommended: bool,
}

/// Declarative capability unlocked by a setup-guide variant's toggles.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityRequirement {
    /// Capability key shared with live [`CapabilityStatus`] results.
    pub capability_key: String,
    /// Human-facing capability name.
    pub label: String,
    /// Toggle keys that must all be enabled. The v1 model is AND-only.
    pub required_toggle_keys: Vec<String>,
}

/// One independent way to prepare a connector's upstream service.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupGuideVariant {
    /// Stable identifier local to this setup guide.
    pub id: String,
    /// Short human-facing variant name.
    pub label: String,
    /// Explanation of when this setup path is appropriate.
    pub description: String,
    /// Client-rendered plain text with schema-field and toggle placeholders.
    pub template: String,
    /// UI-only switches that affect the rendered instructions.
    pub toggles: Vec<SetupGuideToggle>,
    /// Capabilities derived declaratively from the selected toggle state.
    pub capability_requirements: Vec<CapabilityRequirement>,
}

/// Type-level setup paths published alongside a connector's config schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupGuide {
    /// Independent supported setup approaches, in connector-authored order.
    pub variants: Vec<SetupGuideVariant>,
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
    /// Health for an individual addressable view. The empty-string key is the
    /// connector-level view; every other key is a [`SubTarget::id`].
    ///
    /// Additive and empty by default so statuses produced before target-aware
    /// health remain wire-compatible and clients can fall back to `health`.
    #[serde(default)]
    pub target_health: HashMap<String, HealthState>,
    /// The current reading for every data point, nested by target and then id.
    ///
    /// Typed as [`Value`] for serialization flexibility, but the shape is not
    /// free-form and consumers may rely on it: it **MUST** be a JSON object
    /// keyed first by target (`""` for connector-level values, otherwise the
    /// sub-target id) and then by `data_point_id`, matching the descriptors
    /// returned by [`Connector::data_points`]. Use the helpers in
    /// [`details`] rather than constructing this shape by hand. Each value's
    /// shape follows that data point's declared [`DataPointDescriptor::value_type`]:
    ///
    /// | `value_type` | JSON shape |
    /// | --- | --- |
    /// | [`Number`](DataPointValueType::Number) | a JSON number |
    /// | [`String`](DataPointValueType::String) | a JSON string |
    /// | [`Bool`](DataPointValueType::Bool) | a JSON boolean |
    /// | [`TimeSeries`](DataPointValueType::TimeSeries) | a JSON array of `{ "timestamp": <ISO 8601>, "value": <number> }` objects, oldest first |
    ///
    /// A connector may include extra keys within a target object that are not data points — a version
    /// string, a queue depth — and a client that does not recognise one ignores
    /// it. What it may not do is key a data point's value under anything but
    /// that target and data point id, because a saved layout stores both and
    /// resolves them here on every poll. [`ConnectorStatus::data_point_value_for`]
    /// is the intended way to read one.
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
            target_health: HashMap::new(),
            details,
            last_checked: Utc::now(),
        }
    }

    /// Records the health of one target and returns the enriched status.
    #[must_use]
    pub fn with_target_health(mut self, target_id: impl Into<String>, health: HealthState) -> Self {
        self.target_health.insert(target_id.into(), health);
        self
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
        self.data_point_value_for(None, data_point_id)
    }

    /// The current value of one target-scoped data point.
    pub fn data_point_value_for(
        &self,
        target_id: Option<&str>,
        data_point_id: &str,
    ) -> Option<&Value> {
        details::get_detail(&self.details, target_id, data_point_id)
    }
}

/// One addressable view below a connector instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubTarget {
    /// Stable id used by descriptors, placements, status details, and actions.
    pub id: String,
    /// Human-facing name shown when choosing a target.
    pub label: String,
    /// What *sort* of thing this target is, in the connector's own vocabulary
    /// — Docker uses `"container"` and `"stack"`.
    ///
    /// **Deliberately a free-form string and not an enum.** A closed set would
    /// have to name every kind of thing every connector will ever address, and
    /// the first connector to want a "pool", a "share" or a "zone" would either
    /// wait for a Core release or misuse the nearest existing word. It is the
    /// same choice already made for connector type ids, action ids and data
    /// point ids: the vocabulary belongs to the connector, and Loom carries it
    /// without interpreting it.
    ///
    /// Clients may group or icon by it and **must** tolerate a value they do
    /// not recognise by treating the target as an ordinary one. Nothing in Loom
    /// branches on it; a connector that distinguishes behaviour by kind does so
    /// from its own `target_id`, which is the thing it actually receives.
    #[serde(default = "default_sub_target_kind")]
    pub kind: String,
    /// Optional generic icon reference for this particular target. Connectors
    /// use the same curated `lucide:<name>` convention as metadata; clients
    /// fall back to the connector icon when it is absent or unknown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// What [`SubTarget::kind`] means when a connector does not say.
///
/// Every existing sub-target was one of these before the field existed, and a
/// stored or older-connector payload without it still deserializes to one.
pub const SUB_TARGET_KIND_DEFAULT: &str = "target";

fn default_sub_target_kind() -> String {
    SUB_TARGET_KIND_DEFAULT.to_owned()
}

impl SubTarget {
    /// A target of the default kind.
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            kind: default_sub_target_kind(),
            icon: None,
        }
    }

    /// Declares what sort of thing this target is.
    #[must_use]
    pub fn of_kind(mut self, kind: impl Into<String>) -> Self {
        self.kind = kind.into();
        self
    }

    /// Gives this target a generic icon independent of its connector type.
    #[must_use]
    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
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
    /// `None` for an instance-level action; otherwise the addressed sub-target.
    #[serde(default)]
    pub target_id: Option<String>,
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
    /// Whether running this action is expected to make the service *stop
    /// answering for a while*.
    ///
    /// Not "is this dangerous" and not "should we confirm first" — both are
    /// worth having and neither is this. This flag exists for one job: while a
    /// disruptive action is in flight, the platform should say **"Performing:
    /// Restart"** rather than flashing the service as Down, because Down is
    /// true and useless. It is the difference between a dashboard that is
    /// alarming and one that is informative.
    ///
    /// The test is *unexpectedly* unavailable. `stop` makes a service stop
    /// answering too, but the person who pressed Stop is not surprised by that
    /// and does not need it explained; `restart` is the case where a service
    /// disappears and comes back on its own, and the user has no idea how long
    /// the gap should be.
    ///
    /// Defaults to `false` — a connector author opts in per action, because an
    /// action wrongly marked disruptive suppresses a genuine outage.
    #[serde(default)]
    pub is_disruptive: bool,
    /// Data points whose current values are worth recording *before* this
    /// action runs.
    ///
    /// The platform reads each listed [`DataPointDescriptor::id`] from the
    /// latest cached reading — scoped to this descriptor's own
    /// [`target_id`](ConnectorAction::target_id) — and stores the result on the
    /// action's audit-log entry. What that buys is the answer to "what was it
    /// before?", which is the first question anyone asks after an action turns
    /// out to have been a mistake, and the raw material for an eventual undo.
    ///
    /// Ids the connector does not declare, or that the last poll did not
    /// report, are simply absent from the snapshot. A snapshot is a best-effort
    /// record of what was known, not a guarantee that everything listed was
    /// available: refusing to run an action because a reading was missing would
    /// be a far worse failure than recording an incomplete one.
    ///
    /// Empty by default. Marking an action costs one poll's worth of already-
    /// cached values and nothing else — no extra call to the service — so the
    /// bar is simply whether the reading would mean something afterwards.
    #[serde(default)]
    pub snapshot_data_point_ids: Vec<String>,
}

impl ConnectorAction {
    /// A parameterless action with an id and a label.
    ///
    /// Covers the majority of real actions ("restart", "refresh") without
    /// making every call site spell out an empty schema.
    pub fn simple(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            target_id: None,
            label: label.into(),
            description: None,
            params_schema: Value::Object(Default::default()),
            is_disruptive: false,
            snapshot_data_point_ids: Vec::new(),
        }
    }

    /// Attaches a description, for chaining onto [`ConnectorAction::simple`].
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Scopes this action descriptor to one addressable sub-target.
    #[must_use]
    pub fn for_target(mut self, target_id: impl Into<String>) -> Self {
        self.target_id = Some(target_id.into());
        self
    }

    /// Marks this action as one that takes the service away and brings it back
    /// — see [`ConnectorAction::is_disruptive`].
    #[must_use]
    pub fn disruptive(mut self) -> Self {
        self.is_disruptive = true;
        self
    }

    /// Records these data points' current values before this action runs — see
    /// [`ConnectorAction::snapshot_data_point_ids`].
    #[must_use]
    pub fn snapshotting<I, S>(mut self, data_point_ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.snapshot_data_point_ids = data_point_ids.into_iter().map(Into::into).collect();
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
    /// Several named numeric readings carried by one data point.
    ///
    /// Values are arrays of `{ "label": string, "value": number }` objects.
    /// This is intended for Bar/Pie [`DisplayWidgetType::MetricChart`]
    /// bindings: pool capacity by pool today, and similar per-interface or
    /// per-share breakdowns in future connectors.
    CategoryBreakdown,
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
    /// `None` for an instance-level reading; otherwise the addressed sub-target.
    #[serde(default)]
    pub target_id: Option<String>,
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
            target_id: None,
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

    /// Scopes this descriptor to one addressable sub-target.
    #[must_use]
    pub fn for_target(mut self, target_id: impl Into<String>) -> Self {
        self.target_id = Some(target_id.into());
        self
    }
}

/* ------------------------------------------------------------------ */
/* Resource browser                                                    */
/* ------------------------------------------------------------------ */

/// The shape of one table cell, so a client knows how to format it.
///
/// Deliberately *not* [`DataPointValueType`], even though the two overlap. A
/// data point drives a widget: its type decides which widgets may bind to it,
/// so it needs [`TimeSeries`](DataPointValueType::TimeSeries) and has no use
/// for a byte count. A column drives a cell in a table: it never needs a
/// series, and it does need the two cases a raw number renders badly as — a
/// size, which should read `1.4 GB` rather than `1503238553`, and an instant,
/// which should read in the viewer's own locale and timezone rather than as an
/// ISO string. Merging them would give every widget binding two variants it
/// cannot draw and every table cell one it cannot fit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ColumnValueType {
    /// A plain string, shown as-is.
    Text,
    /// A number, shown with the client's ordinary numeric formatting.
    Number,
    /// A flag, shown as the client's yes/no affordance rather than the literal
    /// `true`.
    Bool,
    /// An instant as an ISO 8601 string, shown localized — a date, a time, or
    /// "3 days ago", whichever the client's table style calls for.
    Timestamp,
    /// A size in **bytes**, shown human-readable. The value on the wire is
    /// always the raw byte count; scaling it to KB/MB/GB is the client's
    /// business, exactly as with [`DataPointDescriptor::unit`], so two clients
    /// never disagree about what the number meant.
    Bytes,
    /// A short verdict about the row, shown as a coloured pill. The value is a
    /// [`StatusValue`] object — a label and a tone — not a bare string.
    ///
    /// The **connector** supplies the tone, rather than the client inferring
    /// one from the label. A client cannot know that "unused" is good news for
    /// an image and bad news for a backup job, and a lookup table of known
    /// words in the frontend would be a connector's vocabulary living in
    /// someone else's code.
    Status,
}

/// How a [`ColumnValueType::Status`] pill should read at a glance.
///
/// Deliberately about *sentiment*, not about colour: a client picks the colours
/// that match its theme, including in a high-contrast or colour-blind palette
/// where "green" and "red" are not the distinction being drawn. Naming the
/// tones after literal colours would have hard-coded one palette into every
/// connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StatusTone {
    /// A statement of fact with no judgement attached.
    #[default]
    Neutral,
    /// Working as intended.
    Positive,
    /// Worth a look, but nothing is broken.
    Caution,
    /// Something is wrong, or is about to be.
    Negative,
}

/// One [`ColumnValueType::Status`] cell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusValue {
    /// The words in the pill. Short — this is a label, not a sentence.
    pub label: String,
    /// How it should read.
    pub tone: StatusTone,
}

impl StatusValue {
    /// A pill with a label and a tone.
    pub fn new(label: impl Into<String>, tone: StatusTone) -> Self {
        Self {
            label: label.into(),
            tone,
        }
    }
}

impl From<StatusValue> for Value {
    fn from(status: StatusValue) -> Self {
        // Infallible in practice: the shape is two owned, serializable fields.
        // `Null` rather than a panic if that ever stops being true, because a
        // blank cell is a better outcome than a downed poll.
        serde_json::to_value(status).unwrap_or(Value::Null)
    }
}

/// One column of a browsable resource table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnDescriptor {
    /// Machine key this column's value appears under in
    /// [`ResourceItem::fields`]. Stable, like a data point id.
    pub key: String,
    /// Human-facing column heading.
    pub label: String,
    /// How to format the cell.
    pub value_type: ColumnValueType,
}

impl ColumnDescriptor {
    /// A column with a key, a heading, and a value type.
    pub fn new(
        key: impl Into<String>,
        label: impl Into<String>,
        value_type: ColumnValueType,
    ) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
            value_type,
        }
    }
}

/// One row of a browsable resource table.
///
/// `fields` is keyed by [`ColumnDescriptor::key`]. A missing key renders as an
/// empty cell rather than as a failure — a resource that genuinely has no value
/// for a column is ordinary — and a key that matches no column is ignored by
/// clients, the same tolerance [`ConnectorStatus::details`] has.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceItem {
    /// Stable identifier for this row within its kind, passed back as
    /// `resourceId` when a row action is invoked — see
    /// [`ResourceKindDescriptor::row_actions`].
    pub id: String,
    /// The cell values, keyed by column.
    pub fields: HashMap<String, Value>,
}

impl ResourceItem {
    /// A row with an id and no fields yet.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            fields: HashMap::new(),
        }
    }

    /// Sets one cell, for chaining onto [`ResourceItem::new`].
    #[must_use]
    pub fn with_field(mut self, key: impl Into<String>, value: impl Into<Value>) -> Self {
        self.fields.insert(key.into(), value.into());
        self
    }
}

/// Where a browsable kind makes sense.
///
/// A connector instance is looked at in two places: at the host, where it
/// stands for the whole service, and at one [`SubTarget`], where it stands for
/// a single container, share, or zone. Most kinds only answer a question in one
/// of the two. Docker's images are a property of the daemon and reading them
/// "for one container" is a category error; a future per-container kind —
/// snapshots, mounted paths — is the same error the other way round.
///
/// Declared rather than inferred. The alternative is for every client to guess
/// from whether a listing came back empty, which cannot distinguish "this does
/// not apply here" from "there are none right now" and so shows an empty tab
/// that will never fill. Defaults to [`Any`](ApplicableTarget::Any), so a kind
/// that says nothing keeps the behaviour every existing kind already had.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ApplicableTarget {
    /// Only when the instance as a whole is being viewed.
    HostOnly,
    /// Only when one sub-target is being viewed.
    TargetOnly,
    /// Both. The default, and the right answer for a kind whose rows mean the
    /// same thing at either altitude.
    #[default]
    Any,
}

/// One browsable collection of things a connector's service holds.
///
/// The unit a resource browser is built from: a table of [`ResourceItem`] rows
/// described by [`ColumnDescriptor`] columns, plus the operations offered
/// beside it. Generic on purpose — Docker's images, volumes, and networks are
/// three instances of this shape, not three features.
///
/// # Invoking a row action
///
/// Row actions are ordinary [`ConnectorAction`]s and run through the ordinary
/// [`Connector::execute_action`]. Which row they act on travels in `params`
/// under the key **`resourceId`**, carrying the [`ResourceItem::id`]:
///
/// ```
/// # use serde_json::json;
/// # let params =
/// json!({ "resourceId": "sha256:2f1c…", "force": true })
/// # ;
/// ```
///
/// That convention is why this type adds no argument to `execute_action`. The
/// alternative — a third parameter alongside `target_id` — would be a breaking
/// change to a trait every connector implements, in order to express something
/// `params` already carries perfectly well. `target_id` stays what it has
/// always been: which *sub-target* is addressed, orthogonal to which row is.
///
/// An implementation must treat a missing `resourceId` as
/// [`ConnectorError::InvalidParams`] rather than guessing at a row, and should
/// declare it in the action's [`params_schema`](ConnectorAction::params_schema)
/// so a client can see the requirement rather than learn it from this document.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceKindDescriptor {
    /// Stable machine identifier, unique within this connector, and the URL
    /// segment the rows are fetched under.
    pub kind: String,
    /// Human-facing name for the table or tab ("Images", "Volumes").
    pub label: String,
    /// The columns, in the order they should be shown.
    pub columns: Vec<ColumnDescriptor>,
    /// Operations on a single row. The caller passes the row's
    /// [`ResourceItem::id`] as `resourceId` in the action's `params` — see the
    /// type-level documentation above.
    pub row_actions: Vec<ConnectorAction>,
    /// Operations on the kind as a whole, addressing no particular row —
    /// "prune unused", "pull updates". Invoked exactly like any other action,
    /// with no `resourceId`.
    pub kind_actions: Vec<ConnectorAction>,
    /// A [`ColumnDescriptor::key`] whose value rows should be gathered under,
    /// when this table reads better grouped than flat.
    ///
    /// A hint, not a contract: the rows are the same rows either way, and a
    /// client that ignores it renders a correct flat table. Docker's image list
    /// is the case that earned it — twenty rows of which three are `postgres`
    /// and four are `nginx` is a list you have to read, and the same twenty
    /// under seven repository headings is a list you can scan.
    ///
    /// A key naming no column is ignored rather than an error, the same
    /// tolerance [`ResourceItem::fields`] has.
    #[serde(default)]
    pub group_by_key: Option<String>,
    /// Whether this kind is worth showing at the host, at one sub-target, or
    /// both.
    #[serde(default)]
    pub applicable_target: ApplicableTarget,
    /// Extra values describing each **group** as a whole, shown on the group
    /// heading and never as a row cell. Ignored when
    /// [`group_by_key`](Self::group_by_key) is `None`.
    ///
    /// Each descriptor's `key` names a field that every row of a group carries
    /// with the same value, so a client reads it from any row of the group
    /// rather than computing it.
    ///
    /// **Deliberately not client-side aggregation.** "Sum the size column"
    /// looks like the obvious generic answer and is wrong for the first real
    /// case: Docker lists one row per *tag*, so three tags of one 2 GB image
    /// are three rows of 2 GB, and a client summing them would report 6 GB of
    /// disk that does not exist. Only the connector knows which rows share a
    /// thing. The same applies to a verdict — "some of these are unused" is not
    /// derivable from a column of per-row verdicts without knowing what the
    /// rows mean.
    #[serde(default)]
    pub group_summary: Vec<ColumnDescriptor>,
}

impl ResourceKindDescriptor {
    /// A kind with columns and no actions, for chaining.
    pub fn new(
        kind: impl Into<String>,
        label: impl Into<String>,
        columns: Vec<ColumnDescriptor>,
    ) -> Self {
        Self {
            kind: kind.into(),
            label: label.into(),
            columns,
            row_actions: Vec::new(),
            kind_actions: Vec::new(),
            group_by_key: None,
            applicable_target: ApplicableTarget::Any,
            group_summary: Vec::new(),
        }
    }

    /// Attaches the per-row operations.
    #[must_use]
    pub fn with_row_actions(mut self, actions: Vec<ConnectorAction>) -> Self {
        self.row_actions = actions;
        self
    }

    /// Attaches the whole-kind operations.
    #[must_use]
    pub fn with_kind_actions(mut self, actions: Vec<ConnectorAction>) -> Self {
        self.kind_actions = actions;
        self
    }

    /// Declares the column rows should be grouped under.
    #[must_use]
    pub fn grouped_by(mut self, key: impl Into<String>) -> Self {
        self.group_by_key = Some(key.into());
        self
    }

    /// Declares where this kind is worth showing.
    #[must_use]
    pub fn applicable_to(mut self, target: ApplicableTarget) -> Self {
        self.applicable_target = target;
        self
    }

    /// Declares the group-level values shown on each group heading.
    #[must_use]
    pub fn with_group_summary(mut self, columns: Vec<ColumnDescriptor>) -> Self {
        self.group_summary = columns;
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
/// Externally tagged, so each value is a single-key object (`{"display": …}`,
/// `{"action": …}`, or `{"resourceKindDisplay": …}`) — the same shape as
/// [`ConnectorError`] and [`DisplayWidgetType::MetricChart`].
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
    /// A widget showing one of the connector's browsable resource kinds.
    ///
    /// Display-adjacent, and deliberately its own arm rather than a
    /// [`DisplayWidgetType`]: the rows come from
    /// [`Connector::list_resource_items`] rather than from a
    /// [`DataPointDescriptor`], so it resolves against a third identifier
    /// space — [`ResourceKindDescriptor::kind`] — for the same reason
    /// `Display` and `Action` are already separate arms.
    ///
    /// **It carries no `widget_type` and no `config`, on purpose.** There is
    /// exactly one way to render a resource kind — the table/browser
    /// presentation a client already implements for
    /// `GET /connector-instances/{id}/resources/{kind}` — and it adapts to
    /// whatever area the placement occupies, from a corner tile to a
    /// placement filling an entire dashboard. Offering a widget type here
    /// would invite per-kind rendering variants that no connector asked for
    /// and that the resource browser would then have to keep in step.
    ResourceKindDisplay {
        /// Which [`ResourceKindDescriptor::kind`] to browse. Must be one the
        /// connector currently declares from
        /// [`Connector::resource_kinds`] for the placement's target.
        resource_kind: String,
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

    /// A binding that browses one resource kind.
    pub fn resource_kind_display(resource_kind: impl Into<String>) -> Self {
        Self::ResourceKindDisplay {
            resource_kind: resource_kind.into(),
        }
    }

    /// Attaches widget-specific configuration, for chaining onto
    /// [`WidgetBinding::display`] or [`WidgetBinding::action`].
    ///
    /// A no-op on [`WidgetBinding::ResourceKindDisplay`], which has no config
    /// to attach — see that variant's documentation.
    #[must_use]
    pub fn with_config(mut self, config: Value) -> Self {
        match &mut self {
            Self::Display { config: slot, .. } | Self::Action { config: slot, .. } => {
                *slot = config;
            }
            Self::ResourceKindDisplay { .. } => {}
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

    #[test]
    fn discovered_resource_defaults_a_missing_target_field_value() {
        let resource: DiscoveredResource = serde_json::from_value(json!({
            "suggestedName": "Legacy proposal",
            "targetConnectorType": "debug",
            "config": {},
        }))
        .expect("the new optional field must preserve older discovery payloads");
        assert_eq!(resource.target_field_value, None);
    }

    #[test]
    fn sub_target_icon_is_optional_and_uses_the_shared_wire_convention() {
        let ordinary = serde_json::to_value(SubTarget::new("one", "One")).expect("target");
        assert_eq!(ordinary.get("icon"), None);

        let decorated = serde_json::to_value(
            SubTarget::new("ap-one", "Access point")
                .of_kind("device")
                .with_icon("lucide:wifi"),
        )
        .expect("decorated target");
        assert_eq!(decorated["icon"], "lucide:wifi");
        assert_eq!(decorated["kind"], "device");
    }

    #[test]
    fn setup_guide_and_capability_types_use_the_documented_wire_shape() {
        let guide = SetupGuide {
            variants: vec![SetupGuideVariant {
                id: "proxy".to_owned(),
                label: "Proxy".to_owned(),
                description: "Configure a proxy.".to_owned(),
                template: "FEATURE={{enableFeature}}".to_owned(),
                toggles: vec![SetupGuideToggle {
                    key: "enableFeature".to_owned(),
                    env_var: "FEATURE".to_owned(),
                    label: "Enable feature".to_owned(),
                    description: "Exposes the feature.".to_owned(),
                    default: true,
                    recommended: true,
                }],
                capability_requirements: vec![CapabilityRequirement {
                    capability_key: "read-feature".to_owned(),
                    label: "Read feature".to_owned(),
                    required_toggle_keys: vec!["enableFeature".to_owned()],
                }],
            }],
        };

        let value = serde_json::to_value(&guide).expect("serialize setup guide");
        assert_eq!(value["variants"][0]["toggles"][0]["envVar"], "FEATURE");
        assert_eq!(
            value["variants"][0]["capabilityRequirements"][0]["requiredToggleKeys"],
            json!(["enableFeature"])
        );
        assert_eq!(
            serde_json::from_value::<SetupGuide>(value).expect("deserialize setup guide"),
            guide
        );
    }

    /// A minimal in-test implementation, separate from [`debug::DebugConnector`]
    /// on purpose: it proves the trait alone is enough to write a connector,
    /// with no help from the fixture's machinery.
    struct StubConnector {
        id: &'static str,
        health: HealthState,
    }

    #[tokio::test]
    async fn default_connection_test_maps_health_without_inventing_capabilities() {
        for (health, reachable) in [
            (HealthState::Healthy, true),
            (HealthState::Degraded, true),
            (HealthState::Down, false),
            (HealthState::Unknown, false),
        ] {
            let result = StubConnector {
                id: "connection-test-stub",
                health,
            }
            .test_connection()
            .await;
            assert_eq!(result.reachable, reachable);
            assert!(result.capabilities.is_empty());
            assert_eq!(result.message, None);
        }
    }

    #[async_trait]
    impl Connector for StubConnector {
        async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
            // Keyed by this stub's one data point id, per the
            // `ConnectorStatus::details` contract.
            Ok(ConnectorStatus::new(
                self.health,
                json!({ "": { "reading": 42.0 } }),
            ))
        }

        async fn actions(&self) -> Vec<ConnectorAction> {
            vec![ConnectorAction::simple("noop", "Do nothing")]
        }

        async fn execute_action(
            &self,
            action_id: &str,
            _target_id: Option<&str>,
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
                    status
                        .data_point_value_for(point.target_id.as_deref(), &point.id)
                        .is_some(),
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
            let kinds = connector.resource_kinds(None);
            let kind_ids: Vec<&str> = kinds.iter().map(|kind| kind.kind.as_str()).collect();
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
                    WidgetBinding::ResourceKindDisplay { resource_kind } => assert!(
                        kind_ids.contains(&resource_kind.as_str()),
                        "layout binds unknown resource kind {resource_kind}"
                    ),
                }
            }
        }

        assert_eq!(
            connectors[0]
                .execute_action("noop", None, Value::Null)
                .await
                .expect("noop should succeed")
                .message,
            "did nothing"
        );
        assert_eq!(
            connectors[1]
                .execute_action("nope", None, Value::Null)
                .await
                .expect_err("unknown action should fail"),
            ConnectorError::invalid_action("nope")
        );
    }

    #[test]
    fn an_action_snapshots_only_what_it_names() {
        // The default is the load-bearing half: a connector written before the
        // field existed, or one with nothing worth recording, must produce no
        // snapshot rather than an empty one nobody can distinguish from a
        // failed capture.
        assert!(ConnectorAction::simple("ping", "Ping")
            .snapshot_data_point_ids
            .is_empty());
        assert_eq!(
            ConnectorAction::simple("recalibrate", "Recalibrate")
                .snapshotting(["load", "mode"])
                .snapshot_data_point_ids,
            vec!["load".to_string(), "mode".to_string()]
        );

        let older = json!({
            "id": "restart",
            "label": "Restart",
            "description": null,
            "paramsSchema": {},
            "isDisruptive": true
        });
        let parsed: ConnectorAction = serde_json::from_value(older).expect("older shape");
        assert!(parsed.snapshot_data_point_ids.is_empty());
        assert_eq!(
            serde_json::to_value(&parsed).unwrap()["snapshotDataPointIds"],
            json!([])
        );
    }

    #[test]
    fn an_update_check_result_carries_a_flag_and_an_opaque_reference() {
        assert_eq!(
            serde_json::to_value(UpdateCheckResult::up_to_date()).unwrap(),
            json!({ "available": false, "latestRef": null })
        );

        let found = UpdateCheckResult::available("example/image@sha256:0123abcd");
        let value = serde_json::to_value(&found).unwrap();
        assert_eq!(
            value,
            json!({ "available": true, "latestRef": "example/image@sha256:0123abcd" })
        );
        assert_eq!(
            serde_json::from_value::<UpdateCheckResult>(value).unwrap(),
            found
        );
    }

    #[test]
    fn resource_browser_types_use_the_documented_wire_shape() {
        let kind = ResourceKindDescriptor::new(
            "images",
            "Images",
            vec![
                ColumnDescriptor::new("tag", "Tag", ColumnValueType::Text),
                ColumnDescriptor::new("size", "Size", ColumnValueType::Bytes),
                ColumnDescriptor::new("createdAt", "Created", ColumnValueType::Timestamp),
            ],
        )
        .with_row_actions(vec![ConnectorAction::simple("remove", "Remove")])
        .with_kind_actions(vec![ConnectorAction::simple("prune", "Prune")])
        .grouped_by("repository")
        .applicable_to(ApplicableTarget::HostOnly)
        .with_group_summary(vec![ColumnDescriptor::new(
            "totalSize",
            "Total size",
            ColumnValueType::Bytes,
        )]);

        let value = serde_json::to_value(&kind).unwrap();
        assert_eq!(value["kind"], "images");
        assert_eq!(value["columns"][1]["valueType"], "bytes");
        assert_eq!(value["columns"][2]["valueType"], "timestamp");
        assert_eq!(value["rowActions"][0]["id"], "remove");
        assert_eq!(value["kindActions"][0]["id"], "prune");
        assert_eq!(value["groupByKey"], "repository");
        assert_eq!(value["applicableTarget"], "hostOnly");
        assert_eq!(value["groupSummary"][0]["key"], "totalSize");
        assert_eq!(
            serde_json::from_value::<ResourceKindDescriptor>(value).unwrap(),
            kind
        );

        // A row is an id plus a flat, column-keyed object — the shape a table
        // renderer indexes straight into.
        let item = ResourceItem::new("sha256:abc")
            .with_field("tag", "example:1.0")
            .with_field("size", 1024);
        assert_eq!(
            serde_json::to_value(&item).unwrap(),
            json!({
                "id": "sha256:abc",
                "fields": { "tag": "example:1.0", "size": 1024 }
            })
        );
        assert_eq!(
            serde_json::from_value::<ResourceItem>(serde_json::to_value(&item).unwrap()).unwrap(),
            item
        );
    }

    #[test]
    fn a_status_cell_carries_its_own_tone() {
        let item = ResourceItem::new("nginx:1.27")
            .with_field("usage", StatusValue::new("In use", StatusTone::Positive));
        assert_eq!(
            serde_json::to_value(&item).unwrap()["fields"]["usage"],
            json!({ "label": "In use", "tone": "positive" })
        );
        // The tone travels with the value rather than being inferred from the
        // label, so two connectors can disagree about whether "unused" is good.
        assert_eq!(
            serde_json::to_value(ColumnValueType::Status).unwrap(),
            json!("status")
        );
        assert_eq!(
            serde_json::from_value::<StatusValue>(json!({ "label": "Gone" })).ok(),
            None,
            "a tone is required rather than silently neutral"
        );
    }

    /// The two hint fields are additive: a descriptor written before they
    /// existed still deserializes, and keeps the behaviour it had.
    #[test]
    fn a_descriptor_without_the_hints_defaults_to_ungrouped_and_anywhere() {
        let value = json!({
            "kind": "widgets",
            "label": "Widgets",
            "columns": [],
            "rowActions": [],
            "kindActions": []
        });
        let kind: ResourceKindDescriptor = serde_json::from_value(value).unwrap();
        assert_eq!(kind.group_by_key, None);
        assert_eq!(kind.applicable_target, ApplicableTarget::Any);
        assert!(kind.group_summary.is_empty());
        assert_eq!(
            ResourceKindDescriptor::new("widgets", "Widgets", Vec::new()),
            kind
        );
    }

    /// The trait's resource-browser methods are opt-in: a connector that knows
    /// nothing about them must still compile and must report nothing to browse,
    /// which is what keeps this an additive capability rather than a migration.
    #[tokio::test]
    async fn a_connector_that_ignores_the_resource_browser_browses_nothing() {
        let stub = StubConnector {
            id: "stub-resources",
            health: HealthState::Healthy,
        };
        assert!(stub.resource_kinds(None).is_empty());
        assert!(stub.resource_kinds(Some("anything")).is_empty());
        assert_eq!(
            stub.list_resource_items("anything", None).await.unwrap(),
            Vec::new()
        );

        // Same contract for the update check: a connector that knows nothing
        // about it says so, and answers usefully rather than erroring.
        assert!(!stub.supports_update_checking());
        assert_eq!(
            stub.check_for_updates(None).await.unwrap(),
            UpdateCheckResult::up_to_date()
        );
    }

    #[test]
    fn connector_status_serializes_with_camel_case_keys() {
        let status = ConnectorStatus {
            health: HealthState::Degraded,
            target_health: HashMap::from([
                (String::new(), HealthState::Degraded),
                ("fixture-a".to_owned(), HealthState::Healthy),
            ]),
            details: json!({ "queueDepth": 12 }),
            last_checked: Utc.with_ymd_and_hms(2026, 8, 19, 12, 0, 0).unwrap(),
        };

        let value = serde_json::to_value(&status).unwrap();
        assert_eq!(
            value,
            json!({
                "health": "degraded",
                "targetHealth": {
                    "": "degraded",
                    "fixture-a": "healthy"
                },
                "details": { "queueDepth": 12 },
                "lastChecked": "2026-08-19T12:00:00Z"
            })
        );
        assert_eq!(
            serde_json::from_value::<ConnectorStatus>(value).unwrap(),
            status
        );

        let legacy = serde_json::from_value::<ConnectorStatus>(json!({
            "health": "healthy",
            "details": {},
            "lastChecked": "2026-08-19T12:00:00Z"
        }))
        .unwrap();
        assert!(legacy.target_health.is_empty());
    }

    #[test]
    fn category_breakdown_uses_the_documented_wire_name() {
        assert_eq!(
            serde_json::to_value(DataPointValueType::CategoryBreakdown).unwrap(),
            json!("categoryBreakdown")
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
                "targetId": null,
                "label": "Restart",
                "description": "Restarts it.",
                "paramsSchema": {},
                "isDisruptive": false,
                "snapshotDataPointIds": []
            })
        );
        assert_eq!(
            serde_json::from_value::<ConnectorAction>(value).unwrap(),
            action
        );
    }

    #[test]
    fn an_action_is_only_disruptive_when_it_says_so() {
        // The default matters more than the flag: an action wrongly marked
        // disruptive suppresses a real outage behind a "Performing…" overlay,
        // so opting in has to be the deliberate half.
        assert!(!ConnectorAction::simple("ping", "Ping").is_disruptive);
        assert!(
            ConnectorAction::simple("restart", "Restart")
                .disruptive()
                .is_disruptive
        );

        // `isDisruptive` is `#[serde(default)]`, so a stored or third-party
        // action written before the field existed still deserializes — as
        // not-disruptive, which is the safe reading.
        let older = json!({
            "id": "restart",
            "label": "Restart",
            "description": null,
            "paramsSchema": {}
        });
        let parsed: ConnectorAction = serde_json::from_value(older).expect("older shape");
        assert!(!parsed.is_disruptive);
    }

    #[test]
    fn a_network_target_carries_a_host_and_an_optional_port() {
        let full = NetworkTarget::new("proxy.example", 2375);
        assert_eq!(
            serde_json::to_value(&full).unwrap(),
            json!({ "host": "proxy.example", "port": 2375 })
        );

        // A host with no port is a legitimate target: DNS is still worth
        // checking even when there is nowhere to connect afterwards.
        let dns_only = NetworkTarget {
            host: "proxy.example".to_owned(),
            port: None,
        };
        assert_eq!(
            serde_json::to_value(&dns_only).unwrap(),
            json!({ "host": "proxy.example", "port": null })
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
            json!({ "": { "load": 12.5, "enabled": true }, "worker": { "load": 8 } }),
        );
        assert_eq!(status.data_point_value("load"), Some(&json!(12.5)));
        assert_eq!(status.data_point_value("enabled"), Some(&json!(true)));
        assert_eq!(status.data_point_value("missing"), None);
        assert_eq!(
            status.data_point_value_for(Some("worker"), "load"),
            Some(&json!(8))
        );

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
            target_health: HashMap::new(),
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
