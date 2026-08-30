//! Access tokens (JWT) and refresh tokens (opaque, database-backed).
//!
//! The split is the point, and it is the decision recorded in
//! `docs/adr/0008-auth-model.md`: a short-lived signed token means the common
//! case — an authenticated request — needs no database round trip, while a
//! long-lived opaque token in a table means a session can actually be revoked.
//! Either mechanism alone gives up one of those properties.

use chrono::{DateTime, Duration, Utc};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::auth::permissions::PermissionGrant;

/// How long an access token is valid.
///
/// Short, because an access token cannot be revoked — it is valid until it
/// expires, and the only bound on a stolen one is this number. Fifteen minutes
/// is the usual balance: long enough that refreshes are rare, short enough that
/// a leaked token is not a lasting foothold.
pub const ACCESS_TOKEN_LIFETIME_MINUTES: i64 = 15;

/// How long a refresh token is valid.
///
/// Long, because this is what keeps someone signed in across days. It is safe
/// to be long *because* it is revocable and rotated on every use.
pub const REFRESH_TOKEN_LIFETIME_DAYS: i64 = 7;

/// Bytes of entropy in a refresh token.
///
/// 256 bits. These are never guessed at, only stolen, so the number simply has
/// to be far beyond brute force; there is no reason to economise.
const REFRESH_TOKEN_BYTES: usize = 32;

/// Claims carried by an access token.
///
/// `permissions` is a snapshot taken when the token was issued, not a live
/// view. A grant revoked mid-token stays in the holder's claims until it
/// expires — the bound on that staleness is [`ACCESS_TOKEN_LIFETIME_MINUTES`],
/// and the refresh path recomputes from the database rather than copying old
/// claims forward.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccessClaims {
    /// Subject: the user's id.
    pub sub: String,
    /// The user's username, so a client can render it without another request.
    pub username: String,
    /// Effective grants at issuance time.
    pub permissions: Vec<PermissionGrant>,
    /// Refresh-token row backing this access token. It is an identifier, not a
    /// credential, and lets session listings mark the caller's current device
    /// without transmitting the raw refresh token on ordinary API requests.
    /// Optional so access tokens issued before session visibility was added
    /// remain valid until their normal short expiry.
    #[serde(default, rename = "sid")]
    pub session_id: Option<String>,
    /// Expiry, as a Unix timestamp.
    pub exp: i64,
    /// Issued-at, as a Unix timestamp.
    pub iat: i64,
}

/// A freshly minted access token and the moment it stops being valid.
pub struct IssuedAccessToken {
    pub token: String,
    pub expires_at: DateTime<Utc>,
}

/// Signs an access token for a user.
pub fn issue_access_token(
    secret: &str,
    user_id: &str,
    username: &str,
    permissions: Vec<PermissionGrant>,
    session_id: Option<&str>,
) -> Result<IssuedAccessToken, jsonwebtoken::errors::Error> {
    let now = Utc::now();
    let expires_at = now + Duration::minutes(ACCESS_TOKEN_LIFETIME_MINUTES);

    let claims = AccessClaims {
        sub: user_id.to_owned(),
        username: username.to_owned(),
        permissions,
        session_id: session_id.map(str::to_owned),
        exp: expires_at.timestamp(),
        iat: now.timestamp(),
    };

    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )?;

    Ok(IssuedAccessToken { token, expires_at })
}

/// Verifies an access token's signature and expiry, returning its claims.
///
/// The algorithm is pinned to HS256. Accepting whatever the token's own header
/// asks for is the classic JWT vulnerability — a token claiming `alg: none`, or
/// an RS256 verifier fed an HS256 token signed with the public key — so the
/// expected algorithm is stated here rather than read from untrusted input.
pub fn verify_access_token(
    secret: &str,
    token: &str,
) -> Result<AccessClaims, jsonwebtoken::errors::Error> {
    let mut validation = Validation::new(Algorithm::HS256);
    validation.validate_exp = true;

    decode::<AccessClaims>(
        token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
}

/// Hashes a refresh token for storage.
///
/// SHA-256, not argon2, and deliberately: a refresh token is 256 bits from a
/// CSPRNG, so there is no dictionary to attack and nothing for a slow hash to
/// buy. What matters is that the stored form is not usable as a credential,
/// which a fast hash achieves just as well while keeping every refresh cheap.
fn hash_refresh_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    // Lowercase hex, so the column is comparable with a plain `=` and readable
    // in a database client.
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Context captured when a refresh token is issued.
#[derive(Debug, Clone, Default)]
pub struct RefreshTokenContext {
    pub user_agent: Option<String>,
    pub ip_address: Option<String>,
}

/// A refresh token and the non-secret row id paired into the access token.
pub struct IssuedRefreshToken {
    pub id: String,
    pub token: String,
}

/// Generates a refresh token, stores its hash and recognition context, and
/// returns the raw value exactly once.
///
/// The raw token is returned exactly once and never persisted. Losing it means
/// issuing a new one, which is the intended failure mode.
pub async fn issue_refresh_token(
    pool: &SqlitePool,
    user_id: &str,
    context: &RefreshTokenContext,
) -> Result<IssuedRefreshToken, sqlx::Error> {
    let mut bytes = [0u8; REFRESH_TOKEN_BYTES];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();

    let now = Utc::now();
    let expires_at = now + Duration::days(REFRESH_TOKEN_LIFETIME_DAYS);

    let id = Uuid::new_v4().to_string();
    sqlx::query(
        r#"
        INSERT INTO refresh_tokens (
            id, user_id, token_hash, created_at, expires_at, revoked_at,
            user_agent, ip_address
        )
        VALUES (?, ?, ?, ?, ?, NULL, ?, ?)
        "#,
    )
    .bind(&id)
    .bind(user_id)
    .bind(hash_refresh_token(&token))
    .bind(now.to_rfc3339())
    .bind(expires_at.to_rfc3339())
    .bind(&context.user_agent)
    .bind(&context.ip_address)
    .execute(pool)
    .await?;

    Ok(IssuedRefreshToken { id, token })
}

/// Why a refresh token was not accepted.
///
/// Collapsed into one response by the handler — a caller learns only that the
/// token is no good, never which of these it was, since the difference is
/// useful to an attacker probing tokens and useless to a legitimate client,
/// whose reaction is the same in every case: sign in again.
#[derive(Debug, PartialEq, Eq)]
pub enum RefreshTokenError {
    /// No row matched the presented token.
    NotFound,
    /// Explicitly revoked, by logout or by rotation.
    Revoked,
    /// Past its expiry.
    Expired,
}

/// The user a valid refresh token belongs to.
pub struct ValidRefreshToken {
    /// Row id, so the caller can revoke exactly this token when rotating.
    pub id: String,
    pub user_id: String,
}

#[derive(sqlx::FromRow)]
struct RefreshTokenRow {
    id: String,
    user_id: String,
    expires_at: String,
    revoked_at: Option<String>,
}

/// Looks up a refresh token and checks it is live.
pub async fn validate_refresh_token(
    pool: &SqlitePool,
    token: &str,
) -> Result<Result<ValidRefreshToken, RefreshTokenError>, sqlx::Error> {
    let row = sqlx::query_as::<_, RefreshTokenRow>(
        "SELECT id, user_id, expires_at, revoked_at FROM refresh_tokens WHERE token_hash = ?",
    )
    .bind(hash_refresh_token(token))
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(Err(RefreshTokenError::NotFound));
    };

    if row.revoked_at.is_some() {
        return Ok(Err(RefreshTokenError::Revoked));
    }

    // An unparseable expiry means a corrupted row. Treating it as expired fails
    // closed, which is the only safe direction for a credential check.
    let expires_at = DateTime::parse_from_rfc3339(&row.expires_at)
        .map(|value| value.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now() - Duration::seconds(1));

    if expires_at <= Utc::now() {
        return Ok(Err(RefreshTokenError::Expired));
    }

    Ok(Ok(ValidRefreshToken {
        id: row.id,
        user_id: row.user_id,
    }))
}

/// Marks one refresh token revoked by row id.
///
/// Idempotent: revoking an already-revoked token leaves the original timestamp
/// alone, so the record keeps saying when the session actually ended.
pub async fn revoke_refresh_token_by_id(pool: &SqlitePool, id: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE refresh_tokens SET revoked_at = ? WHERE id = ? AND revoked_at IS NULL")
        .bind(Utc::now().to_rfc3339())
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// Marks one refresh token revoked by its raw value.
///
/// Returns whether a live token was actually revoked. Logout does not surface
/// that difference: reporting "no such token" would let an unauthenticated
/// caller test token validity, and the caller's session is over either way.
pub async fn revoke_refresh_token(pool: &SqlitePool, token: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = ? WHERE token_hash = ? AND revoked_at IS NULL",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(hash_refresh_token(token))
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant(key: &str) -> PermissionGrant {
        PermissionGrant {
            key: key.to_owned(),
            resource_type: None,
            resource_id: None,
        }
    }

    #[test]
    fn an_issued_access_token_verifies_and_round_trips_its_claims() {
        let issued = issue_access_token(
            "test-secret",
            "user-id",
            "someone",
            vec![grant("connectors.view")],
            Some("session-id"),
        )
        .expect("issuing must succeed");

        let claims = verify_access_token("test-secret", &issued.token).expect("must verify");

        assert_eq!(claims.sub, "user-id");
        assert_eq!(claims.username, "someone");
        assert_eq!(claims.permissions, vec![grant("connectors.view")]);
        assert_eq!(claims.session_id.as_deref(), Some("session-id"));
        assert_eq!(claims.exp, issued.expires_at.timestamp());
    }

    #[test]
    fn a_token_signed_with_another_secret_is_rejected() {
        let issued = issue_access_token("the-real-secret", "user-id", "someone", vec![], None)
            .expect("issued");

        assert!(verify_access_token("a-different-secret", &issued.token).is_err());
    }

    #[test]
    fn a_tampered_token_is_rejected() {
        let issued =
            issue_access_token("secret", "user-id", "someone", vec![], None).expect("issued");

        // Flip the last character of the signature.
        let mut tampered = issued.token.clone();
        let last = tampered.pop().expect("token is not empty");
        tampered.push(if last == 'A' { 'B' } else { 'A' });

        assert!(verify_access_token("secret", &tampered).is_err());
    }

    #[test]
    fn an_alg_none_token_is_rejected() {
        // The signature-stripping attack: a token asking to be verified with no
        // algorithm at all. Pinning HS256 at the verifier is what stops it.
        let unsigned = "eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.\
                        eyJzdWIiOiJhdHRhY2tlciIsInVzZXJuYW1lIjoicm9vdCIsInBlcm1pc3Npb25zIjpbXSwiZXhwIjo5OTk5OTk5OTk5LCJpYXQiOjB9.";

        assert!(verify_access_token("secret", unsigned).is_err());
    }

    #[test]
    fn refresh_token_hashing_is_stable_and_not_the_raw_value() {
        let hashed = hash_refresh_token("a-token");

        assert_eq!(hashed, hash_refresh_token("a-token"));
        assert_ne!(hashed, "a-token");
        assert_ne!(hashed, hash_refresh_token("a-token-2"));
        // Hex-encoded SHA-256.
        assert_eq!(hashed.len(), 64);
        assert!(hashed.chars().all(|c| c.is_ascii_hexdigit()));
    }
}
