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

/// A validated configuration for one container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DockerConnectorConfig {
    /// Connection URI — `unix:///path/to.sock` or `tcp://host:port`.
    pub docker_host: String,
    /// Exact container name or id. Not a pattern: this connector manages one
    /// container, and a prefix that matched two would silently pick one.
    pub container_name: String,
}

/// The wire shape, before validation.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawConfig {
    docker_host: Option<String>,
    container_name: Option<String>,
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
                container_name: None,
            },
            other => serde_json::from_value(other)
                .map_err(|error| ConnectorError::invalid_config(error.to_string()))?,
        };

        let docker_host = raw
            .docker_host
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_DOCKER_HOST.to_owned());

        // No default: there is no sensible container to monitor if nobody said
        // which one, and picking the first one on the host would be a guess
        // that looks like a feature until it picks the wrong one.
        let container_name = raw
            .container_name
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ConnectorError::invalid_config(
                    "containerName is required: name the container to monitor",
                )
            })?;

        Ok(Self {
            docker_host,
            container_name,
        })
    }

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
        "title": "Docker container configuration",
        "type": "object",
        "properties": {
            "dockerHost": {
                "type": "string",
                "minLength": 1,
                "default": DEFAULT_DOCKER_HOST,
                "description": "Docker connection URI. Use `unix:///var/run/docker.sock` for a \
                                local socket, or `tcp://host:port` for a remote host or a \
                                docker-socket-proxy container."
            },
            "containerName": {
                "type": "string",
                "minLength": 1,
                "description": "Exact container name or ID to monitor and control."
            }
        },
        "required": ["dockerHost", "containerName"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_container_name_is_required_and_the_host_defaults() {
        let config = DockerConnectorConfig::from_value(json!({ "containerName": "web" }))
            .expect("containerName alone is enough");
        assert_eq!(config.docker_host, DEFAULT_DOCKER_HOST);
        assert_eq!(config.container_name, "web");

        for missing in [json!({}), Value::Null, json!({ "containerName": "   " })] {
            let error = DockerConnectorConfig::from_value(missing)
                .expect_err("a connector with no container has nothing to monitor");
            assert!(
                matches!(error, ConnectorError::InvalidConfig { ref reason } if reason.contains("containerName")),
                "the refusal must name the field: {error}"
            );
        }
    }

    #[test]
    fn values_are_trimmed_and_unknown_keys_are_refused() {
        let config = DockerConnectorConfig::from_value(
            json!({ "containerName": "  web  ", "dockerHost": "  tcp://example:2375  " }),
        )
        .expect("surrounding whitespace is a typo, not a different container");
        assert_eq!(config.container_name, "web");
        assert_eq!(config.docker_host, "tcp://example:2375");

        // A misspelled key is a configuration that will not do what its author
        // meant, so it is refused rather than silently defaulted.
        let error = DockerConnectorConfig::from_value(
            json!({ "containerName": "web", "dockerHostt": "tcp://example:2375" }),
        )
        .expect_err("unknown keys must be refused");
        assert!(matches!(error, ConnectorError::InvalidConfig { .. }));
    }

    #[test]
    fn the_scheme_selects_the_transport_and_anything_else_is_refused() {
        // A `tcp://` host builds a client without touching the network, so this
        // succeeds against a host that does not exist.
        let config = DockerConnectorConfig {
            docker_host: "tcp://docker-proxy.example:2375".to_owned(),
            container_name: "web".to_owned(),
        };
        assert!(config.connect().is_ok());

        // A socket path that is not there fails eagerly, and as *unreachable* —
        // the fix is at the infrastructure level, usually a missing bind mount.
        let config = DockerConnectorConfig {
            docker_host: "unix:///nonexistent/loom-test/docker.sock".to_owned(),
            container_name: "web".to_owned(),
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
                container_name: "web".to_owned(),
            };
            assert!(
                matches!(config.connect(), Err(ConnectorError::InvalidConfig { .. })),
                "{unsupported} should be refused as configuration"
            );
        }
    }

    #[test]
    fn the_schema_describes_exactly_the_keys_the_parser_accepts() {
        let schema = config_schema();
        let properties = schema["properties"].as_object().expect("properties");
        assert_eq!(properties.len(), 2);
        assert_eq!(schema["required"], json!(["dockerHost", "containerName"]));
        assert_eq!(properties["dockerHost"]["default"], DEFAULT_DOCKER_HOST);
        // `additionalProperties: false` has to agree with `deny_unknown_fields`,
        // or the generated form and the parser disagree about what is legal.
        assert_eq!(schema["additionalProperties"], json!(false));
    }
}
