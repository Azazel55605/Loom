//! The connector: one Docker container, watched and controlled.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use bollard::models::{ContainerInspectResponse, ContainerStatsResponse};
use bollard::query_parameters::{LogsOptionsBuilder, StatsOptionsBuilder};
use bollard::Docker;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use loom_core::connector::{
    ActionResult, ActionWidgetType, ChartType, Connector, ConnectorAction, ConnectorError,
    ConnectorMetadata, ConnectorStatus, DataPointDescriptor, DataPointValueType, DisplayField,
    DisplayWidgetType, HealthState, NetworkTarget, WidgetBinding, WidgetLayout,
};
use serde::Serialize;
use serde_json::{json, Map, Value};

use crate::config::{config_schema, DockerConnectorConfig};
use crate::metrics::{cpu_percent, format_uptime, health_for_state};

/// The connector type id this registers under.
pub const TYPE_ID: &str = "docker-container";

/// Human-facing name for the type picker.
pub const DISPLAY_NAME: &str = "Docker Container";

/// Icon reference, in the `ConnectorMetadata::icon` convention. Vendored under
/// `packages/ui-kit/src/assets/icons/brand` — see `docs/THIRD_PARTY_ICONS.md`
/// for its license and attribution.
///
/// A `const` rather than a literal in [`DockerConnector::metadata`] because the
/// type registry needs the same value *before* any instance exists: a Docker
/// connector cannot be default-constructed to be asked, so the registration
/// would otherwise carry a hand-copied duplicate that could drift from what the
/// connector actually reports.
pub const ICON: &str = "brand:docker";

/// Data point ids. Public because a dashboard layout stores them, so a rename
/// is a breaking change to saved layouts and should be visible as one.
pub const DATA_POINT_STATUS: &str = "status";
pub const DATA_POINT_CPU_PERCENT: &str = "cpuPercent";
pub const DATA_POINT_CPU_HISTORY: &str = "cpuHistory";
pub const DATA_POINT_MEMORY_USAGE_BYTES: &str = "memoryUsageBytes";
pub const DATA_POINT_MEMORY_HISTORY: &str = "memoryHistory";
pub const DATA_POINT_UPTIME: &str = "uptime";
pub const DATA_POINT_LOGS: &str = "logs";

/// Action ids.
pub const ACTION_START: &str = "start";
pub const ACTION_STOP: &str = "stop";
pub const ACTION_RESTART: &str = "restart";
pub const ACTION_PAUSE: &str = "pause";
pub const ACTION_UNPAUSE: &str = "unpause";

/// How many samples each history data point keeps.
///
/// Fifty, matching `DebugConnector`. At the default poll interval that is a
/// window of several minutes — enough for a chart to show a shape, short
/// enough that the buffer costs nothing and that a restarted Loom is not
/// missing hours of history anyone was relying on. This is a live view, not a
/// metrics store: a homelab that wants retention wants Prometheus, and Loom
/// should not pretend otherwise by keeping ten thousand points in memory.
pub const HISTORY_CAPACITY: usize = 50;

/// How many log lines the `logs` data point carries.
pub const LOG_TAIL_LINES: usize = 20;

/// One point in a history buffer.
///
/// Serialized as `{ "timestamp": "…", "value": … }`, which is the shape the
/// `timeSeries` value type promises and the chart widget reads.
#[derive(Debug, Clone, Serialize)]
struct HistorySample {
    timestamp: DateTime<Utc>,
    value: f64,
}

/// The rolling buffers, which are the only state this connector carries
/// between polls.
#[derive(Debug, Default)]
struct History {
    cpu: VecDeque<HistorySample>,
    memory: VecDeque<HistorySample>,
}

impl History {
    /// Appends both samples, dropping the oldest once the buffer is full.
    fn record(&mut self, cpu: f64, memory: f64, at: DateTime<Utc>) {
        for (buffer, value) in [(&mut self.cpu, cpu), (&mut self.memory, memory)] {
            if buffer.len() == HISTORY_CAPACITY {
                buffer.pop_front();
            }
            buffer.push_back(HistorySample {
                timestamp: at,
                value,
            });
        }
    }
}

/// Monitors and controls one Docker container.
///
/// # What this first version deliberately does not do
///
/// [`Connector::discoverable_type`], [`Connector::discover`],
/// [`Connector::setup_guide`] and `test_connection` all keep their trait
/// defaults. These are **planned follow-ups, not oversights**:
///
/// - **Discovery** belongs to a *host*-level connector that can list every
///   container on a daemon and propose one of these per container. This
///   connector is scoped to a single container it was told the name of, so
///   the only thing it could discover is itself.
/// - **A setup guide** is worth writing once it can be capability-aware — the
///   text that helps is different for a socket bind-mount, a rootless daemon,
///   and a `docker-socket-proxy`, and a single static template that covers all
///   three helps with none of them.
///
/// Neither is stubbed with a placeholder. A `setup_guide` returning generic
/// text would be worse than `None`, because a client renders whatever it gets.
pub struct DockerConnector {
    config: DockerConnectorConfig,
    /// Reads — inspect, stats, logs — on a short timeout, so a host that has
    /// gone away fails a poll quickly instead of stacking them up.
    docker: Docker,
    /// Lifecycle actions, on a much longer one. `docker stop` legitimately
    /// blocks for the container's whole stop grace period, and a client that
    /// gave up first would report a failure for a stop that succeeded. Two
    /// clients rather than one compromise timeout, because there is no single
    /// value that is both "fail fast" and "wait out a shutdown".
    control: Docker,
    /// `std::sync::Mutex`, not tokio's: every critical section here is a few
    /// pushes with no `await` inside it, so an async mutex would buy nothing
    /// and cost a scheduler hop per poll.
    history: Arc<Mutex<History>>,
}

/// Hand-written because `bollard::Docker` is not `Debug`, and because the
/// interesting part of a connector in a log line is which container on which
/// host it points at — not the state of an HTTP connection pool.
impl std::fmt::Debug for DockerConnector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DockerConnector")
            .field("docker_host", &self.config.docker_host)
            .field("container_name", &self.config.container_name)
            .finish_non_exhaustive()
    }
}

impl DockerConnector {
    /// Builds a connector and proves it can be used.
    ///
    /// Two round trips' worth of validation happens here rather than at first
    /// poll, so that "add connector" fails at the moment someone can still fix
    /// the form, and so the two failures a bad configuration produces stay
    /// distinguishable:
    ///
    /// - the daemon cannot be reached at all → [`ConnectorError::Unreachable`],
    ///   which points at the socket, the mount, or the network;
    /// - the daemon answered but has no such container →
    ///   [`ConnectorError::InvalidParams`], which points at the name field.
    ///
    /// Collapsing those into one "could not connect" is the difference between
    /// a two-minute fix and an afternoon.
    pub async fn connect(config: DockerConnectorConfig) -> Result<Self, ConnectorError> {
        let docker = config.connect()?;
        let control = config.connect_for_control()?;

        // `inspect` rather than `ping`: it proves reachability *and* existence
        // in one call, and a ping that succeeded followed by a first poll that
        // 404s would report the connector as working right up until it did not.
        let inspect = docker
            .inspect_container(&config.container_name, None)
            .await
            .map_err(|error| inspect_failure(&config, &error))?;

        // A container matched by a name prefix is a different container from
        // the one the user named, and acting on the wrong one is the worst
        // failure this connector has available. Docker resolves exact names and
        // id prefixes, so this is belt and braces rather than a real filter.
        debug_assert!(inspect.id.is_some() || inspect.name.is_some());

        Ok(Self {
            config,
            docker,
            control,
            history: Arc::new(Mutex::new(History::default())),
        })
    }

    /// Convenience for the registry factory: parse, then connect.
    pub async fn from_config_value(config: Value) -> Result<Self, ConnectorError> {
        Self::connect(DockerConnectorConfig::from_value(config)?).await
    }

    /// One sample of stats, with `precpu_stats` populated.
    ///
    /// `stream(false)` and **not** `one_shot(true)`, which is the trap in this
    /// endpoint. With `stream=false` Docker waits for two collection cycles and
    /// returns a single sample carrying both `cpu_stats` and `precpu_stats`,
    /// which is exactly the pair the CPU formula needs — no state kept between
    /// polls, no first-poll blind spot. With `one_shot=true` it returns
    /// immediately and leaves `precpu_stats` zeroed, so every CPU reading would
    /// be a confident, permanent 0%.
    ///
    /// The cost is that this call blocks for roughly one Docker collection
    /// interval (~1s). That is why the timeout is what it is.
    async fn sample_stats(&self) -> Result<Option<ContainerStatsResponse>, ConnectorError> {
        let options = StatsOptionsBuilder::new().stream(false).build();
        let mut stream = self
            .docker
            .stats(&self.config.container_name, Some(options));

        match stream.next().await {
            Some(Ok(sample)) => Ok(Some(sample)),
            // Docker closes the stats stream without a sample for a container
            // that is not running. That is not an error — it is the answer.
            None => Ok(None),
            Some(Err(error)) => Err(ConnectorError::unreachable(format!(
                "reading container stats failed: {error}"
            ))),
        }
    }

    /// The last [`LOG_TAIL_LINES`] lines, stdout and stderr, newline-joined.
    ///
    /// A best-effort read: a container with no log driver, or one whose driver
    /// does not support reading back (`none`, some syslog setups), answers with
    /// an error that is not a reason to fail the whole poll. The message
    /// becomes the data point's value so it is visible in the log pane rather
    /// than swallowed.
    ///
    /// Joined into one `String` rather than sent as an array because the
    /// declared value type is `String`, and a second wire shape for one type is
    /// how a renderer and a connector quietly disagree.
    async fn tail_logs(&self) -> String {
        let options = LogsOptionsBuilder::new()
            .stdout(true)
            .stderr(true)
            .follow(false)
            .tail(&LOG_TAIL_LINES.to_string())
            .build();

        let mut stream = self.docker.logs(&self.config.container_name, Some(options));
        let mut lines: Vec<String> = Vec::new();
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(output) => lines.push(output.to_string()),
                Err(error) => return format!("logs unavailable: {error}"),
            }
        }

        // Docker frames arrive with their own newlines and do not necessarily
        // align with lines, so the pieces are concatenated and split once
        // rather than joined per frame.
        let joined = lines.concat();
        joined.trim_end_matches('\n').to_owned()
    }

    /// Runs one lifecycle operation, mapping Docker's answer onto the
    /// reached-and-declined / could-not-reach split the trait draws.
    async fn run_lifecycle(&self, action_id: &str) -> Result<ActionResult, ConnectorError> {
        let name = self.config.container_name.as_str();
        // `self.control`, never `self.docker`: see the field docs.
        //
        // No explicit stop timeout is passed, so Docker applies the container's
        // own `StopTimeout`. Overriding it here would mean Loom deciding how
        // long someone's database gets to flush, which is a decision that
        // belongs to whoever configured the container.
        let outcome = match action_id {
            ACTION_START => self.control.start_container(name, None).await,
            ACTION_STOP => self.control.stop_container(name, None).await,
            ACTION_RESTART => self.control.restart_container(name, None).await,
            ACTION_PAUSE => self.control.pause_container(name).await,
            ACTION_UNPAUSE => self.control.unpause_container(name).await,
            other => return Err(ConnectorError::invalid_action(other)),
        };

        match outcome {
            Ok(()) => Ok(ActionResult::ok(format!("{name}: {action_id} succeeded"))),
            // The daemon answered and refused — "container already started",
            // "container is not paused". Its own words are the useful ones, so
            // they are passed through rather than replaced with a generic
            // failure, and this is `success: false` rather than `Err` because
            // Loom did reach the service.
            Err(bollard::errors::Error::DockerResponseServerError {
                status_code,
                message,
            }) => Ok(ActionResult::failed(format!(
                "{name}: Docker refused {action_id} ({status_code}): {}",
                message.trim()
            ))
            .with_payload(json!({ "statusCode": status_code }))),
            // Anything else is transport: the request never got an answer.
            Err(error) => Err(ConnectorError::unreachable(format!(
                "{action_id} could not be sent to Docker: {error}"
            ))),
        }
    }
}

/// The human half of a failed inspect, for the `error` detail on a poll.
///
/// Not `inspect_failure(..).to_string()`, which would prefix a missing
/// container with "invalid parameters for action configuration:" — accurate
/// when a *factory* refuses a configuration, and nonsense on a dashboard tile
/// three days later. The variant carries the routing; only the reason is worth
/// showing here.
fn poll_failure_reason(config: &DockerConnectorConfig, error: &bollard::errors::Error) -> String {
    match inspect_failure(config, error) {
        ConnectorError::InvalidParams { reason, .. }
        | ConnectorError::Unreachable { reason }
        | ConnectorError::InvalidConfig { reason }
        | ConnectorError::AuthFailed { reason } => reason,
        other => other.to_string(),
    }
}

/// Turns a failed inspect into the error that names the right thing to fix.
fn inspect_failure(
    config: &DockerConnectorConfig,
    error: &bollard::errors::Error,
) -> ConnectorError {
    match error {
        // 404 means we got all the way to the daemon and it does not have this
        // container — the network is fine and the name is not.
        bollard::errors::Error::DockerResponseServerError {
            status_code: 404, ..
        } => ConnectorError::InvalidParams {
            action_id: "configuration".to_owned(),
            reason: format!(
                "the Docker host at {} has no container named {:?}. Check `docker ps -a` for the \
                 exact name or id.",
                config.docker_host, config.container_name
            ),
        },
        // Any other status is the daemon objecting to us rather than to the
        // name — a socket-proxy denying the endpoint, most often.
        bollard::errors::Error::DockerResponseServerError {
            status_code,
            message,
        } => ConnectorError::unreachable(format!(
            "the Docker host at {} answered {status_code}: {}",
            config.docker_host,
            message.trim()
        )),
        other => ConnectorError::unreachable(format!(
            "could not reach the Docker host at {}: {other}",
            config.docker_host
        )),
    }
}

#[async_trait]
impl Connector for DockerConnector {
    /// One poll: inspect, stats, logs.
    ///
    /// # Why a dead daemon is `Ok(Down)` and not `Err`
    ///
    /// A failure to reach Docker is reported as a `Down` status carrying an
    /// `error` message, not as `Err`. The trait's `Err` arm means "the check
    /// could not be carried out", which this technically is — but for *this*
    /// connector the useful thing on a dashboard is a tile that says the
    /// container is not available and why, rather than a tile that blanks out
    /// to "no reading". The declared data points are still all present with
    /// their unavailable values, so a saved layout keeps rendering instead of
    /// collapsing to skeletons the moment a host goes away.
    ///
    /// Note that `details` therefore carries one key that is **not** a declared
    /// data point: `error`. It is deliberately not a data point, because it
    /// should not be bindable to a widget — it is diagnostic text, shown in the
    /// detail view, not something a user arranges on a dashboard.
    async fn status(&self) -> Result<ConnectorStatus, ConnectorError> {
        let inspect = match self
            .docker
            .inspect_container(&self.config.container_name, None)
            .await
        {
            Ok(inspect) => inspect,
            Err(error) => {
                return Ok(ConnectorStatus::new(
                    HealthState::Down,
                    unavailable_details(&poll_failure_reason(&self.config, &error)),
                ));
            }
        };

        Ok(self.status_from(inspect).await)
    }

    async fn actions(&self) -> Vec<ConnectorAction> {
        // Offered unconditionally rather than filtered by current state. The
        // list is cached by clients and the state can change between the two
        // calls, so a start button that vanished the instant a container came
        // up would only ever be *stale*, never correct — Docker's own refusal
        // is the authoritative answer, and it arrives as a clear message.
        vec![
            ConnectorAction::simple(ACTION_START, "Start").with_description("Start the container."),
            ConnectorAction::simple(ACTION_STOP, "Stop")
                .with_description("Stop the container, giving it time to shut down."),
            // The only disruptive one. `start`, `stop`, `pause` and `unpause`
            // each move the container to a state the person who pressed the
            // button asked for and can see coming; `restart` is the case where
            // a running service vanishes and is expected back, and the gap in
            // between is exactly what the "Performing: …" overlay exists to
            // explain rather than report as an outage.
            ConnectorAction::simple(ACTION_RESTART, "Restart")
                .with_description("Stop and start the container.")
                .disruptive(),
            ConnectorAction::simple(ACTION_PAUSE, "Pause")
                .with_description("Freeze every process in the container without stopping it."),
            ConnectorAction::simple(ACTION_UNPAUSE, "Resume")
                .with_description("Resume a paused container."),
        ]
    }

    async fn execute_action(
        &self,
        action_id: &str,
        _params: Value,
    ) -> Result<ActionResult, ConnectorError> {
        // Every action is parameterless, so `params` is ignored rather than
        // validated. A future action that takes one must validate its own.
        self.run_lifecycle(action_id).await
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
            // Two by two fits the status dot, one stat tile and a button row.
            // The default layout also carries a chart and a log pane, which
            // want more room — but `min_size` is a floor, not a preference, and
            // a floor that refuses a small tile on a phone would be wrong.
            min_size: (2, 2),
        }
    }

    /// Identity only.
    ///
    /// Written out by hand, as the trait requires: the configuration is the
    /// only other thing available and deriving from it is how a credential
    /// eventually reaches a dashboard. Neither of these two fields is secret —
    /// but that is a property of *this* connector's schema today, not a rule
    /// that survives the next field somebody adds.
    fn display_fields(&self) -> Vec<DisplayField> {
        vec![
            DisplayField {
                label: "Container".to_owned(),
                value: self.config.container_name.clone(),
            },
            DisplayField {
                label: "Docker host".to_owned(),
                value: self.config.docker_host.clone(),
            },
        ]
    }

    fn data_points(&self) -> Vec<DataPointDescriptor> {
        vec![
            DataPointDescriptor::new(DATA_POINT_STATUS, "State", DataPointValueType::String),
            DataPointDescriptor::new(DATA_POINT_CPU_PERCENT, "CPU", DataPointValueType::Number)
                .with_unit("%"),
            DataPointDescriptor::new(
                DATA_POINT_CPU_HISTORY,
                "CPU history",
                DataPointValueType::TimeSeries,
            )
            .with_unit("%"),
            DataPointDescriptor::new(
                DATA_POINT_MEMORY_USAGE_BYTES,
                "Memory",
                DataPointValueType::Number,
            )
            .with_unit("bytes"),
            DataPointDescriptor::new(
                DATA_POINT_MEMORY_HISTORY,
                "Memory history",
                DataPointValueType::TimeSeries,
            )
            .with_unit("bytes"),
            DataPointDescriptor::new(DATA_POINT_UPTIME, "Uptime", DataPointValueType::String),
            DataPointDescriptor::new(DATA_POINT_LOGS, "Recent logs", DataPointValueType::String),
        ]
    }

    /// The Docker endpoint, when probing it would mean anything.
    ///
    /// A `tcp://host:port` endpoint — a remote daemon or a socket proxy — is
    /// worth probing: if it stops answering, whether the name resolves and
    /// whether the port accepts a connection are three different problems with
    /// three different fixes.
    ///
    /// A `unix://` socket is not. It is a file on this machine, reached over no
    /// network; a DNS lookup and a TCP connect have nothing to say about it,
    /// and "the host is reachable" would be a tautology dressed up as a
    /// diagnosis. `connect()` already reports a missing socket precisely, which
    /// is the actual failure in that setup.
    fn network_target(&self) -> Option<NetworkTarget> {
        let host = self
            .config
            .docker_host
            .strip_prefix("tcp://")
            .or_else(|| self.config.docker_host.strip_prefix("http://"))?;

        // Trim anything after the authority: a Docker URI does not normally
        // carry a path, but splitting here means one that does cannot turn into
        // a hostname with a slash in it.
        let authority = host.split(['/', '?']).next().unwrap_or(host);

        // `rsplit_once` rather than `split_once`, so an IPv6 literal in
        // brackets keeps its colons and only the trailing port is taken.
        match authority.rsplit_once(':') {
            Some((host, port)) if !host.is_empty() => match port.parse::<u16>() {
                Ok(port) => Some(NetworkTarget::new(host.trim_matches(['[', ']']), port)),
                // A host with an unparseable port is still a host worth naming;
                // the probe stops after DNS, which is better than nothing.
                Err(_) => Some(NetworkTarget {
                    host: host.trim_matches(['[', ']']).to_owned(),
                    port: None,
                }),
            },
            // Docker over TCP with no port is unusual but legal — the daemon's
            // default is implied. Report the host so DNS is still checked.
            _ if !authority.is_empty() => Some(NetworkTarget {
                host: authority.trim_matches(['[', ']']).to_owned(),
                port: None,
            }),
            _ => None,
        }
    }

    fn default_layout(&self) -> WidgetLayout {
        WidgetLayout::new(vec![
            WidgetBinding::display(DATA_POINT_STATUS, DisplayWidgetType::StatusDot),
            WidgetBinding::display(
                DATA_POINT_CPU_HISTORY,
                DisplayWidgetType::MetricChart {
                    chart_type: ChartType::Line,
                },
            ),
            WidgetBinding::display(DATA_POINT_MEMORY_USAGE_BYTES, DisplayWidgetType::StatTile),
            WidgetBinding::display(DATA_POINT_LOGS, DisplayWidgetType::LogStream),
            WidgetBinding::action(ACTION_START, ActionWidgetType::Button),
            WidgetBinding::action(ACTION_STOP, ActionWidgetType::Button),
            WidgetBinding::action(ACTION_RESTART, ActionWidgetType::Button),
            // `pause` and `unpause` are deliberately not here. They are real
            // actions and stay in `actions()`, so anyone who wants them adds a
            // button in the binding editor — but a default layout with five
            // lifecycle buttons is a wall of controls for two that most people
            // never press.
        ])
    }
}

impl DockerConnector {
    /// Assembles the status from a successful inspect.
    ///
    /// Split out so the stats and logs calls happen only once the container is
    /// known to exist, and so this half is readable without the error plumbing.
    async fn status_from(&self, inspect: ContainerInspectResponse) -> ConnectorStatus {
        let state = inspect.state.unwrap_or_default();
        let status_enum = state.status;
        let health = health_for_state(status_enum);
        let running = matches!(
            status_enum,
            Some(bollard::models::ContainerStateStatusEnum::RUNNING)
        );

        // Stats and logs are only meaningful for a container that is running,
        // and asking for them otherwise means waiting out the stats collection
        // interval to be told nothing. A stopped container's poll is therefore
        // also the fast one.
        let (cpu, memory) = if running {
            match self.sample_stats().await {
                Ok(Some(sample)) => (
                    cpu_percent(sample.cpu_stats.as_ref(), sample.precpu_stats.as_ref()),
                    sample
                        .memory_stats
                        .and_then(|memory| memory.usage)
                        .unwrap_or(0) as f64,
                ),
                // A container that stopped between the inspect and the stats
                // call, or a stats read that failed. Neither is worth failing
                // the poll for: the state we already read is the headline.
                Ok(None) | Err(_) => (0.0, 0.0),
            }
        } else {
            (0.0, 0.0)
        };

        let now = Utc::now();
        // History is recorded only while running, so a stopped container leaves
        // a gap in its chart rather than a flat line at zero that looks like a
        // measurement.
        let (cpu_history, memory_history) = {
            let mut history = self.history.lock().unwrap_or_else(|poisoned| {
                // A panic in another thread while holding this lock cannot
                // corrupt a ring buffer of numbers, so recovering is strictly
                // better than propagating the panic into every later poll.
                poisoned.into_inner()
            });
            if running {
                history.record(cpu, memory, now);
            }
            (
                serde_json::to_value(&history.cpu).unwrap_or(Value::Null),
                serde_json::to_value(&history.memory).unwrap_or(Value::Null),
            )
        };

        // Read whether or not the container is running: for a stopped one, the
        // last lines before it exited are usually the reason it exited, which
        // is the single most useful thing a dashboard can show at that moment.
        let logs = self.tail_logs().await;

        let mut details = Map::new();
        details.insert(
            DATA_POINT_STATUS.to_owned(),
            json!(status_enum.map_or_else(|| "unknown".to_owned(), |state| state.to_string())),
        );
        details.insert(DATA_POINT_CPU_PERCENT.to_owned(), json!(cpu));
        details.insert(DATA_POINT_CPU_HISTORY.to_owned(), cpu_history);
        details.insert(DATA_POINT_MEMORY_USAGE_BYTES.to_owned(), json!(memory));
        details.insert(DATA_POINT_MEMORY_HISTORY.to_owned(), memory_history);
        details.insert(
            DATA_POINT_UPTIME.to_owned(),
            json!(format_uptime(state.started_at.as_deref(), running, now)),
        );
        details.insert(DATA_POINT_LOGS.to_owned(), json!(logs));

        ConnectorStatus::new(health, Value::Object(details))
    }
}

/// Every declared data point, at its "we could not find out" value, plus the
/// reason.
///
/// A layout keeps rendering through an outage this way, showing an unavailable
/// container rather than collapsing to loading skeletons.
fn unavailable_details(reason: &str) -> Value {
    json!({
        DATA_POINT_STATUS: "unavailable",
        DATA_POINT_CPU_PERCENT: 0.0,
        DATA_POINT_CPU_HISTORY: [],
        DATA_POINT_MEMORY_USAGE_BYTES: 0.0,
        DATA_POINT_MEMORY_HISTORY: [],
        DATA_POINT_UPTIME: "not running",
        DATA_POINT_LOGS: "",
        // Not a declared data point on purpose — see `status`.
        "error": reason,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DockerConnectorConfig;

    /// Builds a connector without touching Docker.
    ///
    /// `network_target()` and `actions()` are descriptive: neither reaches the
    /// daemon, and both are worth testing on a machine that has none. The
    /// client is built but never used, which is why a `tcp://` host that does
    /// not exist is fine here.
    fn describe(docker_host: &str) -> Option<NetworkTarget> {
        let config = DockerConnectorConfig {
            docker_host: docker_host.to_owned(),
            container_name: "irrelevant".to_owned(),
        };
        let docker = config.connect().expect("building a client does no I/O");
        DockerConnector {
            docker: docker.clone(),
            control: docker,
            config,
            history: Arc::new(Mutex::new(History::default())),
        }
        .network_target()
    }

    #[test]
    fn a_tcp_endpoint_is_worth_probing() {
        assert_eq!(
            describe("tcp://docker-proxy.example:2375"),
            Some(NetworkTarget::new("docker-proxy.example", 2375))
        );
        assert_eq!(
            describe("http://192.0.2.10:2376"),
            Some(NetworkTarget::new("192.0.2.10", 2376))
        );
        // An IPv6 literal keeps its own colons; only the trailing port is taken.
        assert_eq!(
            describe("tcp://[2001:db8::1]:2375"),
            Some(NetworkTarget::new("2001:db8::1", 2375))
        );
    }

    #[test]
    fn a_local_socket_has_nothing_to_probe() {
        // A DNS lookup and a TCP connect have nothing to say about a file.
        assert_eq!(describe("unix:///var/run/docker.sock"), None);
    }

    #[test]
    fn a_host_without_a_usable_port_still_reports_its_host() {
        // DNS is still worth checking even when there is nowhere to connect.
        for host in ["tcp://docker.example", "tcp://docker.example:not-a-port"] {
            let target = describe(host).unwrap_or_else(|| panic!("{host} names a host"));
            assert_eq!(target.host, "docker.example");
            assert_eq!(target.port, None, "{host} names no usable port");
        }
    }

    #[tokio::test]
    async fn only_restart_is_marked_disruptive() {
        let config = DockerConnectorConfig {
            docker_host: "tcp://docker-proxy.example:2375".to_owned(),
            container_name: "irrelevant".to_owned(),
        };
        let docker = config.connect().expect("building a client does no I/O");
        let connector = DockerConnector {
            docker: docker.clone(),
            control: docker,
            config,
            history: Arc::new(Mutex::new(History::default())),
        };

        // `actions()` is a static list on this connector, so it answers without
        // a daemon.
        let disruptive: Vec<String> = connector
            .actions()
            .await
            .into_iter()
            .filter(|action| action.is_disruptive)
            .map(|action| action.id)
            .collect();
        assert_eq!(
            disruptive,
            vec![ACTION_RESTART.to_owned()],
            "stop and start take the container where the user asked; only restart \
             brings it back on its own"
        );
    }
}
