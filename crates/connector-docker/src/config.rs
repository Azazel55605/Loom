//! What this connector needs to be told, and how it turns that into a client.

use bollard::{Docker, API_DEFAULT_VERSION};
use loom_core::connector::ConnectorError;
use serde::Deserialize;
use serde_json::{json, Value};

/// Timeout, in seconds, for reads: status polls, inspects, logs, stats.
///
/// Bollard's default is 120, which is right for `docker build` and badly wrong
/// for a status poll: an unreachable host would hold a poll open for two
/// minutes and stall the "add connector" request that validates it. Ten seconds
/// is generous for a local socket and long enough for a proxy on a slow link,
/// and it is under the poll interval so a hung host cannot pile polls up.
pub(crate) const READ_TIMEOUT_SECONDS: u64 = 10;

/// Timeout, in seconds, for lifecycle actions.
///
/// **Much longer than the read timeout, and it has to be.** `docker stop` sends
/// SIGTERM and then waits out the container's stop grace period — ten seconds
/// by default, and routinely raised to thirty or sixty for databases and
/// anything that flushes on shutdown — before it sends SIGKILL and answers.
/// `restart` does the same and then starts the container.
///
/// Sharing the read timeout here is a bug that looks like a network problem:
/// the daemon carries the stop out, Loom gives up waiting, and the user is told
/// the request "could not be sent to Docker" about a container that is now
/// stopped. Ninety seconds covers a generous grace period with room to spare;
/// a container that takes longer than that to stop has a real problem, and
/// timing out is then the honest answer.
pub(crate) const CONTROL_TIMEOUT_SECONDS: u64 = 90;

/// The default Docker endpoint: the local daemon socket.
pub const DEFAULT_DOCKER_HOST: &str = "unix:///var/run/docker.sock";

/// Default interval between update checks, in minutes.
///
/// Six hours. Not because a check is expensive — a manifest `HEAD` is a version
/// check, which Docker Hub's own documentation excludes from its pull limit —
/// but because the limit is enforced per source address, and a homelab's
/// address is shared with everything else on it: the CI runner, the compose
/// pull, the other update tool. Six hours keeps Loom's contribution to that
/// shared budget negligible for any realistic number of containers, and an
/// image people actually update does not change faster than a working day.
/// Anyone who wants faster can set it; the *default* should not be the thing
/// that puts a household over someone else's limit.
pub const DEFAULT_CHECK_INTERVAL_MINUTES: u64 = 360;

/// Floor on the configurable interval, in minutes.
///
/// A one-minute check interval against a public registry is not a
/// configuration, it is a mistake, and it would be made once and then forgotten
/// on a machine that shares an address with other people.
pub const MIN_CHECK_INTERVAL_MINUTES: u64 = 5;

/// A validated configuration for one Docker host.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerConnectorConfig {
    /// Connection URI — `unix:///path/to.sock` or `tcp://host:port`.
    pub docker_host: String,
    /// Whether the platform's update scheduler should check this instance's
    /// containers at all. Off by default: checking reaches out to third-party
    /// registries, and that is not something to start doing on someone's behalf
    /// because they added a connector.
    pub check_for_updates: bool,
    /// Minutes between checks, floored at [`MIN_CHECK_INTERVAL_MINUTES`].
    pub check_interval_minutes: u64,
    /// Whether a found update should be applied without anyone asking.
    pub auto_apply_updates: bool,
    /// `HH:MM` local wall-clock time to apply updates at, or `None` to apply as
    /// soon as one is found. A maintenance window, in other words: the
    /// difference between a service restarting at 03:00 and restarting during
    /// the film.
    pub auto_apply_at_time: Option<AutoApplyTime>,
    /// Whether this instance is checked and reported on but never auto-applied.
    /// Independent of [`auto_apply_updates`](Self::auto_apply_updates) on
    /// purpose: the common case is one host where automatic updates are fine
    /// and one container on it where they are not.
    pub exclude_from_auto_update: bool,
}

impl Default for DockerConnectorConfig {
    /// The local socket, no update checking, nothing automatic.
    ///
    /// Matches what [`DockerConnectorConfig::from_value`] produces from an
    /// empty configuration, so the two cannot drift apart.
    fn default() -> Self {
        Self {
            docker_host: DEFAULT_DOCKER_HOST.to_owned(),
            check_for_updates: false,
            check_interval_minutes: DEFAULT_CHECK_INTERVAL_MINUTES,
            auto_apply_updates: false,
            auto_apply_at_time: None,
            exclude_from_auto_update: false,
        }
    }
}

/// A validated `HH:MM` wall-clock time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AutoApplyTime {
    /// Hour, 0–23.
    pub hour: u32,
    /// Minute, 0–59.
    pub minute: u32,
}

impl AutoApplyTime {
    /// Parses `HH:MM`, rejecting anything that is not a real time of day.
    ///
    /// An empty or blank value is `Ok(None)` rather than an error: the field is
    /// optional and "cleared" is how a user turns a maintenance window off.
    pub fn parse(value: &str) -> Result<Option<Self>, ConnectorError> {
        let value = value.trim();
        if value.is_empty() {
            return Ok(None);
        }

        let refused = || {
            ConnectorError::invalid_config(format!(
                "autoApplyAtTime must be a 24-hour HH:MM time, got {value:?}"
            ))
        };
        let (hour, minute) = value.split_once(':').ok_or_else(refused)?;
        let hour: u32 = hour.parse().map_err(|_| refused())?;
        let minute: u32 = minute.parse().map_err(|_| refused())?;
        if hour > 23 || minute > 59 {
            return Err(refused());
        }

        Ok(Some(Self { hour, minute }))
    }

    /// Minutes since midnight, for comparing against a wall clock.
    pub fn minutes_since_midnight(&self) -> u32 {
        self.hour * 60 + self.minute
    }
}

/// The wire shape, before validation.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawConfig {
    docker_host: Option<String>,
    #[serde(default)]
    check_for_updates: bool,
    #[serde(default)]
    check_interval_minutes: Option<u64>,
    #[serde(default)]
    auto_apply_updates: bool,
    #[serde(default)]
    auto_apply_at_time: Option<String>,
    #[serde(default)]
    exclude_from_auto_update: bool,
}

impl DockerConnectorConfig {
    /// Parses and validates a stored configuration.
    ///
    /// Shape and emptiness only — whether the host answers and whether the
    /// container exists are questions for [`connect`], because they need I/O
    /// and because the two failures need different error variants.
    pub fn from_value(config: Value) -> Result<Self, ConnectorError> {
        let raw: RawConfig = match config {
            Value::Null => RawConfig {
                docker_host: None,
                check_for_updates: false,
                check_interval_minutes: None,
                auto_apply_updates: false,
                auto_apply_at_time: None,
                exclude_from_auto_update: false,
            },
            other => serde_json::from_value(other)
                .map_err(|error| ConnectorError::invalid_config(error.to_string()))?,
        };

        let docker_host = raw
            .docker_host
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_DOCKER_HOST.to_owned());

        let auto_apply_at_time = match raw.auto_apply_at_time.as_deref() {
            Some(value) => AutoApplyTime::parse(value)?,
            None => None,
        };

        Ok(Self {
            docker_host,
            check_for_updates: raw.check_for_updates,
            // Clamped rather than refused. The value arrives from a generated
            // number field, and the failure mode being guarded against is a
            // careless 1, not a hostile one — refusing it would fail the whole
            // connector over a field that has an obviously right nearby value.
            check_interval_minutes: raw
                .check_interval_minutes
                .unwrap_or(DEFAULT_CHECK_INTERVAL_MINUTES)
                .max(MIN_CHECK_INTERVAL_MINUTES),
            auto_apply_updates: raw.auto_apply_updates,
            auto_apply_at_time,
            exclude_from_auto_update: raw.exclude_from_auto_update,
        })
    }

    /// Whether the scheduler should be checking this instance at all.
    pub fn update_checks_enabled(&self) -> bool {
        self.check_for_updates
    }

    /// Whether a found update may be applied without anyone asking.
    ///
    /// Both switches have to agree: the exclusion is what lets one container be
    /// left alone on a host that otherwise updates itself.
    pub fn auto_apply_enabled(&self) -> bool {
        self.auto_apply_updates && !self.exclude_from_auto_update
    }
}

impl DockerConnectorConfig {
    /// Builds a Docker client for this configuration.
    ///
    /// The constructor is chosen from the URI scheme rather than from
    /// `connect_with_defaults`, which reads `DOCKER_HOST` from the process
    /// environment — an ambient value that would silently override what the
    /// user configured, and would make two instances on one server unable to
    /// point at different hosts.
    ///
    /// This performs no I/O. It fails only on a scheme this connector does not
    /// offer, or on a `unix://` path that is not there — which bollard checks
    /// eagerly, and which is the single most common misconfiguration (the
    /// socket not being mounted into Loom's own container).
    ///
    /// `timeout_seconds` differs by what the client is for — see
    /// [`READ_TIMEOUT_SECONDS`] and [`CONTROL_TIMEOUT_SECONDS`].
    pub fn connect_with_timeout(&self, timeout_seconds: u64) -> Result<Docker, ConnectorError> {
        let host = self.docker_host.as_str();

        if let Some(path) = host.strip_prefix("unix://") {
            return Docker::connect_with_unix(path, timeout_seconds, API_DEFAULT_VERSION).map_err(
                |error| {
                    ConnectorError::unreachable(format!(
                        "could not open the Docker socket at {path}: {error}. If Loom is running \
                         in a container, the socket has to be mounted into it."
                    ))
                },
            );
        }

        if host.starts_with("tcp://") || host.starts_with("http://") {
            return Docker::connect_with_http(host, timeout_seconds, API_DEFAULT_VERSION).map_err(
                |error| {
                    ConnectorError::unreachable(format!(
                        "could not reach the Docker host at {host}: {error}"
                    ))
                },
            );
        }

        // Named as configuration rather than reachability: nothing was tried,
        // so calling it unreachable would send someone to check their network.
        Err(ConnectorError::invalid_config(format!(
            "dockerHost must start with unix:// or tcp://, got {host:?}"
        )))
    }

    /// A client for reads, on the short timeout.
    pub fn connect(&self) -> Result<Docker, ConnectorError> {
        self.connect_with_timeout(READ_TIMEOUT_SECONDS)
    }

    /// A client for lifecycle actions, on the long timeout.
    pub fn connect_for_control(&self) -> Result<Docker, ConnectorError> {
        self.connect_with_timeout(CONTROL_TIMEOUT_SECONDS)
    }
}

/// The JSON Schema the clients generate the setup form from.
pub fn config_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Docker configuration",
        "type": "object",
        "properties": {
            "dockerHost": {
                "title": "Docker host",
                "type": "string",
                "minLength": 1,
                "default": DEFAULT_DOCKER_HOST,
                "description": "Docker connection URI. Use `unix:///var/run/docker.sock` for a \
                                local socket, or `tcp://host:port` for a remote host or a \
                                Docker socket-proxy container."
            },
            "checkForUpdates": {
                "title": "Check for updates",
                "type": "boolean",
                "default": false,
                "description": "Periodically ask the image registry whether this host's \
                                containers are running an outdated image. Off by default: \
                                checking contacts third-party registries, which is not \
                                something Loom should start doing on your behalf."
            },
            "checkIntervalMinutes": {
                "title": "Check interval (minutes)",
                "type": "integer",
                "minimum": MIN_CHECK_INTERVAL_MINUTES,
                "default": DEFAULT_CHECK_INTERVAL_MINUTES,
                "description": "Minutes between update checks. The default is six hours. \
                                Registries rate-limit by source address, and yours is shared \
                                with everything else on your network, so checking every few \
                                minutes spends a budget you do not exclusively own — for images \
                                that change no faster than a working day."
            },
            "autoApplyUpdates": {
                "title": "Apply updates automatically",
                "type": "boolean",
                "default": false,
                "description": "Recreate a container automatically when a newer image is \
                                found. The container is stopped, removed, and recreated from \
                                the new image with its existing configuration, which means a \
                                short outage without anyone present."
            },
            "autoApplyAtTime": {
                "title": "Apply at (HH:MM)",
                "type": "string",
                "description": "24-hour HH:MM local time to apply updates at — a maintenance \
                                window. Leave empty to apply as soon as an update is found."
            },
            "excludeFromAutoUpdate": {
                "title": "Never apply automatically",
                "type": "boolean",
                "default": false,
                "description": "Keep checking and reporting available updates for this host, \
                                but never apply one automatically. Overrides automatic \
                                updates, so one host can be left alone without turning the \
                                feature off everywhere."
            }
        },
        "required": ["dockerHost"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_defaults_and_stale_container_configuration_is_ignored() {
        let config = DockerConnectorConfig::from_value(json!({ "containerName": "web" }))
            .expect("the stale per-container key must not prevent startup");
        assert_eq!(config.docker_host, DEFAULT_DOCKER_HOST);
        assert_eq!(
            DockerConnectorConfig::from_value(Value::Null)
                .expect("null uses defaults")
                .docker_host,
            DEFAULT_DOCKER_HOST
        );
    }

    #[test]
    fn values_are_trimmed_and_unknown_keys_are_ignored_for_stale_rows() {
        let config = DockerConnectorConfig::from_value(
            json!({ "containerName": "  web  ", "dockerHost": "  tcp://example:2375  " }),
        )
        .expect("legacy per-container configuration remains loadable");
        assert_eq!(config.docker_host, "tcp://example:2375");

        // Pre-release rows may carry fields from an older connector shape.
        // Serde intentionally ignores them instead of making the whole runtime
        // skip a connector that still has a valid Docker endpoint.
        let config = DockerConnectorConfig::from_value(
            json!({ "containerName": "web", "dockerHostt": "tcp://example:2375" }),
        )
        .expect("unused fields are harmless");
        assert_eq!(config.docker_host, DEFAULT_DOCKER_HOST);
    }

    #[test]
    fn the_scheme_selects_the_transport_and_anything_else_is_refused() {
        // A `tcp://` host builds a client without touching the network, so this
        // succeeds against a host that does not exist.
        let config = DockerConnectorConfig {
            docker_host: "tcp://docker-proxy.example:2375".to_owned(),
            ..DockerConnectorConfig::default()
        };
        assert!(config.connect().is_ok());

        // A socket path that is not there fails eagerly, and as *unreachable* —
        // the fix is at the infrastructure level, usually a missing bind mount.
        let config = DockerConnectorConfig {
            docker_host: "unix:///nonexistent/loom-test/docker.sock".to_owned(),
            ..DockerConnectorConfig::default()
        };
        assert!(matches!(
            config.connect(),
            Err(ConnectorError::Unreachable { .. })
        ));

        // A scheme this connector does not offer is a *configuration* error:
        // nothing was tried, so "unreachable" would send someone to check their
        // network for no reason.
        for unsupported in [
            "ssh://example",
            "npipe:////./pipe/docker_engine",
            "example:2375",
        ] {
            let config = DockerConnectorConfig {
                docker_host: unsupported.to_owned(),
                ..DockerConnectorConfig::default()
            };
            assert!(
                matches!(config.connect(), Err(ConnectorError::InvalidConfig { .. })),
                "{unsupported} should be refused as configuration"
            );
        }
    }

    #[test]
    fn update_settings_default_to_doing_nothing_and_are_read_when_present() {
        let quiet = DockerConnectorConfig::from_value(json!({})).expect("an empty configuration");
        assert!(!quiet.check_for_updates);
        assert!(!quiet.auto_apply_updates);
        assert!(!quiet.exclude_from_auto_update);
        assert_eq!(quiet.auto_apply_at_time, None);
        assert_eq!(quiet.check_interval_minutes, DEFAULT_CHECK_INTERVAL_MINUTES);
        assert!(!quiet.update_checks_enabled());

        let configured = DockerConnectorConfig::from_value(json!({
            "checkForUpdates": true,
            "checkIntervalMinutes": 30,
            "autoApplyUpdates": true,
            "autoApplyAtTime": "03:30",
        }))
        .expect("a full configuration");
        assert!(configured.update_checks_enabled());
        assert!(configured.auto_apply_enabled());
        assert_eq!(configured.check_interval_minutes, 30);
        assert_eq!(
            configured.auto_apply_at_time,
            Some(AutoApplyTime {
                hour: 3,
                minute: 30
            })
        );

        // The exclusion outranks the switch, which is the point of having
        // both: one host updates itself, one container on it does not.
        let excluded = DockerConnectorConfig::from_value(json!({
            "checkForUpdates": true,
            "autoApplyUpdates": true,
            "excludeFromAutoUpdate": true,
        }))
        .expect("a configuration with an exclusion");
        assert!(excluded.update_checks_enabled());
        assert!(!excluded.auto_apply_enabled());

        // An interval nobody should be using is clamped rather than refused:
        // the field comes from a generated number input, and failing the whole
        // connector over a careless 1 helps no one.
        assert_eq!(
            DockerConnectorConfig::from_value(json!({ "checkIntervalMinutes": 1 }))
                .expect("clamped, not refused")
                .check_interval_minutes,
            MIN_CHECK_INTERVAL_MINUTES
        );
    }

    #[test]
    fn a_maintenance_window_is_a_real_time_of_day_or_nothing_at_all() {
        assert_eq!(
            AutoApplyTime::parse("00:00").unwrap(),
            Some(AutoApplyTime { hour: 0, minute: 0 })
        );
        assert_eq!(
            AutoApplyTime::parse("23:59")
                .unwrap()
                .unwrap()
                .minutes_since_midnight(),
            23 * 60 + 59
        );

        // Cleared is how a window is turned off, so blank is not an error.
        assert_eq!(AutoApplyTime::parse("").unwrap(), None);
        assert_eq!(AutoApplyTime::parse("   ").unwrap(), None);

        for invalid in ["24:00", "12:60", "noon", "12", "12:", ":30", "-1:00"] {
            assert!(
                AutoApplyTime::parse(invalid).is_err(),
                "{invalid} is not a time of day"
            );
        }
    }

    #[test]
    fn the_schema_describes_exactly_the_keys_the_parser_accepts() {
        let schema = config_schema();
        let properties = schema["properties"].as_object().expect("properties");
        let mut names = properties.keys().map(String::as_str).collect::<Vec<_>>();
        names.sort_unstable();
        assert_eq!(
            names,
            vec![
                "autoApplyAtTime",
                "autoApplyUpdates",
                "checkForUpdates",
                "checkIntervalMinutes",
                "dockerHost",
                "excludeFromAutoUpdate",
            ]
        );
        // Only the endpoint is required; every update setting has a working
        // default, so an existing instance keeps loading unchanged.
        assert_eq!(schema["required"], json!(["dockerHost"]));
        assert_eq!(properties["dockerHost"]["default"], DEFAULT_DOCKER_HOST);
        assert_eq!(
            properties["checkIntervalMinutes"]["default"],
            json!(DEFAULT_CHECK_INTERVAL_MINUTES)
        );
        // Every field carries a real title and description: the setup form is
        // generated from this schema and nothing else explains what these do.
        // Without a title the generated form labels the field with its raw
        // camelCase key, which is the connector author's spelling, not a name.
        for (name, property) in properties {
            assert!(
                property["title"]
                    .as_str()
                    .is_some_and(|text| !text.is_empty()),
                "{name} needs a title a form can label it with"
            );
            assert!(
                property["description"]
                    .as_str()
                    .is_some_and(|text| text.len() > 40),
                "{name} needs a description a form can show"
            );
        }
        // New clients should only offer the current field. The parser is
        // intentionally more tolerant so stale persisted rows still load.
        assert_eq!(schema["additionalProperties"], json!(false));
    }
}
