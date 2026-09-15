use loom_core::connector::ConnectorError;
use serde::Deserialize;
use serde_json::{json, Value};

/// Configuration for one Govee cloud account.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GoveeConnectorConfig {
    pub api_key: String,
}

impl GoveeConnectorConfig {
    pub fn from_value(value: Value) -> Result<Self, ConnectorError> {
        let mut config: Self =
            serde_json::from_value(value).map_err(|error| ConnectorError::InvalidConfig {
                reason: error.to_string(),
            })?;
        config.api_key = config.api_key.trim().to_owned();
        if config.api_key.is_empty() {
            return Err(ConnectorError::InvalidConfig {
                reason: "`apiKey` must not be empty".to_owned(),
            });
        }
        Ok(config)
    }
}

/// JSON Schema consumed by Loom's generic connector form.
pub fn config_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "apiKey": {
                "type": "string",
                "title": "API key",
                "description": "API key created in the Govee Home app under Settings > Apply for API Key.",
                "minLength": 1,
                "x-loom-sensitive": true
            }
        },
        "required": ["apiKey"],
        "additionalProperties": false
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_key_is_required_and_sensitive() {
        assert!(GoveeConnectorConfig::from_value(json!({ "apiKey": "  " })).is_err());
        let schema = config_schema();
        assert_eq!(schema["properties"]["apiKey"]["x-loom-sensitive"], true);
        assert_eq!(schema["required"], json!(["apiKey"]));
    }
}
