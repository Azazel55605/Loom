use loom_core::connector::{ConnectorError, NetworkTarget};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{json, Value};

/// Persisted configuration for one Pi-hole v6 API connection.
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PiHoleConnectorConfig {
    pub base_url: String,
    pub password: String,
    #[serde(default)]
    pub allow_insecure_cert: bool,
}

impl PiHoleConnectorConfig {
    pub(crate) fn from_value(value: Value) -> Result<Self, ConnectorError> {
        let mut config: Self = serde_json::from_value(value).map_err(|error| {
            ConnectorError::invalid_config(format!(
                "configuration does not match the schema: {error}"
            ))
        })?;
        config.base_url = config.base_url.trim().trim_end_matches('/').to_owned();

        if config.base_url.is_empty() {
            return Err(ConnectorError::invalid_config("baseUrl must not be empty"));
        }
        if config.password.is_empty() {
            return Err(ConnectorError::invalid_config("password must not be empty"));
        }

        let parsed = Url::parse(&config.base_url).map_err(|error| {
            ConnectorError::invalid_config(format!(
                "baseUrl must be a complete HTTP or HTTPS URL: {error}"
            ))
        })?;
        if !matches!(parsed.scheme(), "http" | "https") {
            return Err(ConnectorError::invalid_config(
                "baseUrl must use http:// or https://",
            ));
        }
        if parsed.host_str().is_none() {
            return Err(ConnectorError::invalid_config(
                "baseUrl must include a hostname or IP address",
            ));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(ConnectorError::invalid_config(
                "baseUrl must not contain credentials",
            ));
        }
        if parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(ConnectorError::invalid_config(
                "baseUrl must not contain a query or fragment",
            ));
        }

        Ok(config)
    }

    pub(crate) fn network_target(&self) -> Option<NetworkTarget> {
        let url = Url::parse(&self.base_url).ok()?;
        Some(NetworkTarget::new(
            url.host_str()?.trim_matches(['[', ']']),
            url.port_or_known_default()?,
        ))
    }
}

/// JSON Schema consumed by the shared add/edit connector dialog.
pub fn config_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Pi-hole connection",
        "type": "object",
        "properties": {
            "baseUrl": {
                "type": "string",
                "minLength": 1,
                "description": "Your Pi-hole's address, e.g. http://pi.hole or http://192.168.1.x — include the scheme."
            },
            "password": {
                "type": "string",
                "minLength": 1,
                "x-loom-sensitive": true,
                "description": "Your Pi-hole password, or — recommended — an application password generated under Settings in the Pi-hole web interface, so your real admin password never leaves Pi-hole itself."
            },
            "allowInsecureCert": {
                "type": "boolean",
                "title": "Accept untrusted certificate",
                "default": false,
                "description": "Accept a self-signed, expired, hostname-mismatched, or otherwise untrusted HTTPS certificate. HTTPS encryption remains enabled. Only enable this after verifying the Pi-hole endpoint."
            }
        },
        "required": ["baseUrl", "password"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_marks_only_the_password_sensitive() {
        let schema = config_schema();
        assert_eq!(schema["properties"]["password"]["x-loom-sensitive"], true);
        assert!(schema["properties"]["baseUrl"]["x-loom-sensitive"].is_null());
        assert_eq!(schema["properties"]["allowInsecureCert"]["default"], false);
        assert_eq!(schema["required"], json!(["baseUrl", "password"]));
    }

    #[test]
    fn config_normalizes_a_trailing_slash_and_derives_the_network_target() {
        let config = PiHoleConnectorConfig::from_value(json!({
            "baseUrl": " https://pi.hole:8443/ ",
            "password": "not-a-real-password"
        }))
        .expect("valid config");

        assert_eq!(config.base_url, "https://pi.hole:8443");
        assert!(!config.allow_insecure_cert);
        assert_eq!(
            config.network_target(),
            Some(NetworkTarget::new("pi.hole", 8443))
        );
    }

    #[test]
    fn config_rejects_non_http_urls_and_embedded_credentials() {
        for base_url in ["ftp://pi.hole", "https://admin:secret@pi.hole", "pi.hole"] {
            let error = PiHoleConnectorConfig::from_value(json!({
                "baseUrl": base_url,
                "password": "not-a-real-password"
            }))
            .err()
            .expect("unsafe or incomplete URL must be rejected");
            assert!(matches!(error, ConnectorError::InvalidConfig { .. }));
        }
    }

    #[test]
    fn config_accepts_an_explicit_untrusted_certificate_policy() {
        let config = PiHoleConnectorConfig::from_value(json!({
            "baseUrl": "https://pi.hole",
            "password": "not-a-real-password",
            "allowInsecureCert": true
        }))
        .expect("valid config");

        assert!(config.allow_insecure_cert);
    }
}
