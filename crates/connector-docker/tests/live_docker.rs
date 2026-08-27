//! Integration tests that drive a real Docker daemon.
//!
//! These create a small, short-lived container, point a [`DockerConnector`] at
//! it, and check what the connector reports. There is no mock: the parts worth
//! testing here are the ones where Loom's understanding of the Docker API could
//! be wrong, and a mock built from the same understanding would agree with the
//! bug. The pure arithmetic — the CPU formula, the health mapping, uptime
//! formatting — is unit-tested without a daemon in `src/metrics.rs`.
//!
//! # Skipping
//!
//! Every test here begins by trying to reach the local daemon. **If Docker is
//! not available the test prints why and returns successfully** rather than
//! failing: a contributor with a laptop and no Docker should still be able to
//! run `cargo test --workspace`, which is the same principle that keeps
//! `DebugConnector` in the tree. GitHub-hosted runners do have a daemon, so
//! these run for real in CI.
//!
//! The skip is deliberately loud — `cargo test -- --nocapture` shows exactly
//! which tests were skipped and why, so "all green" cannot quietly mean
//! "nothing ran".
//!
//! Set `LOOM_TEST_DOCKER_HOST` to point these at a different endpoint, or at a
//! deliberately dead one to check that the skip path itself still works:
//!
//! ```sh
//! LOOM_TEST_DOCKER_HOST=unix:///nonexistent.sock cargo test -p loom-connector-docker -- --nocapture
//! ```
//!
//! A skip path that has never been run is a skip path that does not work, and
//! the only environment that would find out is the one without Docker — where
//! nobody is watching.

use std::time::Duration;

use bollard::models::ContainerCreateBody;
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, RemoveContainerOptionsBuilder,
};
use bollard::Docker;
use futures_util::StreamExt;
use loom_connector_docker::{
    DockerConnector, DockerConnectorConfig, ACTION_RESTART, ACTION_START, ACTION_STOP,
    DATA_POINT_CPU_HISTORY, DATA_POINT_CPU_PERCENT, DATA_POINT_DISK_USAGE_BYTES,
    DATA_POINT_DOCKER_VERSION, DATA_POINT_LOGS, DATA_POINT_MEMORY_USAGE_BYTES,
    DATA_POINT_RUNNING_CONTAINERS, DATA_POINT_STATUS, DATA_POINT_STOPPED_CONTAINERS,
    DATA_POINT_TOTAL_CONTAINERS, DATA_POINT_TOTAL_IMAGES, DATA_POINT_UPTIME, DEFAULT_DOCKER_HOST,
};
use loom_core::connector::{Connector, ConnectorError, HealthState};

/// A tiny image with a shell. Pulled rather than assumed present, because a
/// fresh CI runner has no images at all.
const TEST_IMAGE: &str = "alpine";
const TEST_TAG: &str = "3.20";

/// The marker the test container writes, so the log assertion is checking for
/// something this test put there rather than for "some output happened".
const LOG_MARKER: &str = "loom-connector-docker-live-test";

/// Reaches the local daemon, or explains why the caller should skip.
///
/// A `ping` rather than a bare connect: building a client does no I/O, so a
/// connect that "succeeds" against a dead daemon would let the test proceed and
/// fail confusingly a moment later.
async fn docker_or_skip(test_name: &str) -> Option<Docker> {
    let host = test_docker_host();
    // Two minutes, not the connector's ten: this client is the test *harness*,
    // and it pulls an image. The connector's own short timeout is right for a
    // status poll and would abort a cold pull on a fresh CI runner.
    let connected = if let Some(path) = host.strip_prefix("unix://") {
        Docker::connect_with_unix(path, 120, bollard::API_DEFAULT_VERSION)
    } else {
        Docker::connect_with_http(&host, 120, bollard::API_DEFAULT_VERSION)
    };

    let client = match connected {
        Ok(client) => client,
        Err(error) => {
            eprintln!("SKIPPING {test_name}: cannot reach Docker at {host} ({error})");
            return None;
        }
    };

    match client.ping().await {
        Ok(_) => Some(client),
        Err(error) => {
            eprintln!("SKIPPING {test_name}: Docker at {host} did not answer a ping ({error})");
            None
        }
    }
}

/// The endpoint these tests use, overridable so the skip path can be exercised
/// on a machine that *does* have Docker. Test-only: nothing in the shipped
/// connector reads the environment, because an ambient `DOCKER_HOST` silently
/// overriding a user's configured endpoint is exactly the surprise
/// `DockerConnectorConfig::connect` avoids.
fn test_docker_host() -> String {
    std::env::var("LOOM_TEST_DOCKER_HOST").unwrap_or_else(|_| DEFAULT_DOCKER_HOST.to_owned())
}

/// A container that removes itself when the test ends.
///
/// A guard rather than a cleanup call at the end of the test: an assertion
/// failure unwinds, and a test that leaks a running container on every failure
/// leaves a machine dirtier every time someone debugs it.
struct TestContainer {
    docker: Docker,
    name: String,
}

impl Drop for TestContainer {
    fn drop(&mut self) {
        let docker = self.docker.clone();
        let name = self.name.clone();
        // `Drop` cannot await. A detached task would not be polled if the
        // runtime shuts down first, so this blocks on a throwaway runtime on a
        // throwaway thread — slow, and correct even when the test panicked.
        let _ = std::thread::spawn(move || {
            let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
            else {
                return;
            };
            runtime.block_on(async {
                let options = RemoveContainerOptionsBuilder::new().force(true).build();
                let _ = docker.remove_container(&name, Some(options)).await;
            });
        })
        .join();
    }
}

/// Pulls the test image, unless it is already here.
///
/// The local check is not just an optimisation. Pulling reaches out to a public
/// registry, which is the one part of this test that depends on the internet
/// rather than on Docker, and the flakiest thing in the file. Once the image is
/// local — after the first run, or on a runner with a warm cache — these tests
/// need no network at all.
async fn ensure_test_image(docker: &Docker) {
    let reference = format!("{TEST_IMAGE}:{TEST_TAG}");
    if docker.inspect_image(&reference).await.is_ok() {
        return;
    }

    let mut pull = docker.create_image(
        Some(
            CreateImageOptionsBuilder::new()
                .from_image(TEST_IMAGE)
                .tag(TEST_TAG)
                .build(),
        ),
        None,
        None,
    );
    while let Some(progress) = pull.next().await {
        progress.unwrap_or_else(|error| {
            panic!(
                "pulling {reference} failed: {error}. These tests need either a \
                 network path to the registry or {reference} already present locally."
            )
        });
    }
}

/// Creates and starts a container that prints a marker line and then idles.
async fn start_test_container(docker: &Docker, suffix: &str) -> TestContainer {
    // Unique per run and per test, so two tests — or two checkouts on one
    // machine — cannot collide on a name.
    let name = format!("loom-connector-docker-test-{}-{suffix}", std::process::id());

    // Best-effort removal of a leftover from an earlier interrupted run.
    let _ = docker
        .remove_container(
            &name,
            Some(RemoveContainerOptionsBuilder::new().force(true).build()),
        )
        .await;

    ensure_test_image(docker).await;

    docker
        .create_container(
            Some(CreateContainerOptionsBuilder::new().name(&name).build()),
            ContainerCreateBody {
                image: Some(format!("{TEST_IMAGE}:{TEST_TAG}")),
                // Prints the marker once, then burns a little CPU forever so
                // there is something for the stats sample to measure.
                //
                // Deliberately signal-deaf: `sh` as PID 1 with no trap ignores
                // SIGTERM, so `docker stop` waits out the full grace period
                // before SIGKILL. That is exactly the case that caught the
                // connector sharing one timeout between polls and lifecycle
                // actions, so the slow stop stays in the test on purpose.
                cmd: Some(vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    format!("echo {LOG_MARKER}; while true; do :; done"),
                ]),
                ..Default::default()
            },
        )
        .await
        .expect("creating the test container must succeed");

    let guard = TestContainer {
        docker: docker.clone(),
        name: name.clone(),
    };
    docker
        .start_container(&name, None)
        .await
        .expect("starting the test container must succeed");

    guard
}

fn config_for(name: &str) -> DockerConnectorConfig {
    DockerConnectorConfig {
        docker_host: test_docker_host(),
        container_name: Some(name.to_owned()),
    }
}

/// Polls until `predicate` accepts the reported state, or gives up.
///
/// Docker's lifecycle calls return once the daemon has accepted the request,
/// not once the container has finished reacting, so asserting immediately after
/// a stop is a race. Bounded so a genuine failure is a timeout with a message
/// rather than a hang.
async fn wait_for_state(
    connector: &DockerConnector,
    predicate: impl Fn(&str) -> bool,
    what: &str,
) -> loom_core::connector::ConnectorStatus {
    for _ in 0..40 {
        let status = connector.status().await.expect("status must not error");
        let state = status.details[DATA_POINT_STATUS].as_str().unwrap_or("");
        if predicate(state) {
            return status;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    panic!("the container never reached the state we were waiting for: {what}");
}

#[tokio::test]
async fn a_running_container_reports_healthy_with_every_data_point() {
    let test_name = "a_running_container_reports_healthy_with_every_data_point";
    let Some(docker) = docker_or_skip(test_name).await else {
        return;
    };
    let container = start_test_container(&docker, "running").await;

    let connector = DockerConnector::connect(config_for(&container.name))
        .await
        .expect("connecting to a container that exists must succeed");
    assert_eq!(connector.discoverable_type(), None);
    assert_eq!(
        connector.discovery_target_field().as_deref(),
        Some("containerName")
    );
    assert!(connector
        .discover()
        .await
        .expect("container mode discovery is a safe no-op")
        .is_empty());

    let status = wait_for_state(&connector, |state| state == "running", "running").await;
    assert_eq!(status.health, HealthState::Healthy);

    // Every declared data point resolves, and with the shape its declared value
    // type promises — this is the contract a saved dashboard layout relies on.
    let details = status.details.as_object().expect("details is an object");
    for descriptor in connector.data_points() {
        let value = details
            .get(&descriptor.id)
            .unwrap_or_else(|| panic!("details is missing data point {}", descriptor.id));
        match descriptor.value_type {
            loom_core::connector::DataPointValueType::Number => assert!(
                value.is_number(),
                "{} should be a number, got {value}",
                descriptor.id
            ),
            loom_core::connector::DataPointValueType::String => assert!(
                value.is_string(),
                "{} should be a string, got {value}",
                descriptor.id
            ),
            loom_core::connector::DataPointValueType::TimeSeries => {
                let points = value
                    .as_array()
                    .unwrap_or_else(|| panic!("{} should be an array", descriptor.id));
                for point in points {
                    assert!(point["timestamp"].is_string(), "{point} needs a timestamp");
                    assert!(point["value"].is_number(), "{point} needs a value");
                }
            }
            other => panic!(
                "{} declares an unexpected value type {other:?}",
                descriptor.id
            ),
        }
    }

    // The container is deliberately spinning, so this is a real measurement
    // rather than a zero that would also pass if the formula were broken.
    let cpu = details[DATA_POINT_CPU_PERCENT]
        .as_f64()
        .expect("cpu is a number");
    assert!(
        cpu > 0.0,
        "a container in a busy loop must report non-zero CPU, got {cpu}. \
         If this is 0, precpu_stats is probably not being populated."
    );
    let memory = details[DATA_POINT_MEMORY_USAGE_BYTES]
        .as_f64()
        .expect("memory is a number");
    assert!(
        memory > 0.0,
        "a running container uses some memory, got {memory}"
    );

    // Uptime is a duration, not a timestamp and not the "not running" sentinel.
    let uptime = details[DATA_POINT_UPTIME]
        .as_str()
        .expect("uptime is a string");
    assert_ne!(uptime, "not running");
    assert_ne!(uptime, "unknown");

    // Logs come back, and carry what this test put in them.
    let logs = details[DATA_POINT_LOGS].as_str().expect("logs is a string");
    assert!(
        logs.contains(LOG_MARKER),
        "the log tail should contain the marker the container printed, got {logs:?}"
    );

    // History accumulates across polls rather than only holding the latest.
    let before = status.details[DATA_POINT_CPU_HISTORY]
        .as_array()
        .expect("cpu history")
        .len();
    let later = connector.status().await.expect("second poll");
    let after = later.details[DATA_POINT_CPU_HISTORY]
        .as_array()
        .expect("cpu history")
        .len();
    assert!(
        after > before,
        "each poll should append a history sample: {before} then {after}"
    );
}

#[tokio::test]
async fn host_mode_reports_the_daemon_and_discovers_real_containers() {
    let test_name = "host_mode_reports_the_daemon_and_discovers_real_containers";
    let Some(docker) = docker_or_skip(test_name).await else {
        return;
    };
    let container = start_test_container(&docker, "discovery").await;

    let connector = DockerConnector::connect(DockerConnectorConfig {
        docker_host: test_docker_host(),
        container_name: None,
    })
    .await
    .expect("host mode validates only daemon reachability");

    assert_eq!(connector.metadata().id, "docker");
    assert_eq!(connector.discoverable_type().as_deref(), Some("docker"));
    assert_eq!(
        connector.discovery_target_field().as_deref(),
        Some("containerName")
    );
    assert!(connector.actions().await.is_empty());
    assert_eq!(
        connector
            .data_points()
            .into_iter()
            .map(|point| point.id)
            .collect::<Vec<_>>(),
        [
            DATA_POINT_TOTAL_CONTAINERS,
            DATA_POINT_RUNNING_CONTAINERS,
            DATA_POINT_STOPPED_CONTAINERS,
            DATA_POINT_TOTAL_IMAGES,
            DATA_POINT_DISK_USAGE_BYTES,
            DATA_POINT_DOCKER_VERSION,
        ]
    );

    let status = connector.status().await.expect("host status");
    assert!(
        matches!(status.health, HealthState::Healthy | HealthState::Degraded),
        "a reachable daemon may degrade if an optional metric times out: {status:?}"
    );
    assert!(status.details[DATA_POINT_TOTAL_CONTAINERS]
        .as_i64()
        .is_some_and(|count| count >= 1));
    assert!(status.details[DATA_POINT_RUNNING_CONTAINERS]
        .as_i64()
        .is_some_and(|count| count >= 1));
    assert!(status.details[DATA_POINT_STOPPED_CONTAINERS].is_number());
    assert!(status.details[DATA_POINT_TOTAL_IMAGES].is_number());
    assert!(status.details[DATA_POINT_DISK_USAGE_BYTES]
        .as_i64()
        .is_some_and(|bytes| bytes >= 0));
    assert!(status.details[DATA_POINT_DOCKER_VERSION]
        .as_str()
        .is_some_and(|version| !version.is_empty()));

    let resources = connector.discover().await.expect("host discovery");
    let discovered = resources
        .iter()
        .find(|resource| resource.suggested_name == container.name)
        .unwrap_or_else(|| panic!("test container was not discovered: {resources:#?}"));
    assert_eq!(discovered.target_connector_type, "docker");
    assert_eq!(discovered.config["dockerHost"], test_docker_host());
    assert_eq!(discovered.config["containerName"], container.name);
    assert_eq!(
        discovered.target_field_value.as_ref(),
        Some(&serde_json::json!(container.name))
    );
}

#[tokio::test]
async fn stopping_and_restarting_a_container_moves_its_reported_state() {
    let test_name = "stopping_and_restarting_a_container_moves_its_reported_state";
    let Some(docker) = docker_or_skip(test_name).await else {
        return;
    };
    let container = start_test_container(&docker, "lifecycle").await;

    let connector = DockerConnector::connect(config_for(&container.name))
        .await
        .expect("connecting must succeed");
    wait_for_state(&connector, |state| state == "running", "running").await;

    let result = connector
        .execute_action(ACTION_STOP, serde_json::Value::Null)
        .await
        .expect("stop must reach Docker");
    assert!(result.success, "stop was refused: {}", result.message);

    let stopped = wait_for_state(&connector, |state| state == "exited", "exited").await;
    assert_eq!(stopped.health, HealthState::Down);
    assert_eq!(stopped.details[DATA_POINT_UPTIME], "not running");
    // The last lines before it exited are still readable, which is the whole
    // reason logs are fetched for a stopped container.
    assert!(stopped.details[DATA_POINT_LOGS]
        .as_str()
        .is_some_and(|logs| logs.contains(LOG_MARKER)));

    // A second stop is *refused*, not an error: Docker was reached and said no,
    // so this must be `success: false` carrying Docker's own words rather than
    // an `Err` that would read as "Loom could not talk to Docker".
    let again = connector
        .execute_action(ACTION_STOP, serde_json::Value::Null)
        .await
        .expect("a refusal is still a reachable service");
    assert!(
        again.success || !again.message.is_empty(),
        "a refusal must carry Docker's message: {again:?}"
    );

    let result = connector
        .execute_action(ACTION_START, serde_json::Value::Null)
        .await
        .expect("start must reach Docker");
    assert!(result.success, "start was refused: {}", result.message);
    wait_for_state(&connector, |state| state == "running", "running again").await;

    let result = connector
        .execute_action(ACTION_RESTART, serde_json::Value::Null)
        .await
        .expect("restart must reach Docker");
    assert!(result.success, "restart was refused: {}", result.message);
    wait_for_state(
        &connector,
        |state| state == "running",
        "running after restart",
    )
    .await;

    // An action this connector does not expose is refused by id, before
    // anything is sent to Docker.
    let error = connector
        .execute_action("delete-everything", serde_json::Value::Null)
        .await
        .expect_err("an unknown action id must be refused");
    assert!(matches!(error, ConnectorError::InvalidAction { .. }));
}

/// The two ways a configuration can be wrong have to stay distinguishable —
/// that distinction is most of the value of validating at construction time.
#[tokio::test]
async fn a_bad_configuration_says_which_half_is_wrong() {
    let test_name = "a_bad_configuration_says_which_half_is_wrong";
    let Some(_docker) = docker_or_skip(test_name).await else {
        return;
    };

    // Reached the daemon; it has no such container. The name field is at fault.
    let error = DockerConnector::connect(config_for(
        "loom-connector-docker-test-no-such-container-ever",
    ))
    .await
    .expect_err("a container that does not exist must be refused");
    assert!(
        matches!(error, ConnectorError::InvalidParams { .. }),
        "a missing container is a bad parameter, not an unreachable host: {error}"
    );
    assert!(
        error.to_string().contains("no container named"),
        "the message must point at the name: {error}"
    );

    // Never reached a daemon. The host field is at fault. Port 1 is reserved
    // and nothing listens on it, so this is a connection refusal rather than a
    // timeout — and it is a loopback address, so the test needs no network.
    let error = DockerConnector::connect(DockerConnectorConfig {
        docker_host: "tcp://127.0.0.1:1".to_owned(),
        container_name: Some("anything".to_owned()),
    })
    .await
    .expect_err("an unreachable host must be refused");
    assert!(
        matches!(error, ConnectorError::Unreachable { .. }),
        "an unreachable host is not a bad container name: {error}"
    );
}
