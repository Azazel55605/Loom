//! Update checking and applying, for one Docker host.
//!
//! Two halves that only touch through an image reference:
//!
//! - **Checking** asks the registry what a tag currently points at and compares
//!   it with what the daemon recorded when it pulled — see [`crate::registry`].
//!   It downloads nothing and changes nothing.
//! - **Applying** pulls a reference and recreates the container on it, keeping
//!   the configuration the running container was created with.
//!
//! The apply half takes the image reference as a *parameter* rather than
//! working it out. That is what makes one action serve both directions: given
//! the newer reference it is an update, and given the reference the action log
//! recorded before the last update it is a rollback. Nothing here knows which
//! of the two it is doing, and there is deliberately no second action, no
//! stored "previous image" field, and no rollback bookkeeping — see
//! `docs/adr/0023-docker-update-management.md`.

use std::collections::HashMap;

use bollard::models::{ContainerCreateBody, ContainerInspectResponse};
use bollard::query_parameters::{
    CreateContainerOptionsBuilder, CreateImageOptionsBuilder, InspectContainerOptions,
    RemoveContainerOptionsBuilder,
};
use bollard::Docker;
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use loom_core::connector::{ActionResult, ConnectorError, UpdateCheckResult};
use serde_json::json;

use crate::registry::{current_digest, is_outdated, ImageReference, RegistryTransport};

/// One container's last update check, as this connector remembers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateReading {
    /// The image reference the container is running, as configured.
    pub current_ref: String,
    /// Whether the registry is serving something newer for that reference.
    pub available: bool,
    /// The registry's current digest for the tag, when one was read.
    pub latest_ref: Option<String>,
    /// When the check ran.
    pub checked_at: DateTime<Utc>,
}

/// What each container was found to be running, keyed by container name.
pub type UpdateCache = HashMap<String, UpdateReading>;

/// The image reference a container was created from.
///
/// `Config.Image` rather than the resolved `Image` id: the id is a digest,
/// which answers "what is running" and not "what should this follow". An update
/// check needs the tag a human wrote, because a tag is the only thing that can
/// point somewhere new.
pub fn configured_image_ref(inspect: &ContainerInspectResponse) -> Option<String> {
    inspect
        .config
        .as_ref()
        .and_then(|config| config.image.clone())
        .filter(|image| !image.trim().is_empty())
}

/// Checks one container's image against its registry.
///
/// Returns "up to date" rather than an error for a container whose image cannot
/// be checked at all — one pinned to a digest or named by id, and one built
/// locally and never pushed (no repository digests, so nothing upstream to
/// compare against). Neither is a failure: there is genuinely no newer version
/// of an image that names one exact digest, and reporting an error would put a
/// permanent red mark on a container that is working fine.
pub async fn check_container(
    docker: &Docker,
    transport: &dyn RegistryTransport,
    container_name: &str,
) -> Result<UpdateReading, ConnectorError> {
    let inspect = docker
        .inspect_container(container_name, None::<InspectContainerOptions>)
        .await
        .map_err(|error| {
            ConnectorError::unreachable(format!(
                "could not inspect {container_name} before checking for updates: {error}"
            ))
        })?;

    let now = Utc::now();
    let Some(image_ref) = configured_image_ref(&inspect) else {
        return Err(ConnectorError::Internal(format!(
            "{container_name} does not report the image it was created from"
        )));
    };

    let Some(reference) = ImageReference::parse(&image_ref) else {
        return Ok(UpdateReading {
            current_ref: image_ref,
            available: false,
            latest_ref: None,
            checked_at: now,
        });
    };

    let local = docker
        .inspect_image(&image_ref)
        .await
        .map_err(|error| {
            ConnectorError::unreachable(format!(
                "could not inspect the local image {image_ref}: {error}"
            ))
        })?
        .repo_digests
        .unwrap_or_default();

    // An image with no repository digests was never pulled from a registry —
    // it was built here. There is no upstream version of it to be behind, and
    // asking a registry about it produces a query for a repository that does
    // not exist, whose refusal reads to a user as "your private repository
    // needs credentials". Observed against a real daemon: a homelab that builds
    // its own images filled the log with that. The check is skipped, which is
    // both the correct answer and one fewer request.
    if local.is_empty() {
        return Ok(UpdateReading {
            current_ref: image_ref,
            available: false,
            latest_ref: None,
            checked_at: now,
        });
    }

    let digest = current_digest(transport, &reference).await?;

    Ok(UpdateReading {
        available: is_outdated(&local, &digest),
        current_ref: image_ref,
        latest_ref: Some(format!("{}@{digest}", reference.repository)),
        checked_at: now,
    })
}

impl UpdateReading {
    /// The trait-level answer this reading corresponds to.
    pub fn as_result(&self) -> UpdateCheckResult {
        UpdateCheckResult {
            available: self.available,
            latest_ref: self.available.then(|| self.latest_ref.clone()).flatten(),
        }
    }
}

/// Recreates `container_name` on `target_image_ref`, preserving its
/// configuration.
///
/// The sequence is pull, inspect, stop, remove, create, start — and the order
/// matters. The pull happens *first* so a registry failure costs nothing: the
/// container is still running, untouched, and the action reports why. Only once
/// the new image is on the host does anything get taken down.
///
/// The configuration carried over is the container's own creation config
/// (environment, labels, entrypoint, exposed ports) together with its
/// `HostConfig` (volume mounts, port bindings, restart policy, resource limits)
/// and its network attachments. What is deliberately *not* carried over is the
/// image: that is the one field being changed.
pub async fn apply_update(
    control: &Docker,
    container_name: &str,
    target_image_ref: &str,
) -> Result<ActionResult, ConnectorError> {
    let target_image_ref = target_image_ref.trim();
    if target_image_ref.is_empty() {
        return Err(ConnectorError::InvalidParams {
            action_id: "applyUpdate".to_owned(),
            reason: "expected a non-empty string `targetImageRef`".to_owned(),
        });
    }

    let inspect = control
        .inspect_container(container_name, None::<InspectContainerOptions>)
        .await
        .map_err(|error| {
            ConnectorError::unreachable(format!(
                "could not inspect {container_name}: {error}. Nothing was changed."
            ))
        })?;

    if let Err(message) = pull_image(control, target_image_ref).await {
        // A failed pull is the safe failure, and the message says so
        // explicitly: someone reading it needs to know immediately whether
        // their service is still up.
        return Ok(ActionResult::failed(format!(
            "{container_name}: could not pull {target_image_ref} ({message}). The container was \
             left running as it was."
        )));
    }

    let create_body = recreate_body(&inspect, target_image_ref);
    let network_config = inspect
        .network_settings
        .as_ref()
        .and_then(|settings| settings.networks.clone());

    if let Err(error) = control.stop_container(container_name, None).await {
        return Ok(ActionResult::failed(format!(
            "{container_name}: the new image was pulled but the container could not be stopped \
             ({error}). Nothing was replaced."
        )));
    }

    let remove_options = RemoveContainerOptionsBuilder::new().force(true).build();
    if let Err(error) = control
        .remove_container(container_name, Some(remove_options))
        .await
    {
        return Ok(ActionResult::failed(format!(
            "{container_name}: the container was stopped but could not be removed ({error}). It \
             is stopped and still present; start it to return to the previous image."
        )));
    }

    let create_options = CreateContainerOptionsBuilder::new()
        .name(container_name)
        .build();
    if let Err(error) = control
        .create_container(Some(create_options), create_body)
        .await
    {
        // The worst point to fail at, so the message has to say exactly what
        // state the host is in and what the operator's options are.
        return Ok(ActionResult::failed(format!(
            "{container_name}: the old container was removed but the new one could not be \
             created ({error}). The service is down; recreating it from {target_image_ref} by \
             hand, or from the previous reference in the action log, will restore it."
        )));
    }

    // Networks beyond the first have to be attached after creation: the create
    // call carries at most one endpoint, so a container on two networks would
    // silently come back on one.
    let mut network_warnings = Vec::new();
    if let Some(networks) = network_config {
        for (name, endpoint) in networks {
            if name == "bridge" && endpoint.network_id.is_none() {
                continue;
            }
            let connect = bollard::models::NetworkConnectRequest {
                container: container_name.to_owned(),
                endpoint_config: Some(endpoint),
            };
            if let Err(error) = control.connect_network(&name, connect).await {
                network_warnings.push(format!("{name} ({error})"));
            }
        }
    }

    if let Err(error) = control.start_container(container_name, None).await {
        return Ok(ActionResult::failed(format!(
            "{container_name}: recreated on {target_image_ref} but it would not start ({error}). \
             The container exists and is stopped."
        )));
    }

    let mut message = format!("{container_name}: now running {target_image_ref}.");
    if !network_warnings.is_empty() {
        message.push_str(&format!(
            " Some networks could not be reattached: {}.",
            network_warnings.join(", ")
        ));
    }

    Ok(ActionResult::ok(message).with_payload(json!({
        "containerName": container_name,
        "imageRef": target_image_ref,
        "reattachmentFailures": network_warnings,
    })))
}

/// Pulls an image reference, draining the progress stream.
///
/// The stream has to be read to completion or the pull is abandoned half-done;
/// the progress frames themselves are of no interest here, but an error frame
/// is — Docker reports "manifest unknown" and "unauthorized" that way rather
/// than as a transport failure.
async fn pull_image(docker: &Docker, image_ref: &str) -> Result<(), String> {
    let options = CreateImageOptionsBuilder::new()
        .from_image(image_ref)
        .build();
    let mut stream = docker.create_image(Some(options), None, None);

    while let Some(frame) = stream.next().await {
        match frame {
            Ok(info) => {
                if let Some(detail) = info.error_detail {
                    return Err(detail
                        .message
                        .unwrap_or_else(|| "the pull failed".to_owned())
                        .trim()
                        .to_owned());
                }
            }
            Err(error) => return Err(error.to_string()),
        }
    }

    Ok(())
}

/// The creation request for the replacement container.
///
/// Everything the old container was created with, with the image swapped. Split
/// out as a pure function so the preservation can be asserted on without a
/// daemon: what survives a recreate is the whole point of the action, and a
/// test that had to create a real container to check one field would not be
/// written for every field.
pub fn recreate_body(
    inspect: &ContainerInspectResponse,
    target_image_ref: &str,
) -> ContainerCreateBody {
    let config = inspect.config.clone().unwrap_or_default();

    ContainerCreateBody {
        hostname: config.hostname,
        domainname: config.domainname,
        user: config.user,
        attach_stdin: config.attach_stdin,
        attach_stdout: config.attach_stdout,
        attach_stderr: config.attach_stderr,
        exposed_ports: config.exposed_ports,
        tty: config.tty,
        open_stdin: config.open_stdin,
        stdin_once: config.stdin_once,
        env: config.env,
        cmd: config.cmd,
        healthcheck: config.healthcheck,
        args_escaped: config.args_escaped,
        // The one field that changes.
        image: Some(target_image_ref.to_owned()),
        volumes: config.volumes,
        working_dir: config.working_dir,
        entrypoint: config.entrypoint,
        network_disabled: config.network_disabled,
        on_build: config.on_build,
        labels: config.labels,
        stop_signal: config.stop_signal,
        stop_timeout: config.stop_timeout,
        shell: config.shell,
        // Mounts, port bindings, restart policy, resource limits, devices,
        // capabilities: everything that makes the container *this* container
        // rather than a fresh one from the same image.
        host_config: inspect.host_config.clone(),
        networking_config: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{ContainerConfig, HostConfig, PortBinding, RestartPolicy};
    use std::collections::HashMap;

    fn inspected() -> ContainerInspectResponse {
        ContainerInspectResponse {
            config: Some(ContainerConfig {
                image: Some("example/app:1.0".to_owned()),
                env: Some(vec!["TZ=UTC".to_owned(), "MODE=production".to_owned()]),
                cmd: Some(vec!["serve".to_owned()]),
                entrypoint: Some(vec!["/entrypoint.sh".to_owned()]),
                working_dir: Some("/srv".to_owned()),
                user: Some("1000:1000".to_owned()),
                labels: Some(HashMap::from([(
                    "com.example.role".to_owned(),
                    "web".to_owned(),
                )])),
                exposed_ports: Some(vec!["8080/tcp".to_owned()]),
                ..Default::default()
            }),
            host_config: Some(HostConfig {
                binds: Some(vec!["app-data:/var/lib/app".to_owned()]),
                port_bindings: Some(HashMap::from([(
                    "8080/tcp".to_owned(),
                    Some(vec![PortBinding {
                        host_ip: Some("127.0.0.1".to_owned()),
                        host_port: Some("8080".to_owned()),
                    }]),
                )])),
                restart_policy: Some(RestartPolicy {
                    name: Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED),
                    maximum_retry_count: None,
                }),
                memory: Some(536_870_912),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn a_recreate_keeps_everything_except_the_image() {
        let original = inspected();
        let body = recreate_body(&original, "example/app:2.0");

        assert_eq!(body.image.as_deref(), Some("example/app:2.0"));

        let config = original.config.clone().unwrap();
        assert_eq!(body.env, config.env, "environment must survive");
        assert_eq!(body.cmd, config.cmd);
        assert_eq!(body.entrypoint, config.entrypoint);
        assert_eq!(body.working_dir, config.working_dir);
        assert_eq!(body.user, config.user);
        assert_eq!(body.labels, config.labels);
        assert_eq!(body.exposed_ports, config.exposed_ports);

        // The host config is what carries volumes, published ports, the restart
        // policy and the limits — lose it and the container comes back looking
        // right and behaving nothing like itself.
        let host = body.host_config.expect("host config must survive");
        assert_eq!(host.binds, original.host_config.clone().unwrap().binds);
        assert_eq!(
            host.port_bindings,
            original.host_config.clone().unwrap().port_bindings
        );
        assert_eq!(
            host.restart_policy.and_then(|policy| policy.name),
            Some(bollard::models::RestartPolicyNameEnum::UNLESS_STOPPED)
        );
        assert_eq!(host.memory, Some(536_870_912));
    }

    #[test]
    fn a_container_with_no_configuration_still_produces_a_usable_request() {
        // A daemon that answers an inspect with almost nothing must not make
        // the action panic; the replacement is then simply the image.
        let body = recreate_body(&ContainerInspectResponse::default(), "example/app:2.0");
        assert_eq!(body.image.as_deref(), Some("example/app:2.0"));
        assert_eq!(body.env, None);
        assert_eq!(body.host_config, None);
    }

    #[test]
    fn the_image_reference_comes_from_the_creation_config_not_the_resolved_id() {
        assert_eq!(
            configured_image_ref(&inspected()).as_deref(),
            Some("example/app:1.0")
        );

        // A blank or absent image is "cannot be checked", not an empty tag to
        // go and ask a registry about.
        let mut blank = inspected();
        blank.config.as_mut().unwrap().image = Some("   ".to_owned());
        assert_eq!(configured_image_ref(&blank), None);
        assert_eq!(
            configured_image_ref(&ContainerInspectResponse::default()),
            None
        );
    }

    #[test]
    fn a_reading_only_names_a_latest_reference_when_there_is_an_update() {
        let outdated = UpdateReading {
            current_ref: "example/app:1.0".to_owned(),
            available: true,
            latest_ref: Some("example/app@sha256:aaaa".to_owned()),
            checked_at: Utc::now(),
        };
        assert_eq!(
            outdated.as_result(),
            UpdateCheckResult::available("example/app@sha256:aaaa")
        );

        // Up to date: the digest that was read is still remembered locally for
        // the browser's "checked at" column, but the trait-level answer says
        // "nothing available" rather than handing a client a reference it might
        // offer to apply.
        let current = UpdateReading {
            available: false,
            ..outdated
        };
        assert_eq!(current.as_result(), UpdateCheckResult::up_to_date());
    }
}
