use loom_core::connector::{ConnectorError, NetworkTarget};
use reqwest::Url;
use serde::Deserialize;
use serde_json::{json, Value};

/// Persisted configuration for one Tasmota device.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TasmotaConnectorConfig {
    pub host: String,
    #[serde(default)]
    pub password: Option<String>,
}

impl TasmotaConnectorConfig {
    pub(crate) fn from_value(value: Value) -> Result<Self, ConnectorError> {
        let mut config: Self = serde_json::from_value(value).map_err(|error| {
            ConnectorError::invalid_config(format!(
                "configuration does not match the schema: {error}"
            ))
        })?;
        config.host = normalize_host(&config.host)?;
        config.password = config.password.filter(|password| !password.is_empty());
        Ok(config)
    }

    pub(crate) fn base_url(&self) -> String {
        format!("http://{}", self.host)
    }

    pub(crate) fn network_target(&self) -> Option<NetworkTarget> {
        let url = Url::parse(&self.base_url()).ok()?;
        Some(NetworkTarget::new(
            url.host_str()?.trim_matches(['[', ']']),
            url.port_or_known_default()?,
        ))
    }
}

fn normalize_host(value: &str) -> Result<String, ConnectorError> {
    let value = value.trim().trim_end_matches('/');
    if value.is_empty() {
        return Err(ConnectorError::invalid_config("host must not be empty"));
    }
    let candidate = if value.starts_with("http://") {
        value.to_owned()
    } else if value.contains("://") {
        return Err(ConnectorError::invalid_config(
            "host must be a hostname or IP address; Tasmota's command API uses HTTP",
        ));
    } else {
        format!("http://{value}")
    };
    let url = Url::parse(&candidate).map_err(|error| {
        ConnectorError::invalid_config(format!(
            "host is not a valid hostname or IP address: {error}"
        ))
    })?;
    if url.scheme() != "http" || url.host_str().is_none() {
        return Err(ConnectorError::invalid_config(
            "host must identify an HTTP hostname or IP address",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ConnectorError::invalid_config(
            "host must not contain credentials",
        ));
    }
    if !matches!(url.path(), "" | "/") || url.query().is_some() || url.fragment().is_some() {
        return Err(ConnectorError::invalid_config(
            "host must not contain a path, query, or fragment",
        ));
    }

    let host = url
        .host_str()
        .expect("the host presence was checked above")
        .trim_matches(['[', ']']);
    let authority_host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host.to_owned()
    };
    Ok(match url.port() {
        Some(port) => format!("{authority_host}:{port}"),
        None => authority_host,
    })
}

/// JSON Schema consumed by the shared add/edit connector dialog.
pub fn config_schema() -> Value {
    json!({
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "title": "Tasmota connection",
        "type": "object",
        "properties": {
            "host": {
                "type": "string",
                "minLength": 1,
                "description": "The Tasmota device's hostname or IP address, optionally with its HTTP port. Do not include a path."
            },
            "password": {
                "type": "string",
                "x-loom-sensitive": true,
                "description": "Only needed when the Tasmota web admin password is set. Tasmota sends web credentials over its plaintext HTTP command API, so use this only on a trusted network."
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
    fn schema_marks_the_optional_password_sensitive() {
        let schema = config_schema();
        assert_eq!(schema["properties"]["password"]["x-loom-sensitive"], true);
        assert_eq!(schema["required"], json!(["host"]));
    }

    #[test]
    fn config_accepts_bare_hosts_and_normalizes_an_http_origin() {
        let bare = TasmotaConnectorConfig::from_value(json!({ "host": " plug.example:8080/ " }))
            .expect("bare host");
        assert_eq!(bare.host, "plug.example:8080");
        assert_eq!(bare.base_url(), "http://plug.example:8080");
        assert_eq!(
            bare.network_target(),
            Some(NetworkTarget::new("plug.example", 8080))
        );

        let origin = TasmotaConnectorConfig::from_value(json!({
            "host": "http://plug.example",
            "password": ""
        }))
        .expect("HTTP origin");
        assert_eq!(origin.host, "plug.example");
        assert_eq!(origin.password, None);
    }

    #[test]
    fn config_rejects_non_http_schemes_paths_and_embedded_credentials() {
        for host in [
            "https://plug.example",
            "http://plug.example/cm",
            "http://admin:secret@plug.example",
        ] {
            let error = TasmotaConnectorConfig::from_value(json!({ "host": host }))
                .expect_err("unsafe or ambiguous host must be rejected");
            assert!(matches!(error, ConnectorError::InvalidConfig { .. }));
        }
    }

    #[test]
    fn config_preserves_ipv6_authority_syntax() {
        let config = TasmotaConnectorConfig::from_value(json!({ "host": "[2001:db8::42]" }))
            .expect("IPv6 host");
        assert_eq!(config.host, "[2001:db8::42]");
        assert_eq!(config.base_url(), "http://[2001:db8::42]");
        assert_eq!(
            config.network_target(),
            Some(NetworkTarget::new("2001:db8::42", 80))
        );
    }
}
