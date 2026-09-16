use loom_core::connector::ConnectorError;
use serde::Deserialize;
use serde_json::{json, Value};

use crate::DEFAULT_PORT;

/// Configuration for one Music Assistant server.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MusicAssistantConnectorConfig {
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub token: Option<String>,
}

impl MusicAssistantConnectorConfig {
    pub fn from_value(value: Value) -> Result<Self, ConnectorError> {
        let mut config: Self =
            serde_json::from_value(value).map_err(|error| ConnectorError::InvalidConfig {
                reason: error.to_string(),
            })?;
        config.host = config.host.trim().trim_end_matches('/').to_owned();
        config.token = config
            .token
            .take()
            .map(|token| token.trim().to_owned())
            .filter(|token| !token.is_empty());
        if config.host.is_empty() {
            return Err(ConnectorError::InvalidConfig {
                reason: "`host` must not be empty".to_owned(),
            });
        }
        Ok(config)
    }
}

const fn default_port() -> u16 {
    DEFAULT_PORT
}

/// JSON Schema consumed by Loom's generic connector form.
pub fn config_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "title": "Host",
                "description": "Music Assistant hostname or IP address. Prefix with https:// when connecting through a TLS reverse proxy.",
                "minLength": 1
            },
            "port": {
                "type": "integer",
                "title": "Port",
                "description": "Music Assistant web server port.",
                "minimum": 1,
                "maximum": 65535,
                "default": DEFAULT_PORT
            },
            "token": {
                "type": "string",
                "title": "Access token",
                "description": "Long-lived Music Assistant access token. Required by current Music Assistant servers; blank is supported only for legacy pre-schema-28 servers.",
                "x-loom-sensitive": true
            }
        },
        "required": ["host"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_marks_token_sensitive_and_defaults_the_port() {
        let schema = config_schema();
        assert_eq!(schema["properties"]["token"]["x-loom-sensitive"], true);
        assert_eq!(schema["properties"]["port"]["default"], DEFAULT_PORT);
        let config = MusicAssistantConnectorConfig::from_value(json!({
            "host": " music.example.com/ ",
            "token": " token "
        }))
        .unwrap();
        assert_eq!(config.host, "music.example.com");
        assert_eq!(config.port, DEFAULT_PORT);
        assert_eq!(config.token.as_deref(), Some("token"));
    }
}
