//! Per-peer login failure limiting.
//!
//! The key is the direct peer IP, deliberately not a username. A username key
//! lets an attacker lock out a known account simply by submitting bad
//! passwords for it. An IP key raises the cost of guessing without turning the
//! limiter into an account-lockout endpoint. The trade-off is explicit in ADR
//! 0030: a distributed attacker can spread guesses across addresses, and a
//! reverse proxy appears as one peer until trusted-proxy handling exists.

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

/// Failed attempts allowed in one rolling window before later requests are
/// rejected. The tenth failure is still answered as invalid credentials; the
/// next attempt is rate-limited until the oldest failure ages out.
pub const LOGIN_FAILURE_LIMIT: usize = 10;

/// Length of the rolling failure window.
pub const LOGIN_FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);

#[derive(Clone, Default)]
pub struct LoginRateLimiter {
    failures: Arc<Mutex<HashMap<IpAddr, VecDeque<Instant>>>>,
}

impl LoginRateLimiter {
    /// How long this peer must wait, or `None` when it may attempt a login.
    pub async fn retry_after(&self, peer: IpAddr) -> Option<Duration> {
        let now = Instant::now();
        let mut failures = self.failures.lock().await;
        prune_all(&mut failures, now);
        let attempts = failures.get(&peer)?;
        if attempts.len() < LOGIN_FAILURE_LIMIT {
            return None;
        }
        attempts
            .front()
            .map(|oldest| oldest.saturating_duration_since(now - LOGIN_FAILURE_WINDOW))
    }

    /// Records one generic credential failure for this peer.
    pub async fn record_failure(&self, peer: IpAddr) {
        let now = Instant::now();
        let mut failures = self.failures.lock().await;
        prune_all(&mut failures, now);
        failures.entry(peer).or_default().push_back(now);
    }

    /// A successful login resets the peer's history immediately.
    pub async fn clear(&self, peer: IpAddr) {
        self.failures.lock().await.remove(&peer);
    }
}

fn prune_all(failures: &mut HashMap<IpAddr, VecDeque<Instant>>, now: Instant) {
    failures.retain(|_, attempts| {
        while attempts
            .front()
            .is_some_and(|attempt| now.duration_since(*attempt) >= LOGIN_FAILURE_WINDOW)
        {
            attempts.pop_front();
        }
        !attempts.is_empty()
    });
}
