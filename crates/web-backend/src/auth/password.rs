//! Password hashing.
//!
//! Argon2id with the `argon2` crate's default parameters, which track the
//! OWASP-recommended settings rather than being a number picked here. The
//! resulting PHC string carries its own parameters, so raising the cost later
//! does not invalidate stored hashes — they keep verifying under whatever they
//! were created with, and can be upgraded on next successful login.

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, SaltString};
use argon2::{Argon2, PasswordVerifier};

/// Minimum password length accepted at setup.
///
/// A length floor, deliberately without composition rules: length is what
/// actually costs an attacker, while character-class rules mostly produce
/// predictable substitutions. This is a starting point, not a finished policy —
/// checking against a breached-password list would do more than raising this
/// number, and is the obvious next step.
pub const MIN_PASSWORD_LENGTH: usize = 8;

/// Hashes a password into a PHC-format argon2id string.
///
/// The salt is drawn per call from the OS CSPRNG, so two identical passwords do
/// not produce identical hashes.
pub fn hash_password(password: &str) -> Result<String, argon2::password_hash::Error> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)?
        .to_string())
}

/// Checks a password against a stored PHC hash.
///
/// Returns `false` for a wrong password *and* for a malformed stored hash. A
/// hash that cannot be parsed is not something a login attempt should surface
/// as an error to the caller — there is no credential that would satisfy it, so
/// the honest answer to "are these credentials valid" is no.
pub fn verify_password(password: &str, phc_hash: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(phc_hash) else {
        return false;
    };

    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hashed_password_verifies() {
        let hash = hash_password("correct horse battery staple").expect("hashing must succeed");
        assert!(verify_password("correct horse battery staple", &hash));
    }

    #[test]
    fn a_wrong_password_does_not_verify() {
        let hash = hash_password("correct horse battery staple").expect("hashing must succeed");
        assert!(!verify_password("Correct horse battery staple", &hash));
        assert!(!verify_password("", &hash));
    }

    #[test]
    fn the_same_password_hashes_differently_each_time() {
        let first = hash_password("same input").expect("hashing must succeed");
        let second = hash_password("same input").expect("hashing must succeed");

        // Per-hash salts: identical passwords must not produce identical
        // stored values, or a leaked database reveals which accounts share one.
        assert_ne!(first, second);
        assert!(verify_password("same input", &first));
        assert!(verify_password("same input", &second));
    }

    #[test]
    fn a_malformed_hash_fails_closed() {
        assert!(!verify_password("anything", "not-a-phc-string"));
        assert!(!verify_password("anything", ""));
    }

    #[test]
    fn hashes_are_argon2id() {
        let hash = hash_password("whatever").expect("hashing must succeed");
        assert!(hash.starts_with("$argon2id$"), "got {hash}");
    }
}
