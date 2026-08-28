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
    DockerConnector, DockerConnectorConfig, ACTION_APPLY_UPDATE, ACTION_RESTART, ACTION_START,
    ACTION_STOP, DATA_POINT_CPU_HISTORY, DATA_POINT_CPU_PERCENT, DATA_POINT_DISK_USAGE_BYTES,
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

#[tokio::test]
async fn a_reachable_unix_socket_reports_full_setup_capabilities() {
    let test_name = "a_reachable_unix_socket_reports_full_setup_capabilities";
    let host = test_docker_host();
    if !host.starts_with("unix://") {
        eprintln!("SKIPPING {test_name}: configured test host {host} is not a Unix socket");
        return;
    }
    let Some(_docker) = docker_or_skip(test_name).await else {
        return;
    };

    let connector = DockerConnector::connect(DockerConnectorConfig {
        docker_host: host,
        ..DockerConnectorConfig::default()
    })
    .await
    .expect("the already-reachable daemon must build");
    let result = connector.test_connection().await;
    assert!(result.reachable);
    assert_eq!(result.capabilities.len(), 8);
    assert!(result
        .capabilities
        .iter()
        .all(|capability| capability.available));
    assert!(result
        .capabilities
        .iter()
        .all(|capability| capability.note.is_none()));
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

fn config_for(_name: &str) -> DockerConnectorConfig {
    DockerConnectorConfig {
        docker_host: test_docker_host(),
        ..DockerConnectorConfig::default()
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
    target_id: &str,
    predicate: impl Fn(&str) -> bool,
    what: &str,
) -> loom_core::connector::ConnectorStatus {
    for _ in 0..40 {
        let status = connector.status().await.expect("status must not error");
        let state = status
            .data_point_value_for(Some(target_id), DATA_POINT_STATUS)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("");
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
    assert!(connector.supports_sub_targets());
    assert!(connector
        .list_sub_targets()
        .await
        .expect("target enumeration")
        .iter()
        .any(|target| target.id == container.name));

    let first_status = wait_for_state(
        &connector,
        &container.name,
        |state| state == "running",
        "running",
    )
    .await;
    assert!(
        matches!(
            first_status.health,
            HealthState::Healthy | HealthState::Degraded
        ),
        "a reachable daemon may degrade if optional host metrics time out: {first_status:?}"
    );

    // One-shot stats deliberately need a previous cumulative counter. The
    // first poll seeds it; the second computes a real CPU delta without holding
    // the proxy connection open for Docker's two-cycle stats response.
    let status = connector.status().await.expect("second status poll");

    // Every declared data point resolves, and with the shape its declared value
    // type promises — this is the contract a saved dashboard layout relies on.
    for descriptor in connector.data_points() {
        let value = status
            .data_point_value_for(descriptor.target_id.as_deref(), &descriptor.id)
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
    let cpu = status
        .data_point_value_for(Some(&container.name), DATA_POINT_CPU_PERCENT)
        .expect("cpu detail")
        .as_f64()
        .expect("cpu is a number");
    assert!(
        cpu > 0.0,
        "a container in a busy loop must report non-zero CPU, got {cpu}. \
         If this is 0, precpu_stats is probably not being populated."
    );
    let memory = status
        .data_point_value_for(Some(&container.name), DATA_POINT_MEMORY_USAGE_BYTES)
        .expect("memory detail")
        .as_f64()
        .expect("memory is a number");
    assert!(
        memory > 0.0,
        "a running container uses some memory, got {memory}"
    );

    // Uptime is a duration, not a timestamp and not the "not running" sentinel.
    let uptime = status
        .data_point_value_for(Some(&container.name), DATA_POINT_UPTIME)
        .expect("uptime detail")
        .as_str()
        .expect("uptime is a string");
    assert_ne!(uptime, "not running");
    assert_ne!(uptime, "unknown");

    // Logs come back, and carry what this test put in them.
    let logs = status
        .data_point_value_for(Some(&container.name), DATA_POINT_LOGS)
        .and_then(serde_json::Value::as_str)
        .expect("logs is a string");
    assert!(
        logs.contains(LOG_MARKER),
        "the log tail should contain the marker the container printed, got {logs:?}"
    );

    // History accumulates across polls rather than only holding the latest.
    let before = status
        .data_point_value_for(Some(&container.name), DATA_POINT_CPU_HISTORY)
        .expect("cpu history detail")
        .as_array()
        .expect("cpu history")
        .len();
    let later = connector.status().await.expect("second poll");
    let after = later
        .data_point_value_for(Some(&container.name), DATA_POINT_CPU_HISTORY)
        .expect("cpu history detail")
        .as_array()
        .expect("cpu history")
        .len();
    assert!(
        after > before,
        "each poll should append a history sample: {before} then {after}"
    );
}

#[tokio::test]
async fn one_host_instance_reports_the_daemon_and_lists_real_sub_targets() {
    let test_name = "one_host_instance_reports_the_daemon_and_lists_real_sub_targets";
    let Some(docker) = docker_or_skip(test_name).await else {
        return;
    };
    let container = start_test_container(&docker, "discovery").await;

    let connector = DockerConnector::connect(DockerConnectorConfig {
        docker_host: test_docker_host(),
        ..DockerConnectorConfig::default()
    })
    .await
    .expect("host mode validates only daemon reachability");

    assert_eq!(connector.metadata().id, "docker");
    assert!(connector.supports_sub_targets());
    let targets = connector.list_sub_targets().await.expect("sub-target list");
    assert!(targets.iter().any(|target| target.id == container.name));
    let actions = connector.actions().await;
    assert!(
        actions
            .iter()
            .filter(|action| action.target_id.as_deref() == Some(&container.name))
            .count()
            >= 5
    );
    let points = connector.data_points();
    assert_eq!(
        points
            .iter()
            .filter(|point| point.target_id.is_none())
            .count(),
        6
    );
    assert_eq!(
        points
            .iter()
            .filter(|point| point.target_id.as_deref() == Some(&container.name))
            .count(),
        8
    );

    let status = connector.status().await.expect("host status");
    assert!(
        matches!(status.health, HealthState::Healthy | HealthState::Degraded),
        "a reachable daemon may degrade if an optional metric times out: {status:?}"
    );
    assert!(status
        .data_point_value(DATA_POINT_TOTAL_CONTAINERS)
        .expect("container count")
        .as_i64()
        .is_some_and(|count| count >= 1));
    assert!(status
        .data_point_value(DATA_POINT_RUNNING_CONTAINERS)
        .expect("running count")
        .as_i64()
        .is_some_and(|count| count >= 1));
    assert!(status
        .data_point_value(DATA_POINT_STOPPED_CONTAINERS)
        .is_some_and(serde_json::Value::is_number));
    assert!(status
        .data_point_value(DATA_POINT_TOTAL_IMAGES)
        .is_some_and(serde_json::Value::is_number));
    assert!(status
        .data_point_value(DATA_POINT_DISK_USAGE_BYTES)
        .expect("disk usage")
        .as_i64()
        .is_some_and(|bytes| bytes >= 0));
    assert!(status
        .data_point_value(DATA_POINT_DOCKER_VERSION)
        .expect("Docker version")
        .as_str()
        .is_some_and(|version| !version.is_empty()));
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
    wait_for_state(
        &connector,
        &container.name,
        |state| state == "running",
        "running",
    )
    .await;

    let result = connector
        .execute_action(ACTION_STOP, Some(&container.name), serde_json::Value::Null)
        .await
        .expect("stop must reach Docker");
    assert!(result.success, "stop was refused: {}", result.message);

    let stopped = wait_for_state(
        &connector,
        &container.name,
        |state| state == "exited",
        "exited",
    )
    .await;
    // The daemon remains healthy; the addressed container's own status is the
    // state a target placement renders.
    assert!(
        matches!(stopped.health, HealthState::Healthy | HealthState::Degraded),
        "container state is still valid when an optional host metric timed out: {stopped:?}"
    );
    assert_eq!(
        stopped.data_point_value_for(Some(&container.name), DATA_POINT_UPTIME),
        Some(&serde_json::json!("not running"))
    );
    // The last lines before it exited are still readable, which is the whole
    // reason logs are fetched for a stopped container.
    assert!(stopped
        .data_point_value_for(Some(&container.name), DATA_POINT_LOGS)
        .expect("logs")
        .as_str()
        .is_some_and(|logs| logs.contains(LOG_MARKER)));

    // A second stop is *refused*, not an error: Docker was reached and said no,
    // so this must be `success: false` carrying Docker's own words rather than
    // an `Err` that would read as "Loom could not talk to Docker".
    let again = connector
        .execute_action(ACTION_STOP, Some(&container.name), serde_json::Value::Null)
        .await
        .expect("a refusal is still a reachable service");
    assert!(
        again.success || !again.message.is_empty(),
        "a refusal must carry Docker's message: {again:?}"
    );

    let result = connector
        .execute_action(ACTION_START, Some(&container.name), serde_json::Value::Null)
        .await
        .expect("start must reach Docker");
    assert!(result.success, "start was refused: {}", result.message);
    wait_for_state(
        &connector,
        &container.name,
        |state| state == "running",
        "running again",
    )
    .await;

    let result = connector
        .execute_action(
            ACTION_RESTART,
            Some(&container.name),
            serde_json::Value::Null,
        )
        .await
        .expect("restart must reach Docker");
    assert!(result.success, "restart was refused: {}", result.message);
    wait_for_state(
        &connector,
        &container.name,
        |state| state == "running",
        "running after restart",
    )
    .await;

    // An action this connector does not expose is refused by id, before
    // anything is sent to Docker.
    let error = connector
        .execute_action(
            "delete-everything",
            Some(&container.name),
            serde_json::Value::Null,
        )
        .await
        .expect_err("an unknown action id must be refused");
    assert!(matches!(error, ConnectorError::InvalidAction { .. }));
}

#[tokio::test]
async fn an_unreachable_host_is_rejected_at_construction() {
    let test_name = "an_unreachable_host_is_rejected_at_construction";
    let Some(_docker) = docker_or_skip(test_name).await else {
        return;
    };

    // Never reached a daemon. The host field is at fault. Port 1 is reserved
    // and nothing listens on it, so this is a connection refusal rather than a
    // timeout — and it is a loopback address, so the test needs no network.
    let error = DockerConnector::connect(DockerConnectorConfig {
        docker_host: "tcp://127.0.0.1:1".to_owned(),
        ..DockerConnectorConfig::default()
    })
    .await
    .expect_err("an unreachable host must be refused");
    assert!(
        matches!(error, ConnectorError::Unreachable { .. }),
        "an unreachable host is not a bad container name: {error}"
    );
}

/// The recreate is the one part of update management that cannot be trusted to
/// a unit test: what survives `create_container` is Docker's behaviour, not
/// ours, and a mock built from the same understanding would agree with the bug.
///
/// The "update" here deliberately moves the container to the *same* image
/// reference. What is being tested is the recreate — that a container comes back
/// with its environment, labels, volumes, restart policy and command intact —
/// and pulling a genuinely newer image would make the test depend on a public
/// registry serving a moving tag.
#[tokio::test]
async fn applying_an_update_recreates_the_container_with_its_configuration() {
    let test_name = "applying_an_update_recreates_the_container_with_its_configuration";
    let Some(docker) = docker_or_skip(test_name).await else {
        return;
    };

    let name = format!("loom-connector-docker-test-{}-update", std::process::id());
    let _ = docker
        .remove_container(
            &name,
            Some(RemoveContainerOptionsBuilder::new().force(true).build()),
        )
        .await;
    ensure_test_image(&docker).await;

    let image = format!("{TEST_IMAGE}:{TEST_TAG}");
    let volume_name = format!("{name}-data");
    docker
        .create_container(
            Some(CreateContainerOptionsBuilder::new().name(&name).build()),
            ContainerCreateBody {
                image: Some(image.clone()),
                cmd: Some(vec![
                    "/bin/sh".to_owned(),
                    "-c".to_owned(),
                    format!("echo {LOG_MARKER}; while true; do :; done"),
                ]),
                env: Some(vec![
                    "LOOM_TEST_SETTING=preserved".to_owned(),
                    "TZ=UTC".to_owned(),
                ]),
                labels: Some(std::collections::HashMap::from([(
                    "com.example.loom-test".to_owned(),
                    "preserved".to_owned(),
                )])),
                working_dir: Some("/tmp".to_owned()),
                host_config: Some(bollard::models::HostConfig {
                    binds: Some(vec![format!("{volume_name}:/data")]),
                    restart_policy: Some(bollard::models::RestartPolicy {
                        name: Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED),
                        maximum_retry_count: None,
                    }),
                    ..Default::default()
                }),
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

    let before = docker
        .inspect_container(
            &name,
            None::<bollard::query_parameters::InspectContainerOptions>,
        )
        .await
        .expect("inspecting before the update");
    let container_id_before = before.id.clone();

    let connector = DockerConnector::connect(config_for(&guard.name))
        .await
        .expect("connecting must succeed");

    let result = connector
        .execute_action(
            ACTION_APPLY_UPDATE,
            Some(&name),
            serde_json::json!({ "targetImageRef": image }),
        )
        .await
        .expect("the recreate must reach Docker");
    assert!(result.success, "applyUpdate failed: {}", result.message);

    let after = docker
        .inspect_container(
            &name,
            None::<bollard::query_parameters::InspectContainerOptions>,
        )
        .await
        .expect("inspecting after the update");

    // A genuinely new container, not the old one restarted — otherwise the
    // preservation assertions below would be testing nothing.
    assert_ne!(
        after.id, container_id_before,
        "applyUpdate must replace the container, not restart it"
    );

    let config = after
        .config
        .clone()
        .expect("the new container has a config");
    let env = config.env.clone().unwrap_or_default();
    assert!(
        env.contains(&"LOOM_TEST_SETTING=preserved".to_owned()),
        "environment was lost: {env:?}"
    );
    assert_eq!(
        config.cmd,
        before.config.as_ref().and_then(|config| config.cmd.clone()),
        "the command was lost"
    );
    assert_eq!(config.working_dir.as_deref(), Some("/tmp"));
    assert_eq!(
        config
            .labels
            .as_ref()
            .and_then(|labels| labels.get("com.example.loom-test"))
            .map(String::as_str),
        Some("preserved"),
        "labels were lost"
    );

    let host_config = after
        .host_config
        .clone()
        .expect("the new container has a host config");
    assert_eq!(
        host_config.binds,
        Some(vec![format!("{volume_name}:/data")]),
        "the volume mount was lost — this is the failure that loses someone's data directory"
    );
    assert_eq!(
        host_config.restart_policy.and_then(|policy| policy.name),
        Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED),
        "the restart policy was lost"
    );

    // And it is running again, not left stopped.
    wait_for_state(
        &connector,
        &name,
        |state| state == "running",
        "the recreated container should come back up",
    )
    .await;

    let _ = docker
        .remove_volume(
            &volume_name,
            None::<bollard::query_parameters::RemoveVolumeOptions>,
        )
        .await;
}

/// A pull that cannot succeed must leave the container alone. This is the
/// failure mode that matters most: a user with a typo in a tag should end up
/// with a running service and an explanation, not with a removed container.
#[tokio::test]
async fn a_failed_pull_leaves_the_container_running() {
    let test_name = "a_failed_pull_leaves_the_container_running";
    let Some(docker) = docker_or_skip(test_name).await else {
        return;
    };
    let container = start_test_container(&docker, "failed-pull").await;
    let connector = DockerConnector::connect(config_for(&container.name))
        .await
        .expect("connecting must succeed");

    let result = connector
        .execute_action(
            ACTION_APPLY_UPDATE,
            Some(&container.name),
            // A tag that cannot exist, on a repository that does.
            serde_json::json!({
                "targetImageRef": format!("{TEST_IMAGE}:loom-no-such-tag-ever")
            }),
        )
        .await
        .expect("a failed pull is an answered action, not a transport failure");

    assert!(
        !result.success,
        "a pull that cannot work must not report success"
    );
    assert!(
        result.message.contains("left running"),
        "the message must say the service is untouched: {}",
        result.message
    );

    // The proof: still there, still running.
    wait_for_state(
        &connector,
        &container.name,
        |state| state == "running",
        "the container should never have been touched",
    )
    .await;
}
