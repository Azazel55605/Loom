use loom_core::connector::ConnectorError;
use serde::Deserialize;
use serde_json::{json, Value};

/// Persisted configuration for one TrueNAS host connection.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrueNasConnectorConfig {
    /// Hostname or literal IP only. The transport owns the mandatory scheme
    /// and JSON-RPC path so stored configuration cannot downgrade either.
    pub host: String,
    /// TrueNAS account that owns the API key. Current TrueNAS authentication
    /// requires it alongside the key for `auth.login_ex`.
    #[serde(default)]
    pub username: Option<String>,
    /// TrueNAS API key. The backend encrypts this field at rest from the schema
    /// annotation below before it ever reaches persistence.
    pub api_key: String,
    /// Explicit opt-in for a self-signed or otherwise untrusted certificate.
    #[serde(default)]
    pub allow_insecure_cert: bool,
}

impl TrueNasConnectorConfig {
    pub(crate) fn from_value(value: Value) -> Result<Self, ConnectorError> {
        let supplied_username = value.get("username").is_some();
        let mut config: Self = serde_json::from_value(value).map_err(|error| {
            ConnectorError::invalid_config(format!(
                "configuration does not match the schema: {error}"
            ))
        })?;
        config.host = config.host.trim().to_owned();
        config.username = config
            .username
            .map(|username| username.trim().to_owned())
            .filter(|username| !username.is_empty());

        if config.host.is_empty() {
            return Err(ConnectorError::invalid_config("host must not be empty"));
        }
        if config.host.contains("://")
            || config.host.contains('/')
            || config.host.contains('?')
            || config.host.contains('#')
        {
            return Err(ConnectorError::invalid_config(
                "host must be a hostname or IP address without a scheme, path, query, or fragment",
            ));
        }
        if config.api_key.trim().is_empty() {
            return Err(ConnectorError::invalid_config("apiKey must not be empty"));
        }
        if supplied_username && config.username.is_none() {
            return Err(ConnectorError::invalid_config("username must not be empty"));
        }

        Ok(config)
    }
}

/// JSON Schema consumed by all three clients' shared `SchemaForm`.
pub fn config_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "TrueNAS connection",
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "minLength": 1,
                "description": "TrueNAS hostname or IP address, without a scheme. Loom always connects with encrypted wss:// transport."
            },
            "username": {
                "type": "string",
                "minLength": 1,
                "description": "TrueNAS username that owns the API key. This is required by the current auth.login_ex API-key authentication flow."
            },
            "apiKey": {
                "type": "string",
                "minLength": 1,
                "x-loom-sensitive": true,
                "description": "API key generated from the TrueNAS top-right account/settings menu > My API Keys screen."
            },
            "allowInsecureCert": {
                "type": "boolean",
                "default": false,
                "description": "Accept a self-signed or otherwise untrusted certificate. TLS encryption remains mandatory; this never enables an unencrypted connection."
            }
        },
        "required": ["host", "username", "apiKey"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_marks_only_the_api_key_sensitive() {
        let schema = config_schema();
        assert_eq!(schema["properties"]["apiKey"]["x-loom-sensitive"], true);
        assert!(schema["properties"]["host"]["x-loom-sensitive"].is_null());
        assert!(schema["properties"]["username"]["x-loom-sensitive"].is_null());
        assert_eq!(schema["required"], json!(["host", "username", "apiKey"]));
    }

    #[test]
    fn config_rejects_a_scheme_before_the_transport_is_built() {
        let error = TrueNasConnectorConfig::from_value(json!({
            "host": "https://nas.example.com",
            "username": "api-user",
            "apiKey": "not-a-real-key"
        }))
        .expect_err("the schema promises a bare host");
        assert!(error.to_string().contains("without a scheme"));
    }

    #[test]
    fn an_old_configuration_without_username_remains_parseable_for_legacy_auth() {
        let config = TrueNasConnectorConfig::from_value(json!({
            "host": "nas.example.com",
            "apiKey": "not-a-real-key"
        }))
        .expect("stored configurations can predate the username field");

        assert_eq!(config.username, None);
    }

    #[test]
    fn a_supplied_username_must_not_be_blank() {
        let error = TrueNasConnectorConfig::from_value(json!({
            "host": "nas.example.com",
            "username": "  ",
            "apiKey": "not-a-real-key"
        }))
        .expect_err("new configuration cannot silently select legacy auth");

        assert!(error.to_string().contains("username must not be empty"));
    }
}
