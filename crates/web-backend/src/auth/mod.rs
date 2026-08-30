//! Authentication: passwords, tokens, and effective permissions.
//!
//! This is the storage and credential half of the auth system described in
//! `docs/adr/0008-auth-model.md`. It answers "who is this" and "what were they
//! granted"; it does not answer "may they do this particular thing", which is
//! the authorization middleware's job and lands separately.
//!
//! Nothing in here makes a trust decision on behalf of `loom-core` — Core has
//! no idea any of this exists, which is the boundary `docs/ARCHITECTURE.md`
//! requires.

pub mod extract;
pub mod password;
pub mod permissions;
pub mod rate_limit;
pub mod secret;
pub mod tokens;
