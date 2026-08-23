//! The arithmetic and the mappings, kept apart from the I/O.
//!
//! Everything in this module is a pure function over values that happen to
//! come from Docker. That is the point: the CPU formula and the state→health
//! mapping are where this connector is most likely to be quietly wrong, and
//! neither needs a daemon to test. The parts that do need one live in
//! `connector.rs` and in `tests/live_docker.rs`.

use bollard::models::{ContainerCpuStats, ContainerStateStatusEnum};
use chrono::{DateTime, Utc};
use loom_core::connector::HealthState;

/// Docker's documented CPU-percentage formula.
///
/// ```text
/// cpu_delta        = cpu_stats.cpu_usage.total_usage - precpu_stats.cpu_usage.total_usage
/// system_cpu_delta = cpu_stats.system_cpu_usage      - precpu_stats.system_cpu_usage
/// percent          = (cpu_delta / system_cpu_delta) * online_cpus * 100
/// ```
///
/// This is the same calculation `docker stats` performs, and it is a
/// *percentage of one CPU multiplied by the core count* — so a container fully
/// saturating four cores reads 400%, not 100%. That is Docker's convention and
/// deviating from it would make Loom disagree with the tool people cross-check
/// against.
///
/// Returns `0.0` rather than a `NaN` or a panic in every degenerate case:
///
/// - `system_cpu_delta == 0`, which happens on the first sample of a container
///   and whenever two samples land in the same scheduler tick;
/// - counters that went backwards (a restarted container resets them), which
///   would otherwise produce a negative percentage;
/// - any of the four fields being absent, which is how Docker reports a
///   Windows container or a stopped one.
///
/// `0.0` is honest here in a way `None` would not be: "we have no delta to
/// measure across" and "it used no CPU in that interval" are indistinguishable
/// from outside, and a gauge that blanks out every time a container restarts is
/// worse than one that reads zero for one poll.
pub fn cpu_percent(
    current: Option<&ContainerCpuStats>,
    previous: Option<&ContainerCpuStats>,
) -> f64 {
    let (Some(current), Some(previous)) = (current, previous) else {
        return 0.0;
    };

    let total = current
        .cpu_usage
        .as_ref()
        .and_then(|usage| usage.total_usage);
    let pre_total = previous
        .cpu_usage
        .as_ref()
        .and_then(|usage| usage.total_usage);
    let (Some(total), Some(pre_total)) = (total, pre_total) else {
        return 0.0;
    };
    let (Some(system), Some(pre_system)) = (current.system_cpu_usage, previous.system_cpu_usage)
    else {
        return 0.0;
    };

    // Saturating, not wrapping: these are `u64` counters, and a container that
    // restarted between samples resets them. `total - pre_total` would panic in
    // debug and wrap to something astronomical in release.
    let cpu_delta = total.saturating_sub(pre_total) as f64;
    let system_delta = system.saturating_sub(pre_system) as f64;
    if system_delta <= 0.0 {
        return 0.0;
    }

    // `online_cpus` is Linux-only and omitted for Windows containers; one core
    // is the conservative assumption, and it keeps the reading finite rather
    // than dropping it.
    let cores = f64::from(current.online_cpus.unwrap_or(1).max(1));
    let percent = (cpu_delta / system_delta) * cores * 100.0;

    // Two decimals, matching how the rest of the tree rounds a percentage
    // before it reaches a widget.
    if percent.is_finite() && percent > 0.0 {
        (percent * 100.0).round() / 100.0
    } else {
        0.0
    }
}

/// Maps Docker's container state onto the coarse verdict dashboards colour.
///
/// | Docker state | Health | Why |
/// | --- | --- | --- |
/// | `running` | Healthy | Serving. |
/// | `restarting`, `removing`, `stopping` | Degraded | Transitional. It is doing something, and the next poll will say what. |
/// | `paused` | Degraded | Deliberately suspended, not broken. Down would send someone looking for a crash. |
/// | `exited`, `dead`, `created` | Down | Not serving. `created` is included because a container that has never been started is as unavailable as one that stopped — Unknown would suggest Loom could not find out, when it knows exactly. |
/// | anything else | Unknown | Docker returned a state this build does not know. Reported as such rather than guessed at. |
///
/// Note that `paused` cannot be inferred from the `Running` boolean: Docker
/// documents a paused container as both `Running` **and** `Paused`, which is
/// why this reads `Status` and nothing else.
pub fn health_for_state(state: Option<ContainerStateStatusEnum>) -> HealthState {
    match state {
        Some(ContainerStateStatusEnum::RUNNING) => HealthState::Healthy,
        Some(
            ContainerStateStatusEnum::RESTARTING
            | ContainerStateStatusEnum::PAUSED
            | ContainerStateStatusEnum::REMOVING
            | ContainerStateStatusEnum::STOPPING,
        ) => HealthState::Degraded,
        Some(
            ContainerStateStatusEnum::EXITED
            | ContainerStateStatusEnum::DEAD
            | ContainerStateStatusEnum::CREATED,
        ) => HealthState::Down,
        _ => HealthState::Unknown,
    }
}

/// How long the container has been up, as text.
///
/// A `String` data point rather than a number of seconds, because the only
/// consumer is a stat tile and "3d 4h" is what a person wants to read there.
/// A caller that needs arithmetic should use `StartedAt` directly.
///
/// `started_at` is Docker's `State.StartedAt`, RFC 3339. Docker reports
/// `0001-01-01T00:00:00Z` for a container that has never run, which is far
/// enough in the past that a naive subtraction would claim two thousand years
/// of uptime — so anything that is not currently running reports
/// `"not running"` instead, and so does a timestamp in the future.
pub fn format_uptime(started_at: Option<&str>, running: bool, now: DateTime<Utc>) -> String {
    if !running {
        return "not running".to_owned();
    }
    let Some(started) = started_at.and_then(|value| DateTime::parse_from_rfc3339(value).ok())
    else {
        return "unknown".to_owned();
    };

    let seconds = now
        .signed_duration_since(started.with_timezone(&Utc))
        .num_seconds();
    if seconds < 0 {
        return "unknown".to_owned();
    }
    format_duration(seconds)
}

/// `93784` → `"1d 2h 3m"`. Two units at most: a third is noise at every scale.
fn format_duration(total_seconds: i64) -> String {
    let days = total_seconds / 86_400;
    let hours = (total_seconds % 86_400) / 3_600;
    let minutes = (total_seconds % 3_600) / 60;
    let seconds = total_seconds % 60;

    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else if minutes > 0 {
        format!("{minutes}m {seconds}s")
    } else {
        format!("{seconds}s")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::ContainerCpuUsage;

    /// Builds the half of `ContainerCpuStats` the formula reads.
    fn stats(total_usage: u64, system_cpu_usage: u64, online_cpus: u32) -> ContainerCpuStats {
        ContainerCpuStats {
            cpu_usage: Some(ContainerCpuUsage {
                total_usage: Some(total_usage),
                ..Default::default()
            }),
            system_cpu_usage: Some(system_cpu_usage),
            online_cpus: Some(online_cpus),
            ..Default::default()
        }
    }

    #[test]
    fn cpu_percent_matches_dockers_documented_formula() {
        // A tenth of the system's CPU time across the interval, on four cores:
        // (100 / 1000) * 4 * 100 = 40%.
        let current = stats(1_100, 11_000, 4);
        let previous = stats(1_000, 10_000, 4);
        assert_eq!(cpu_percent(Some(&current), Some(&previous)), 40.0);

        // The same deltas on one core is a quarter of that.
        let current = stats(1_100, 11_000, 1);
        let previous = stats(1_000, 10_000, 1);
        assert_eq!(cpu_percent(Some(&current), Some(&previous)), 10.0);

        // Saturating one core out of eight reads 100%, not 12.5%: Docker's
        // percentage is per-core-multiplied, and `docker stats` shows the same.
        let current = stats(1_000, 8_000, 8);
        let previous = stats(0, 0, 8);
        assert_eq!(cpu_percent(Some(&current), Some(&previous)), 100.0);
    }

    #[test]
    fn cpu_percent_is_zero_rather_than_nan_when_there_is_no_interval() {
        // The division-by-zero case: identical system counters. This is what a
        // container's very first sample looks like, so it is not exotic.
        let current = stats(1_100, 10_000, 4);
        let previous = stats(1_000, 10_000, 4);
        let percent = cpu_percent(Some(&current), Some(&previous));
        assert_eq!(percent, 0.0);
        assert!(percent.is_finite(), "must never produce NaN or infinity");

        // Both counters at zero, which is what Docker sends for a container
        // that is not running.
        assert_eq!(
            cpu_percent(Some(&stats(0, 0, 4)), Some(&stats(0, 0, 4))),
            0.0
        );
    }

    #[test]
    fn cpu_percent_survives_counters_that_went_backwards() {
        // A restart resets the counters, so `current` can be lower than
        // `previous`. Subtracting would panic in debug and wrap in release.
        let current = stats(5, 10, 2);
        let previous = stats(1_000, 10_000, 2);
        assert_eq!(cpu_percent(Some(&current), Some(&previous)), 0.0);
    }

    #[test]
    fn cpu_percent_is_zero_when_docker_omits_the_fields() {
        let complete = stats(1_100, 11_000, 4);
        assert_eq!(cpu_percent(None, Some(&complete)), 0.0);
        assert_eq!(cpu_percent(Some(&complete), None), 0.0);
        assert_eq!(cpu_percent(None, None), 0.0);

        // Windows containers omit `system_cpu_usage` entirely.
        let windows = ContainerCpuStats {
            cpu_usage: Some(ContainerCpuUsage {
                total_usage: Some(1_100),
                ..Default::default()
            }),
            system_cpu_usage: None,
            online_cpus: None,
            ..Default::default()
        };
        assert_eq!(cpu_percent(Some(&windows), Some(&complete)), 0.0);
    }

    #[test]
    fn health_maps_every_docker_state_deliberately() {
        use ContainerStateStatusEnum as State;

        assert_eq!(health_for_state(Some(State::RUNNING)), HealthState::Healthy);

        for transitional in [
            State::RESTARTING,
            State::PAUSED,
            State::REMOVING,
            State::STOPPING,
        ] {
            assert_eq!(
                health_for_state(Some(transitional)),
                HealthState::Degraded,
                "{transitional} should be Degraded"
            );
        }

        for stopped in [State::EXITED, State::DEAD, State::CREATED] {
            assert_eq!(
                health_for_state(Some(stopped)),
                HealthState::Down,
                "{stopped} should be Down"
            );
        }

        // A state Docker did not give us, and the empty string it sends for a
        // container it cannot describe, are both Unknown rather than assumed.
        assert_eq!(health_for_state(None), HealthState::Unknown);
        assert_eq!(health_for_state(Some(State::EMPTY)), HealthState::Unknown);
    }

    #[test]
    fn uptime_reads_as_a_duration_not_a_timestamp() {
        let now = DateTime::parse_from_rfc3339("2026-08-23T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        assert_eq!(
            format_uptime(Some("2026-08-23T11:59:15Z"), true, now),
            "45s"
        );
        assert_eq!(
            format_uptime(Some("2026-08-23T11:30:30Z"), true, now),
            "29m 30s"
        );
        assert_eq!(
            format_uptime(Some("2026-08-23T09:45:00Z"), true, now),
            "2h 15m"
        );
        assert_eq!(
            format_uptime(Some("2026-08-20T02:00:00Z"), true, now),
            "3d 10h"
        );
    }

    #[test]
    fn uptime_refuses_to_invent_one() {
        let now = DateTime::parse_from_rfc3339("2026-08-23T12:00:00Z")
            .unwrap()
            .with_timezone(&Utc);

        // Docker's zero value for a container that has never run. Subtracting
        // it would claim two millennia of uptime.
        assert_eq!(
            format_uptime(Some("0001-01-01T00:00:00Z"), false, now),
            "not running"
        );
        assert_eq!(
            format_uptime(Some("2026-08-23T11:00:00Z"), false, now),
            "not running"
        );
        assert_eq!(format_uptime(None, true, now), "unknown");
        assert_eq!(format_uptime(Some("not a timestamp"), true, now), "unknown");
        // A clock skew between Loom and the Docker host, rather than negative
        // uptime rendered as a duration.
        assert_eq!(
            format_uptime(Some("2026-08-23T12:30:00Z"), true, now),
            "unknown"
        );
    }
}
