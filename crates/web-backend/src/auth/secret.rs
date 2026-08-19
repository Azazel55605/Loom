//! The JWT signing secret: generated once, then persisted.
//!
//! ADR 0004 forbids a required environment variable, so this cannot be
//! configuration. It also cannot be ephemeral: a secret regenerated on each
//! start would invalidate every outstanding access token on every restart and
//! deploy, signing everyone out for no reason. So it is generated from the OS
//! CSPRNG on first boot and stored in `server_config`, living with the database
//! it protects.

use chrono::Utc;
use rand::RngCore;
use sqlx::SqlitePool;

/// `server_config` key holding the signing secret.
const JWT_SECRET_KEY: &str = "jwt_signing_secret";

/// Bytes of entropy in the signing secret. 256 bits, matching HS256's output.
const SECRET_BYTES: usize = 32;

/// Reads the signing secret, generating and storing one on first call.
///
/// The insert is `ON CONFLICT DO NOTHING` followed by a read, rather than
/// check-then-insert: two concurrent callers must end up with the *same*
/// secret, and the loser of the race has to see the winner's value rather than
/// overwrite it. Overwriting would invalidate tokens the winner had already
/// signed.
pub async fn load_or_create_jwt_secret(pool: &SqlitePool) -> Result<String, sqlx::Error> {
    let mut bytes = [0u8; SECRET_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let candidate: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();

    sqlx::query(
        r#"
        INSERT INTO server_config (key, value, updated_at)
        VALUES (?, ?, ?)
        ON CONFLICT (key) DO NOTHING
        "#,
    )
    .bind(JWT_SECRET_KEY)
    .bind(&candidate)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;

    let (secret,): (String,) = sqlx::query_as("SELECT value FROM server_config WHERE key = ?")
        .bind(JWT_SECRET_KEY)
        .fetch_one(pool)
        .await?;

    Ok(secret)
}
