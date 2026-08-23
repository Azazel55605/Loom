//! Why a connector that is Down might be Down, established from outside it.
//!
//! When a connector stops answering, the connector itself can only report that
//! it stopped answering. That is true and it is rarely the useful sentence. The
//! useful sentence distinguishes three situations that look identical from
//! inside Loom and need three different fixes:
//!
//! 1. the name does not resolve — a DNS entry, a typo, a search domain;
//! 2. the name resolves but nothing accepts a connection there — the host is
//!    off, or a firewall is in the way;
//! 3. the host accepts a connection and the service still is not answering —
//!    the service crashed or is misconfigured, and the network is innocent.
//!
//! # Why TCP connect and not ICMP ping
//!
//! Ping is the reflex and it is the wrong tool twice over.
//!
//! **It is not portable.** An ICMP echo needs a raw socket, which needs
//! `CAP_NET_RAW` or root. Loom runs as an unprivileged user in a container by
//! design, so a diagnostic built on ping would be one that silently reports
//! "unreachable" for every host on most real deployments — the worst possible
//! failure mode for a feature whose entire job is to explain a failure.
//!
//! **It answers a different question.** Plenty of hosts drop ICMP at the
//! firewall while serving happily on their ports; plenty of others answer ping
//! from a network stack whose services are all dead. "Ping fails" and "the
//! service is unreachable" are independent facts, and reporting one as the
//! other is worse than saying nothing.
//!
//! A TCP connect to the port the connector actually uses is the same question
//! the connector is asking, one layer down, with no privileges required. It
//! also has a real cost worth naming: it opens a connection to the service. It
//! is therefore run on a debounce rather than on every failed poll, and only
//! against the port the user configured.

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use loom_core::connector::NetworkTarget;
use tokio::net::TcpStream;

/// How long a probe waits for a TCP connection.
///
/// Short on purpose. This runs inside a status poll, and a diagnostic that
/// takes longer than the poll interval would be a diagnostic that delays the
/// reading it is trying to explain. Three seconds is past any local network and
/// well past a same-host connection; a service that needs longer than that to
/// accept a socket is one this would rightly call unreachable.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// Probes `target` and returns a sentence about what is wrong, or `None` when
/// there is nothing meaningful to say.
///
/// `None` — rather than a reassuring string — when the target names no port.
/// A DNS lookup alone cannot distinguish a healthy host from a dead one, and
/// "the name resolves" is not a diagnosis.
pub async fn diagnose(target: &NetworkTarget) -> Option<String> {
    let port = target.port?;
    let host = target.host.as_str();

    // A literal address is already resolved. Running it through a resolver
    // would work, and would let a broken resolver invent a DNS failure for a
    // target that never needed DNS.
    let address = match host.parse::<IpAddr>() {
        Ok(address) => address,
        Err(_) => match resolve(host, port).await {
            Some(address) => address,
            None => {
                return Some(format!(
                    "DNS resolution failed for `{host}`. If this is a local DNS entry, check \
                     your DNS server."
                ))
            }
        },
    };

    match tokio::time::timeout(
        PROBE_TIMEOUT,
        TcpStream::connect(SocketAddr::new(address, port)),
    )
    .await
    {
        // Connected. The network path is fine and the service on the other end
        // is not, which is the one outcome that rules the network *out*.
        Ok(Ok(_stream)) => Some(
            "The host is reachable, but the service itself isn't responding. It may have \
             crashed or is misconfigured."
                .to_owned(),
        ),
        // Refused, unreachable, or timed out. Named together because the
        // remedy is the same one: go look at the host.
        Ok(Err(_)) | Err(_) => Some(format!(
            "Host `{address}` is unreachable on port `{port}`. It may be offline, or a \
             firewall is blocking the connection."
        )),
    }
}

/// First address `host` resolves to, or `None` if it resolves to nothing.
///
/// The port is part of the lookup because `lookup_host` takes an authority, not
/// a bare name; which address is chosen does not matter, because the question
/// is whether the name resolves at all.
async fn resolve(host: &str, port: u16) -> Option<IpAddr> {
    tokio::net::lookup_host((host, port))
        .await
        .ok()?
        .next()
        .map(|address| address.ip())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_target_with_no_port_has_no_diagnosis() {
        // "The name resolves" is not a diagnosis, so nothing is claimed.
        let target = NetworkTarget {
            host: "localhost".to_owned(),
            port: None,
        };
        assert_eq!(diagnose(&target).await, None);
    }

    #[tokio::test]
    async fn a_name_that_cannot_resolve_is_reported_as_dns() {
        // `.invalid` is reserved by RFC 2606 precisely so it can never resolve,
        // which makes this deterministic on any machine and on any network.
        let target = NetworkTarget::new("loom-connector-probe.invalid", 9);
        let diagnosis = diagnose(&target).await.expect("a name that cannot resolve");
        assert!(
            diagnosis.contains("DNS resolution failed"),
            "should blame DNS: {diagnosis}"
        );
        assert!(
            diagnosis.contains("loom-connector-probe.invalid"),
            "should name the host: {diagnosis}"
        );
    }

    #[tokio::test]
    async fn a_literal_address_with_nothing_listening_is_reported_as_unreachable() {
        // Loopback port 1 is reserved and nothing binds it, so the connection
        // is refused immediately — no network, no timeout, no flake.
        let target = NetworkTarget::new("127.0.0.1", 1);
        let diagnosis = diagnose(&target).await.expect("nothing is listening");
        assert!(
            diagnosis.contains("unreachable on port `1`"),
            "should blame the host and name the port: {diagnosis}"
        );
        assert!(
            !diagnosis.contains("DNS"),
            "a literal address never goes near a resolver: {diagnosis}"
        );
    }

    #[tokio::test]
    async fn a_reachable_port_clears_the_network_of_blame() {
        // A listener with no protocol behind it is exactly the third case: the
        // socket accepts, and the service says nothing useful.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("binding a loopback port");
        let port = listener.local_addr().expect("local addr").port();

        let diagnosis = diagnose(&NetworkTarget::new("127.0.0.1", port))
            .await
            .expect("a reachable port still warrants an explanation");
        assert!(
            diagnosis.contains("host is reachable"),
            "should rule the network out: {diagnosis}"
        );
    }
}
