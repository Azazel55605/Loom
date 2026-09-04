use loom_core::connector::{ConnectorError, NetworkTarget};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{json, Value};

/// Persisted configuration for one local UniFi Network console and site.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UniFiNetworkConfig {
    pub host: String,
    pub api_key: String,
    #[serde(default = "default_site")]
    pub site: String,
    #[serde(default)]
    pub allow_insecure_cert: bool,
}

impl UniFiNetworkConfig {
    pub(crate) fn from_value(value: Value) -> Result<Self, ConnectorError> {
        let mut config: Self = serde_json::from_value(value).map_err(|error| {
            ConnectorError::invalid_config(format!(
                "configuration does not match the schema: {error}"
            ))
        })?;
        config.host = config.host.trim().trim_end_matches('/').to_owned();
        config.site = config.site.trim().to_owned();

        if config.host.is_empty() {
            return Err(ConnectorError::invalid_config("host must not be empty"));
        }
        if config.api_key.trim().is_empty() {
            return Err(ConnectorError::invalid_config("apiKey must not be empty"));
        }
        if config.site.is_empty() {
            return Err(ConnectorError::invalid_config("site must not be empty"));
        }

        let parsed = Url::parse(&config.host).map_err(|error| {
            ConnectorError::invalid_config(format!("host must be a complete HTTPS URL: {error}"))
        })?;
        if parsed.scheme() != "https" {
            return Err(ConnectorError::invalid_config("host must use https://"));
        }
        if parsed.host_str().is_none() {
            return Err(ConnectorError::invalid_config(
                "host must include a hostname or IP address",
            ));
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(ConnectorError::invalid_config(
                "host must not contain credentials",
            ));
        }
        if parsed.path() != "/" || parsed.query().is_some() || parsed.fragment().is_some() {
            return Err(ConnectorError::invalid_config(
                "host must not contain a path, query, or fragment",
            ));
        }

        Ok(config)
    }

    pub(crate) fn network_target(&self) -> Option<NetworkTarget> {
        let url = Url::parse(&self.host).ok()?;
        Some(NetworkTarget::new(
            url.host_str()?.trim_matches(['[', ']']),
            url.port_or_known_default()?,
        ))
    }
}

fn default_site() -> String {
    "default".to_owned()
}

/// JSON Schema consumed by the shared add/edit connector dialog.
pub fn config_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "UniFi Network connection",
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "minLength": 1,
                "description": "Your local UniFi console origin, including https:// but no API path, for example https://192.168.1.1."
            },
            "apiKey": {
                "type": "string",
                "minLength": 1,
                "x-loom-sensitive": true,
                "description": "API key generated in UniFi Network under Settings > Control Plane > Integrations. The official Integration API requires UniFi Network 9.1.105 or newer."
            },
            "site": {
                "type": "string",
                "minLength": 1,
                "default": "default",
                "description": "Site UUID, internal reference, or display name. Most homelab consoles use the default site."
            },
            "allowInsecureCert": {
                "type": "boolean",
                "title": "Accept untrusted certificate",
                "default": false,
                "description": "Accept a self-signed, expired, hostname-mismatched, or otherwise untrusted HTTPS certificate. HTTPS encryption remains enabled. Only enable this after verifying the console."
            }
        },
        "required": ["host", "apiKey"],
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
        assert_eq!(schema["properties"]["site"]["default"], "default");
        assert_eq!(schema["properties"]["allowInsecureCert"]["default"], false);
    }

    #[test]
    fn config_normalizes_the_origin_and_defaults_the_site() {
        let config = UniFiNetworkConfig::from_value(json!({
            "host": " https://console.example.com:8443/ ",
            "apiKey": "not-a-real-key"
        }))
        .expect("valid config");

        assert_eq!(config.host, "https://console.example.com:8443");
        assert_eq!(config.site, "default");
        assert!(!config.allow_insecure_cert);
        assert_eq!(
            config.network_target(),
            Some(NetworkTarget::new("console.example.com", 8443))
        );
    }

    #[test]
    fn config_requires_a_bare_https_origin() {
        for host in [
            "http://console.example.com",
            "console.example.com",
            "https://admin:secret@console.example.com",
            "https://console.example.com/proxy/network",
        ] {
            let error = UniFiNetworkConfig::from_value(json!({
                "host": host,
                "apiKey": "not-a-real-key"
            }))
            .expect_err("unsafe or ambiguous origin must be rejected");
            assert!(matches!(error, ConnectorError::InvalidConfig { .. }));
        }
    }
}
