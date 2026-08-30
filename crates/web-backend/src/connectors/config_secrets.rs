//! Storage-boundary handling for sensitive connector configuration fields.
//!
//! Connector schemas mark a top-level property with the JSON Schema extension
//! `"x-loom-sensitive": true`. The backend validates and constructs connectors
//! from plaintext, but replaces those string values with independently
//! authenticated AES-256-GCM blobs before the configuration reaches SQLite.
//! API responses omit the values and report only which keys are set.

use std::fmt;

use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::{Aes256Gcm, Key, KeyInit, Nonce};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chrono::Utc;
use rand::RngCore;
use serde_json::Value;
use sqlx::SqlitePool;

const CONFIG_ENCRYPTION_KEY_NAME: &str = "connector_config_encryption_key";
const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = 12;

/// The AES-256 key used for connector configuration encryption.
pub type ConfigEncryptionKey = Key<Aes256Gcm>;

/// A persisted key and whether this call won the first-start insert race.
pub struct LoadedConfigEncryptionKey {
    pub key: ConfigEncryptionKey,
    pub generated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigSecretError {
    MalformedBlob(String),
    AuthenticationFailed,
    InvalidUtf8,
    SensitiveValueMustBeString(String),
    ConfigurationMustBeObject,
}

impl fmt::Display for ConfigSecretError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedBlob(reason) => write!(formatter, "malformed encrypted value: {reason}"),
            Self::AuthenticationFailed => formatter
                .write_str("encrypted value failed authentication (wrong key or corrupted data)"),
            Self::InvalidUtf8 => formatter.write_str("decrypted value is not valid UTF-8"),
            Self::SensitiveValueMustBeString(key) => {
                write!(
                    formatter,
                    "sensitive configuration field `{key}` must be a string"
                )
            }
            Self::ConfigurationMustBeObject => {
                formatter.write_str("connector configuration must be a JSON object")
            }
        }
    }
}

impl std::error::Error for ConfigSecretError {}

/// Loads the independent connector-config key from `server_config`, creating it
/// on first startup with the same insert-if-absent pattern as the JWT secret.
pub async fn load_or_create_config_encryption_key(
    pool: &SqlitePool,
) -> Result<LoadedConfigEncryptionKey, Box<dyn std::error::Error>> {
    let mut candidate_bytes = [0u8; KEY_BYTES];
    rand::thread_rng().fill_bytes(&mut candidate_bytes);
    let candidate = STANDARD.encode(candidate_bytes);

    let inserted = sqlx::query(
        r#"
        INSERT INTO server_config (key, value, updated_at)
        VALUES (?, ?, ?)
        ON CONFLICT (key) DO NOTHING
        "#,
    )
    .bind(CONFIG_ENCRYPTION_KEY_NAME)
    .bind(candidate)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;

    let (encoded,): (String,) = sqlx::query_as("SELECT value FROM server_config WHERE key = ?")
        .bind(CONFIG_ENCRYPTION_KEY_NAME)
        .fetch_one(pool)
        .await?;
    let bytes = STANDARD.decode(encoded)?;
    let key = ConfigEncryptionKey::from_exact_iter(bytes)
        .ok_or_else(|| format!("persisted connector config key is not {KEY_BYTES} bytes"))?;

    Ok(LoadedConfigEncryptionKey {
        key,
        generated: inserted.rows_affected() == 1,
    })
}

/// Encrypts a UTF-8 value as `base64(nonce || ciphertext || authentication-tag)`.
///
/// The nonce is a fresh random 96-bit value. RustCrypto's returned ciphertext
/// already has the 128-bit GCM authentication tag appended, so the one encoded
/// blob contains everything decryption needs except the independently persisted
/// master key.
pub fn encrypt_value(plaintext: &str, key: &ConfigEncryptionKey) -> String {
    let cipher = Aes256Gcm::new(key);
    let mut nonce_bytes = [0u8; NONCE_BYTES];
    OsRng.fill_bytes(&mut nonce_bytes);
    let ciphertext = cipher
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_bytes())
        .expect("AES-GCM encryption with a fixed-size key and nonce cannot fail");

    let mut blob = Vec::with_capacity(NONCE_BYTES + ciphertext.len());
    blob.extend_from_slice(&nonce_bytes);
    blob.extend_from_slice(&ciphertext);
    STANDARD.encode(blob)
}

/// Decrypts the documented nonce/ciphertext/tag blob and authenticates it.
pub fn decrypt_value(blob: &str, key: &ConfigEncryptionKey) -> Result<String, ConfigSecretError> {
    let decoded = STANDARD
        .decode(blob)
        .map_err(|error| ConfigSecretError::MalformedBlob(error.to_string()))?;
    if decoded.len() <= NONCE_BYTES {
        return Err(ConfigSecretError::MalformedBlob(
            "blob is too short to contain a nonce and ciphertext".to_owned(),
        ));
    }
    let (nonce, ciphertext) = decoded.split_at(NONCE_BYTES);
    let plaintext = Aes256Gcm::new(key)
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| ConfigSecretError::AuthenticationFailed)?;
    String::from_utf8(plaintext).map_err(|_| ConfigSecretError::InvalidUtf8)
}

/// Returns top-level schema property keys marked `x-loom-sensitive: true`.
pub fn sensitive_field_keys(config_schema: &Value) -> Vec<String> {
    config_schema
        .get("properties")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
        .filter(|(_, property)| {
            property
                .get("x-loom-sensitive")
                .and_then(Value::as_bool)
                .is_some_and(|sensitive| sensitive)
        })
        .map(|(key, _)| key.clone())
        .collect()
}

/// Encrypts every present sensitive string while leaving all other JSON intact.
pub fn encrypt_sensitive_fields(
    plaintext: &Value,
    schema: &Value,
    key: &ConfigEncryptionKey,
) -> Result<Value, ConfigSecretError> {
    let mut stored = plaintext.clone();
    let fields = sensitive_field_keys(schema);
    if fields.is_empty() || stored.is_null() {
        return Ok(stored);
    }
    let object = stored
        .as_object_mut()
        .ok_or(ConfigSecretError::ConfigurationMustBeObject)?;
    for field in fields {
        let Some(value) = object.get_mut(&field) else {
            continue;
        };
        let plaintext = value
            .as_str()
            .ok_or_else(|| ConfigSecretError::SensitiveValueMustBeString(field.clone()))?;
        *value = Value::String(encrypt_value(plaintext, key));
    }
    Ok(stored)
}

/// Decrypts every present sensitive field for validation/runtime construction.
pub fn decrypt_sensitive_fields(
    stored: &Value,
    schema: &Value,
    key: &ConfigEncryptionKey,
) -> Result<Value, ConfigSecretError> {
    let mut plaintext = stored.clone();
    let fields = sensitive_field_keys(schema);
    if fields.is_empty() || plaintext.is_null() {
        return Ok(plaintext);
    }
    let object = plaintext
        .as_object_mut()
        .ok_or(ConfigSecretError::ConfigurationMustBeObject)?;
    for field in fields {
        let Some(value) = object.get_mut(&field) else {
            continue;
        };
        let blob = value
            .as_str()
            .ok_or_else(|| ConfigSecretError::SensitiveValueMustBeString(field.clone()))?;
        *value = Value::String(decrypt_value(blob, key)?);
    }
    Ok(plaintext)
}

/// Applies replace-all config semantics while preserving omitted secrets.
///
/// Non-sensitive fields come only from `incoming`. A sensitive field supplied
/// by the caller is plaintext and is freshly encrypted; an omitted one is
/// copied byte-for-byte from `stored` while its decrypted value is inserted
/// only into the temporary plaintext copy used for factory validation.
pub fn merge_sensitive_update(
    stored: &Value,
    incoming: &Value,
    schema: &Value,
    key: &ConfigEncryptionKey,
) -> Result<(Value, Value), ConfigSecretError> {
    let fields = sensitive_field_keys(schema);
    if fields.is_empty() {
        return Ok((incoming.clone(), incoming.clone()));
    }

    let stored_object = stored.as_object();
    let incoming_object = match incoming {
        Value::Null => None,
        Value::Object(object) => Some(object),
        _ => return Err(ConfigSecretError::ConfigurationMustBeObject),
    };
    let mut plaintext = incoming_object.cloned().unwrap_or_default();
    let mut encrypted = incoming_object.cloned().unwrap_or_default();

    for field in fields {
        if let Some(value) = incoming_object.and_then(|object| object.get(&field)) {
            let value = value
                .as_str()
                .ok_or_else(|| ConfigSecretError::SensitiveValueMustBeString(field.clone()))?;
            encrypted.insert(field, Value::String(encrypt_value(value, key)));
        } else if let Some(existing) = stored_object.and_then(|object| object.get(&field)) {
            let blob = existing
                .as_str()
                .ok_or_else(|| ConfigSecretError::SensitiveValueMustBeString(field.clone()))?;
            plaintext.insert(field.clone(), Value::String(decrypt_value(blob, key)?));
            encrypted.insert(field, existing.clone());
        }
    }

    Ok((Value::Object(plaintext), Value::Object(encrypted)))
}

/// Removes sensitive values from an API-facing copy and reports non-empty ones.
pub fn redact_sensitive_fields(stored: &Value, schema: &Value) -> (Value, Vec<String>) {
    let mut redacted = stored.clone();
    let Some(object) = redacted.as_object_mut() else {
        return (redacted, Vec::new());
    };
    let mut set = Vec::new();
    for field in sensitive_field_keys(schema) {
        if object
            .remove(&field)
            .and_then(|value| value.as_str().map(str::to_owned))
            .is_some_and(|value| !value.is_empty())
        {
            set.push(field);
        }
    }
    (redacted, set)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn key() -> ConfigEncryptionKey {
        ConfigEncryptionKey::clone_from_slice(&[7u8; KEY_BYTES])
    }

    #[test]
    fn encrypted_values_round_trip() {
        let encrypted = encrypt_value("fixture-secret", &key());
        assert_ne!(encrypted, "fixture-secret");
        assert_eq!(decrypt_value(&encrypted, &key()).unwrap(), "fixture-secret");
    }

    #[test]
    fn tampering_fails_authentication_without_panicking() {
        let encrypted = encrypt_value("fixture-secret", &key());
        let mut blob = STANDARD.decode(encrypted).unwrap();
        *blob.last_mut().unwrap() ^= 1;
        let tampered = STANDARD.encode(blob);
        assert_eq!(
            decrypt_value(&tampered, &key()),
            Err(ConfigSecretError::AuthenticationFailed)
        );
    }

    #[test]
    fn sensitive_keys_come_only_from_explicit_true_flags() {
        let schema = json!({
            "properties": {
                "apiToken": { "type": "string", "x-loom-sensitive": true },
                "label": { "type": "string" },
                "disabledSecret": { "type": "string", "x-loom-sensitive": false }
            }
        });
        assert_eq!(sensitive_field_keys(&schema), vec!["apiToken"]);
    }

    #[test]
    fn an_omitted_sensitive_update_keeps_the_exact_ciphertext() {
        let schema = json!({
            "properties": {
                "apiToken": { "type": "string", "x-loom-sensitive": true },
                "label": { "type": "string" }
            }
        });
        let stored = encrypt_sensitive_fields(
            &json!({ "apiToken": "original", "label": "before" }),
            &schema,
            &key(),
        )
        .unwrap();
        let original_blob = stored["apiToken"].clone();

        let (plaintext, replacement) =
            merge_sensitive_update(&stored, &json!({ "label": "after" }), &schema, &key()).unwrap();

        assert_eq!(
            plaintext,
            json!({ "apiToken": "original", "label": "after" })
        );
        assert_eq!(replacement["apiToken"], original_blob);
        assert_eq!(replacement["label"], "after");
    }
}
