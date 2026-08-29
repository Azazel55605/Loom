//! The Docker host's own inventory, as browsable tables.
//!
//! Three kinds — images, volumes, networks — that are the same shape as each
//! other and as every other connector's: a list of rows described by columns,
//! with per-row and whole-kind operations attached. Nothing here is a feature
//! in its own right; it is three instances of
//! [`ResourceKindDescriptor`](loom_core::connector::ResourceKindDescriptor).
//!
//! # "Used by" is one container listing, not three
//!
//! Every one of the three tables wants to answer "is anything using this?", and
//! all three answers come from the same place: the container list. Asking
//! Docker once and building three reverse-lookup maps from it
//! ([`Usage::from_containers`]) costs one request per browse rather than one
//! per row, and the mapping itself is pure, so it is tested without a daemon.
//!
//! # Deleting is offered even when it will be refused
//!
//! A row's delete button is shown for every row, including a volume with a
//! container attached and Docker's built-in `bridge`, `host` and `none`
//! networks. Docker refuses those, and its refusal — "network bridge is a
//! pre-defined network and cannot be removed" — is passed through verbatim as
//! the action's message.
//!
//! This is a deliberate simplification, not an oversight. Hiding the button
//! would mean re-implementing Docker's removability rules here, in a second
//! place, from the outside: which networks are pre-defined, whether a stopped
//! container still counts as using a volume, whether an image is a parent of
//! another image. Every one of those is a rule the daemon already applies
//! authoritatively and can change between versions, and a copy of it here would
//! be wrong quietly, whereas passing the refusal through is wrong loudly and
//! only at the moment someone actually tried.

use std::collections::{BTreeMap, HashMap};

use bollard::models::{ContainerSummary, NetworkCreateRequest, VolumeCreateRequest};
use bollard::query_parameters::{
    CreateImageOptionsBuilder, ListContainersOptionsBuilder, ListImagesOptionsBuilder,
    ListNetworksOptions, ListVolumesOptions, PruneImagesOptionsBuilder, RemoveImageOptionsBuilder,
    RemoveVolumeOptions,
};
use bollard::Docker;
use chrono::DateTime;
use futures_util::StreamExt;
use loom_core::connector::{
    ActionResult, ApplicableTarget, ColumnDescriptor, ColumnValueType, ConnectorAction,
    ConnectorError, ResourceItem, ResourceKindDescriptor, StatusTone, StatusValue,
};
use serde_json::{json, Value};

use crate::registry::{current_digest, is_outdated, ImageReference, RegistryTransport};

/// Every image the daemon holds, one row per tag.
pub const RESOURCE_KIND_IMAGES: &str = "images";
/// Every volume the daemon holds.
pub const RESOURCE_KIND_VOLUMES: &str = "volumes";
/// Every network the daemon holds.
pub const RESOURCE_KIND_NETWORKS: &str = "networks";

/// Removes one image (or untags one tag of it).
pub const ACTION_DELETE_IMAGE: &str = "deleteImage";
/// Asks the registry whether one image's tag has moved on.
pub const ACTION_CHECK_IMAGE_UPDATE: &str = "checkImageUpdate";
/// Pulls a reference the user names.
pub const ACTION_PULL_IMAGE: &str = "pullImage";
/// Removes every image no container is using.
pub const ACTION_PRUNE_IMAGES: &str = "pruneImages";
/// Removes one volume.
pub const ACTION_DELETE_VOLUME: &str = "deleteVolume";
/// Creates one volume.
pub const ACTION_CREATE_VOLUME: &str = "createVolume";
/// Removes one network.
pub const ACTION_DELETE_NETWORK: &str = "deleteNetwork";
/// Creates one network.
pub const ACTION_CREATE_NETWORK: &str = "createNetwork";

/// The `params` key a row action names its row with, per the resource-browser
/// contract.
const RESOURCE_ID_PARAM: &str = "resourceId";

/// Docker's own placeholder for "this image has no repository or tag".
const UNTAGGED: &str = "<none>";

/// The one `MountPoint.Type` the volume table lists. bollard types this field
/// as a plain string rather than an enum, so the value is spelled out here.
const MOUNT_TYPE_VOLUME: &str = "volume";

/// Group-level fields the images table attaches to every row of a group.
///
/// Both are the *group's* answer repeated on each of its rows, which is what
/// `group_summary` expects: the client reads them from any row rather than
/// deriving them, because neither can be derived correctly from the rows alone.
const GROUP_USAGE_FIELD: &str = "groupUsage";
const GROUP_SIZE_FIELD: &str = "groupSize";

/// Scratch field carrying an image's content id while group totals are being
/// computed, removed before the rows are returned. Not a column: what a reader
/// wants is the short id, which `imageId` already shows.
const IMAGE_ID_FIELD: &str = "__imageId";

/// The driver a volume gets when the caller does not name one.
const DEFAULT_VOLUME_DRIVER: &str = "local";
/// The driver a network gets when the caller does not name one.
const DEFAULT_NETWORK_DRIVER: &str = "bridge";

/// Which containers are using each image, volume, and network.
///
/// Built from one container listing. The keys are the identifiers the
/// respective listings report: an image's content id, a volume's name, a
/// network's name.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Usage {
    by_image: BTreeMap<String, Vec<String>>,
    by_volume: BTreeMap<String, Vec<String>>,
    by_network: BTreeMap<String, Vec<String>>,
}

impl Usage {
    /// Builds the three reverse lookups from one container listing.
    ///
    /// Images are keyed by `ImageId` — the resolved content digest — and never
    /// by the reference the container was created from. A container created
    /// from `app:latest` keeps naming `app:latest` after the tag has moved,
    /// which would attribute it to whichever image holds that tag *now* rather
    /// than to the one it is actually running. The id is what it is running.
    pub fn from_containers(containers: Vec<ContainerSummary>) -> Self {
        let mut usage = Self::default();
        for container in containers {
            let Some(name) = container_name(&container) else {
                continue;
            };

            if let Some(image_id) = container.image_id.filter(|id| !id.is_empty()) {
                usage
                    .by_image
                    .entry(image_id)
                    .or_default()
                    .push(name.clone());
            }
            for mount in container.mounts.unwrap_or_default() {
                // A bind mount has a source path and no name; only a *volume*
                // mount refers to something the volume table lists.
                if mount.typ.as_deref() != Some(MOUNT_TYPE_VOLUME) {
                    continue;
                }
                if let Some(volume) = mount.name.filter(|value| !value.is_empty()) {
                    usage
                        .by_volume
                        .entry(volume)
                        .or_default()
                        .push(name.clone());
                }
            }
            let networks = container
                .network_settings
                .and_then(|settings| settings.networks)
                .unwrap_or_default();
            for network in networks.into_keys() {
                usage
                    .by_network
                    .entry(network)
                    .or_default()
                    .push(name.clone());
            }
        }

        // Sorted so a cell does not reshuffle its names between refreshes, and
        // deduplicated because a container can be attached to the same network
        // or volume more than once.
        for names in usage
            .by_image
            .values_mut()
            .chain(usage.by_volume.values_mut())
            .chain(usage.by_network.values_mut())
        {
            names.sort();
            names.dedup();
        }
        usage
    }

    /// The containers running one image id, as one cell.
    pub fn for_image(&self, image_id: &str) -> String {
        join(self.by_image.get(image_id))
    }

    /// The containers mounting one volume, as one cell.
    pub fn for_volume(&self, volume: &str) -> String {
        join(self.by_volume.get(volume))
    }

    /// The containers attached to one network, as one cell.
    pub fn for_network(&self, network: &str) -> String {
        join(self.by_network.get(network))
    }
}

fn join(names: Option<&Vec<String>>) -> String {
    names.map(|names| names.join(", ")).unwrap_or_default()
}

/// A container's display name, the same one [`crate::connector`] uses as a
/// sub-target id.
fn container_name(container: &ContainerSummary) -> Option<String> {
    container
        .names
        .as_ref()
        .and_then(|names| names.first())
        .map(|name| name.trim_start_matches('/').to_owned())
        .filter(|name| !name.is_empty())
        .or_else(|| {
            container
                .id
                .as_ref()
                .map(|id| id.chars().take(12).collect::<String>())
        })
        .filter(|name| !name.is_empty())
}

/// Reads the container list once, for the "used by" columns.
///
/// A failure here is not a failure of the browse: a table of images with an
/// empty "used by" column is still the table someone asked for, and refusing to
/// show it because a second request failed would be the worse answer.
pub async fn usage(docker: &Docker) -> Usage {
    let options = ListContainersOptionsBuilder::new().all(true).build();
    match docker.list_containers(Some(options)).await {
        Ok(containers) => Usage::from_containers(containers),
        Err(_) => Usage::default(),
    }
}

/* ------------------------------------------------------------------ */
/* Descriptors                                                         */
/* ------------------------------------------------------------------ */

/// The three host-inventory kinds.
///
/// All three are [`ApplicableTarget::HostOnly`]: images, volumes and networks
/// belong to the daemon, and "the images of one container" is not a smaller
/// version of the question — it is a different one, with no answer.
pub fn resource_kinds() -> Vec<ResourceKindDescriptor> {
    vec![images_kind(), volumes_kind(), networks_kind()]
}

fn images_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_IMAGES,
        "Images",
        vec![
            ColumnDescriptor::new("repository", "Repository", ColumnValueType::Text),
            ColumnDescriptor::new("tag", "Tag", ColumnValueType::Text),
            ColumnDescriptor::new("imageId", "Image ID", ColumnValueType::Text),
            ColumnDescriptor::new("size", "Size", ColumnValueType::Bytes),
            ColumnDescriptor::new("created", "Created", ColumnValueType::Timestamp),
            ColumnDescriptor::new("usedBy", "Used by", ColumnValueType::Text),
            ColumnDescriptor::new("usage", "Usage", ColumnValueType::Status),
        ],
    )
    // One repository with six tags is six rows that belong together; a flat
    // list of forty images is the thing `docker images` is hard to read for.
    .grouped_by("repository")
    // What a collapsed heading has to say to be worth collapsing: whether
    // there is anything to clean up in there, and how much disk it would give
    // back. Both are computed here rather than summed by the client — see
    // `ResourceKindDescriptor::group_summary` for why summing is wrong.
    .with_group_summary(vec![
        ColumnDescriptor::new(GROUP_USAGE_FIELD, "Usage", ColumnValueType::Status),
        ColumnDescriptor::new(GROUP_SIZE_FIELD, "Combined size", ColumnValueType::Bytes),
    ])
    .applicable_to(ApplicableTarget::HostOnly)
    .with_row_actions(vec![
        row_action(
            ACTION_DELETE_IMAGE,
            "Delete",
            "Remove this image. Docker refuses while a container is using it, and \
             says so.",
            true,
        ),
        row_action(
            ACTION_CHECK_IMAGE_UPDATE,
            "Check for update",
            "Ask this image's registry whether its tag now points somewhere newer. \
             Downloads nothing.",
            false,
        ),
    ])
    .with_kind_actions(vec![
        ConnectorAction {
            id: ACTION_PULL_IMAGE.to_owned(),
            target_id: None,
            label: "Pull image".to_owned(),
            description: Some(
                "Download an image by reference, for example `nginx:1.27`.".to_owned(),
            ),
            params_schema: json!({
                "type": "object",
                "properties": {
                    "imageRef": {
                        "type": "string",
                        "title": "Image reference",
                        "description": "Repository and tag, for example `nginx:1.27`. \
                                        Without a tag, `latest` is pulled."
                    }
                },
                "required": ["imageRef"],
                "additionalProperties": false
            }),
            is_disruptive: false,
            snapshot_data_point_ids: Vec::new(),
        },
        ConnectorAction {
            id: ACTION_PRUNE_IMAGES.to_owned(),
            target_id: None,
            label: "Prune unused".to_owned(),
            description: Some(
                "Remove every image no container is using, and report the disk space that \
             freed. Images a stopped container was created from count as in use and are \
             kept."
                    .to_owned(),
            ),
            params_schema: json!({ "type": "object", "additionalProperties": false }),
            // Nothing running stops, but images go away and cannot be brought back
            // without pulling them again — which needs the registry to still have
            // them, and to be reachable.
            is_disruptive: true,
            snapshot_data_point_ids: Vec::new(),
        },
    ])
}

fn volumes_kind() -> ResourceKindDescriptor {
    // Volume *size* is deliberately not a column. Docker only knows it from the
    // `/system/df` endpoint, which walks every volume's directory tree and can
    // take tens of seconds on a host with a large database volume — far too
    // expensive to pay on every browse, and the connector already caches that
    // endpoint for the host's disk-usage data point at a much slower cadence
    // for exactly that reason. If per-volume size turns out to be something
    // people actually want here, the honest way to add it is from that cached
    // reading, with its age shown, rather than by making this listing slow.
    ResourceKindDescriptor::new(
        RESOURCE_KIND_VOLUMES,
        "Volumes",
        vec![
            ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
            ColumnDescriptor::new("driver", "Driver", ColumnValueType::Text),
            ColumnDescriptor::new("mountpoint", "Mount point", ColumnValueType::Text),
            ColumnDescriptor::new("created", "Created", ColumnValueType::Timestamp),
            ColumnDescriptor::new("usedBy", "Used by", ColumnValueType::Text),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    .with_row_actions(vec![row_action(
        ACTION_DELETE_VOLUME,
        "Delete",
        "Remove this volume and the data in it. Docker refuses while a container \
         is using it, and says so.",
        true,
    )])
    .with_kind_actions(vec![ConnectorAction {
        id: ACTION_CREATE_VOLUME.to_owned(),
        target_id: None,
        label: "Create volume".to_owned(),
        description: Some("Add an empty volume for containers to mount.".to_owned()),
        params_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "title": "Name",
                    "description": "What containers will refer to this volume by."
                },
                "driver": {
                    "type": "string",
                    "title": "Driver",
                    "description": "Volume driver. Left empty, Docker's built-in \
                                    `local` driver is used."
                }
            },
            "required": ["name"],
            "additionalProperties": false
        }),
        is_disruptive: false,
        snapshot_data_point_ids: Vec::new(),
    }])
}

fn networks_kind() -> ResourceKindDescriptor {
    ResourceKindDescriptor::new(
        RESOURCE_KIND_NETWORKS,
        "Networks",
        vec![
            ColumnDescriptor::new("name", "Name", ColumnValueType::Text),
            ColumnDescriptor::new("driver", "Driver", ColumnValueType::Text),
            ColumnDescriptor::new("scope", "Scope", ColumnValueType::Text),
            ColumnDescriptor::new("subnet", "Subnet", ColumnValueType::Text),
            ColumnDescriptor::new("created", "Created", ColumnValueType::Timestamp),
            ColumnDescriptor::new("usedBy", "Used by", ColumnValueType::Text),
        ],
    )
    .applicable_to(ApplicableTarget::HostOnly)
    .with_row_actions(vec![row_action(
        ACTION_DELETE_NETWORK,
        "Delete",
        "Remove this network. Docker refuses for a network in use and for its own \
         built-in networks, and says so.",
        true,
    )])
    .with_kind_actions(vec![ConnectorAction {
        id: ACTION_CREATE_NETWORK.to_owned(),
        target_id: None,
        label: "Create network".to_owned(),
        description: Some("Add a network for containers to attach to.".to_owned()),
        params_schema: json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "title": "Name",
                    "description": "What containers will refer to this network by."
                },
                "driver": {
                    "type": "string",
                    "title": "Driver",
                    "description": "Network driver. Left empty, Docker's `bridge` \
                                    driver is used."
                }
            },
            "required": ["name"],
            "additionalProperties": false
        }),
        is_disruptive: false,
        snapshot_data_point_ids: Vec::new(),
    }])
}

/// One row-scoped action, which by the contract takes exactly `resourceId`.
fn row_action(id: &str, label: &str, description: &str, is_disruptive: bool) -> ConnectorAction {
    ConnectorAction {
        id: id.to_owned(),
        target_id: None,
        label: label.to_owned(),
        description: Some(description.to_owned()),
        // Declared rather than left implicit, as the resource-browser contract
        // asks: a client can see what the action needs instead of learning it
        // from prose. The browser fills it from the row, so nobody types it.
        params_schema: json!({
            "type": "object",
            "properties": {
                RESOURCE_ID_PARAM: {
                    "type": "string",
                    "title": "Resource",
                    "description": "The row this action applies to."
                }
            },
            "required": [RESOURCE_ID_PARAM],
            "additionalProperties": false
        }),
        is_disruptive,
        snapshot_data_point_ids: Vec::new(),
    }
}

/* ------------------------------------------------------------------ */
/* Rows                                                                */
/* ------------------------------------------------------------------ */

/// The image table: one row per tag, plus one row for each untagged image.
///
/// A single image carrying three tags is three rows, because a tag is what a
/// person deletes, pulls and checks — the image behind them is shared, which is
/// what the repeated image id and size say.
pub async fn list_images(
    docker: &Docker,
    usage: &Usage,
) -> Result<Vec<ResourceItem>, ConnectorError> {
    let options = ListImagesOptionsBuilder::new().all(false).build();
    let images = docker
        .list_images(Some(options))
        .await
        .map_err(|error| ConnectorError::unreachable(format!("listing images failed: {error}")))?;

    let mut rows = Vec::new();
    for image in images {
        let used_by = usage.for_image(&image.id);
        let in_use = !used_by.is_empty();
        let short = short_id(&image.id);
        let tags: Vec<String> = image
            .repo_tags
            .into_iter()
            .filter(|tag| !tag.is_empty() && !tag.starts_with(UNTAGGED))
            .collect();

        // One `ResourceItem` per tag, but the *image* — its id and its size —
        // is shared between them, which is what the group totals have to know.
        let row = |reference: String, repository: String, tag: String| {
            ResourceItem::new(reference)
                .with_field("repository", repository)
                .with_field("tag", tag)
                .with_field("imageId", short.clone())
                .with_field("size", image.size)
                .with_field("created", iso_from_unix(image.created))
                .with_field("usedBy", used_by.clone())
                .with_field("usage", row_usage(in_use))
                // Carried alongside the row so the group total can be computed
                // below without a second pass over the daemon's listing.
                .with_field(IMAGE_ID_FIELD, image.id.clone())
        };

        if tags.is_empty() {
            // A dangling image: no reference to act on, so the row is keyed by
            // the id, which is also the only thing `deleteImage` can be given.
            rows.push(row(
                image.id.clone(),
                UNTAGGED.to_owned(),
                UNTAGGED.to_owned(),
            ));
            continue;
        }

        for reference in tags {
            let (repository, tag) = split_reference(&reference);
            rows.push(row(reference.clone(), repository, tag));
        }
    }

    // Sorted by the grouping key first, so the groups a client builds are
    // contiguous without it having to sort them itself.
    //
    // Untagged images sort *last*, ahead of the plain alphabetical order they
    // would otherwise take (`<` precedes every letter). On a real host they are
    // the largest group and the least interesting one — a homelab that rebuilds
    // its own images accumulates hundreds — and leading with three hundred rows
    // that all read `<none>` buries every image somebody could actually name.
    rows.sort_by(|left, right| {
        untagged_last(left)
            .cmp(&untagged_last(right))
            .then_with(|| field(left, "repository").cmp(field(right, "repository")))
            .then_with(|| field(left, "tag").cmp(field(right, "tag")))
    });
    attach_group_summary(&mut rows);
    Ok(rows)
}

/// One image's own verdict.
fn row_usage(in_use: bool) -> StatusValue {
    if in_use {
        StatusValue::new("In use", StatusTone::Positive)
    } else {
        // Caution, not Negative: an unused image is not a fault, it is disk
        // somebody may want back. Colouring it as an error would make a normal
        // Docker host look broken.
        StatusValue::new("Unused", StatusTone::Caution)
    }
}

/// Fills in each row's group-level fields, in place.
///
/// Two things a client cannot work out for itself:
///
/// - **Combined size** is the sum over *distinct images*, not over rows. Three
///   tags of one 2 GB image are three 2 GB rows; adding them gives 6 GB of disk
///   that does not exist.
/// - **The group's verdict** distinguishes "none of these is used", "some are"
///   and "all are", which needs the same distinct-image view: two tags of one
///   used image are not two used images.
///
/// # Why "combined" and not "total"
///
/// Docker's per-image `Size` counts every layer the image is built from, and
/// images share layers — that is the whole point of layers. Summing even
/// distinct images therefore counts shared layers once per image, so the result
/// is an **upper bound on disk, not disk**. On a real host the gap is large:
/// a machine reporting 102 GiB of Docker disk in total showed 297 GB across its
/// untagged images alone.
///
/// The exact figure exists — `Size - SharedSize` — but `SharedSize` is only
/// computed when a listing asks for it, and that computation is the expensive
/// layer walk `/system/df` does. This connector already treats that endpoint as
/// too costly to call at poll cadence; paying it on every table refresh instead
/// would be worse. So the cheap upper bound is reported under a name that does
/// not promise otherwise, and `pruneImages` reports the daemon's own count of
/// what was actually reclaimed.
fn attach_group_summary(rows: &mut [ResourceItem]) {
    let mut totals: HashMap<String, GroupTotals> = HashMap::new();
    for item in rows.iter() {
        let entry = totals
            .entry(field(item, "repository").to_owned())
            .or_default();
        let image_id = field(item, IMAGE_ID_FIELD).to_owned();
        if entry.seen.insert(image_id) {
            entry.bytes += item
                .fields
                .get("size")
                .and_then(Value::as_i64)
                .unwrap_or_default();
            if field(item, "usedBy").is_empty() {
                entry.unused += 1;
            } else {
                entry.used += 1;
            }
        }
    }

    for item in rows.iter_mut() {
        let Some(totals) = totals.get(field(item, "repository")) else {
            continue;
        };
        let usage = match (totals.used, totals.unused) {
            (0, _) => StatusValue::new("Unused", StatusTone::Caution),
            (_, 0) => StatusValue::new("In use", StatusTone::Positive),
            _ => StatusValue::new("Some unused", StatusTone::Caution),
        };
        item.fields
            .insert(GROUP_USAGE_FIELD.to_owned(), usage.into());
        item.fields
            .insert(GROUP_SIZE_FIELD.to_owned(), totals.bytes.into());
        // Only ever needed to compute the two fields above.
        item.fields.remove(IMAGE_ID_FIELD);
    }
}

/// One repository's distinct images, while they are being counted.
#[derive(Default)]
struct GroupTotals {
    seen: std::collections::HashSet<String>,
    bytes: i64,
    used: usize,
    unused: usize,
}

/// The volume table.
pub async fn list_volumes(
    docker: &Docker,
    usage: &Usage,
) -> Result<Vec<ResourceItem>, ConnectorError> {
    let response = docker
        .list_volumes(None::<ListVolumesOptions>)
        .await
        .map_err(|error| ConnectorError::unreachable(format!("listing volumes failed: {error}")))?;

    let mut rows: Vec<ResourceItem> = response
        .volumes
        .unwrap_or_default()
        .into_iter()
        .map(|volume| {
            let used_by = usage.for_volume(&volume.name);
            ResourceItem::new(volume.name.clone())
                .with_field("name", volume.name)
                .with_field("driver", volume.driver)
                .with_field("mountpoint", volume.mountpoint)
                .with_field("created", volume.created_at.unwrap_or_default())
                .with_field("usedBy", used_by)
        })
        .collect();
    rows.sort_by(|left, right| field(left, "name").cmp(field(right, "name")));
    Ok(rows)
}

/// The network table.
pub async fn list_networks(
    docker: &Docker,
    usage: &Usage,
) -> Result<Vec<ResourceItem>, ConnectorError> {
    let networks = docker
        .list_networks(None::<ListNetworksOptions>)
        .await
        .map_err(|error| {
            ConnectorError::unreachable(format!("listing networks failed: {error}"))
        })?;

    let mut rows: Vec<ResourceItem> = networks
        .into_iter()
        .map(|network| {
            let name = network.name.unwrap_or_default();
            let used_by = usage.for_network(&name);
            // Keyed by id, not by name: `docker network rm` accepts either, and
            // the id is the one Docker guarantees is unambiguous.
            let id = network.id.clone().unwrap_or_else(|| name.clone());
            ResourceItem::new(id)
                .with_field("name", name)
                .with_field("driver", network.driver.unwrap_or_default())
                .with_field("scope", network.scope.unwrap_or_default())
                .with_field("subnet", subnets(network.ipam.as_ref()))
                .with_field("created", network.created.unwrap_or_default())
                .with_field("usedBy", used_by)
        })
        .collect();
    rows.sort_by(|left, right| field(left, "name").cmp(field(right, "name")));
    Ok(rows)
}

/// Every subnet a network's IPAM declares, best effort.
///
/// A network can have none — `host` and `none` do, and an overlay network whose
/// IPAM is managed elsewhere can too — and can have more than one, once IPv6 is
/// enabled. Both are ordinary, so this returns an empty string rather than a
/// placeholder and joins rather than picking the first.
pub fn subnets(ipam: Option<&bollard::models::Ipam>) -> String {
    ipam.and_then(|ipam| ipam.config.as_ref())
        .map(|configs| {
            configs
                .iter()
                .filter_map(|config| config.subnet.clone())
                .filter(|subnet| !subnet.is_empty())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

/// Splits `repository:tag` the way the image table's two columns need it.
///
/// Only a colon *after* the last slash separates a tag; one before it is a
/// registry port, which is why `registry.example.com:5000/app` is one
/// repository with the implicit `latest` tag and not a repository called
/// `registry.example.com` on port `5000/app`.
pub fn split_reference(reference: &str) -> (String, String) {
    match reference.rsplit_once(':') {
        Some((repository, tag)) if !tag.contains('/') && !tag.is_empty() => {
            (repository.to_owned(), tag.to_owned())
        }
        _ => (reference.to_owned(), "latest".to_owned()),
    }
}

/// Docker's own display form for an image id: the algorithm dropped, twelve
/// hex characters kept.
fn short_id(id: &str) -> String {
    id.split_once(':')
        .map_or(id, |(_, digest)| digest)
        .chars()
        .take(12)
        .collect()
}

/// A Unix timestamp as the ISO 8601 string a `Timestamp` column carries.
fn iso_from_unix(seconds: i64) -> String {
    DateTime::from_timestamp(seconds, 0)
        .map(|instant| instant.to_rfc3339())
        .unwrap_or_default()
}

/// Sort key putting Docker's untagged placeholder after every real repository.
fn untagged_last(item: &ResourceItem) -> u8 {
    u8::from(field(item, "repository") == UNTAGGED)
}

fn field<'a>(item: &'a ResourceItem, key: &str) -> &'a str {
    item.fields.get(key).and_then(Value::as_str).unwrap_or("")
}

/* ------------------------------------------------------------------ */
/* Actions                                                             */
/* ------------------------------------------------------------------ */

/// Whether an action id belongs to this module.
pub fn owns_action(action_id: &str) -> bool {
    matches!(
        action_id,
        ACTION_DELETE_IMAGE
            | ACTION_CHECK_IMAGE_UPDATE
            | ACTION_PULL_IMAGE
            | ACTION_PRUNE_IMAGES
            | ACTION_DELETE_VOLUME
            | ACTION_CREATE_VOLUME
            | ACTION_DELETE_NETWORK
            | ACTION_CREATE_NETWORK
    )
}

/// Runs one of this module's actions.
///
/// `registry` is only needed by [`ACTION_CHECK_IMAGE_UPDATE`]; the connector
/// has none when an HTTPS client could not be built, which makes exactly that
/// one action unavailable rather than the whole table.
pub async fn execute(
    docker: &Docker,
    registry: Option<&dyn RegistryTransport>,
    action_id: &str,
    params: &Value,
) -> Result<ActionResult, ConnectorError> {
    match action_id {
        ACTION_DELETE_IMAGE => {
            delete_image(docker, required(action_id, params, RESOURCE_ID_PARAM)?).await
        }
        ACTION_CHECK_IMAGE_UPDATE => {
            let registry = registry.ok_or_else(|| {
                ConnectorError::Internal(
                    "no HTTPS client is available, so registries cannot be queried".to_owned(),
                )
            })?;
            check_image(
                docker,
                registry,
                required(action_id, params, RESOURCE_ID_PARAM)?,
            )
            .await
        }
        ACTION_PULL_IMAGE => pull_image(docker, required(action_id, params, "imageRef")?).await,
        ACTION_PRUNE_IMAGES => prune_images(docker).await,
        ACTION_DELETE_VOLUME => {
            delete_volume(docker, required(action_id, params, RESOURCE_ID_PARAM)?).await
        }
        ACTION_CREATE_VOLUME => {
            create_volume(
                docker,
                required(action_id, params, "name")?,
                optional(params, "driver").unwrap_or(DEFAULT_VOLUME_DRIVER),
            )
            .await
        }
        ACTION_DELETE_NETWORK => {
            delete_network(docker, required(action_id, params, RESOURCE_ID_PARAM)?).await
        }
        ACTION_CREATE_NETWORK => {
            create_network(
                docker,
                required(action_id, params, "name")?,
                optional(params, "driver").unwrap_or(DEFAULT_NETWORK_DRIVER),
            )
            .await
        }
        other => Err(ConnectorError::invalid_action(other)),
    }
}

/// A required string parameter, or the error that names it.
fn required<'a>(action_id: &str, params: &'a Value, key: &str) -> Result<&'a str, ConnectorError> {
    optional(params, key).ok_or_else(|| ConnectorError::InvalidParams {
        action_id: action_id.to_owned(),
        reason: format!("expected a non-empty string `{key}`"),
    })
}

fn optional<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params
        .get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

async fn delete_image(docker: &Docker, reference: &str) -> Result<ActionResult, ConnectorError> {
    // No `force`. Docker refuses to remove an image a container was created
    // from, and forcing past that leaves the container running something with
    // no name — a state that is far harder to explain than the refusal.
    let options = RemoveImageOptionsBuilder::new().build();
    match docker.remove_image(reference, Some(options), None).await {
        Ok(deleted) => Ok(ActionResult::ok(format!(
            "Removed {reference} ({} layer or tag reference(s) deleted).",
            deleted.len()
        ))),
        Err(error) => refusal_or(error, "remove", reference),
    }
}

async fn check_image(
    docker: &Docker,
    registry: &dyn RegistryTransport,
    reference: &str,
) -> Result<ActionResult, ConnectorError> {
    let Some(parsed) = ImageReference::parse(reference) else {
        // A dangling image row, or one pinned to a digest. Neither can move.
        return Ok(ActionResult::ok(format!(
            "{reference} names one exact image rather than a tag, so there is nothing to check."
        )));
    };

    let local = docker
        .inspect_image(reference)
        .await
        .map_err(|error| {
            ConnectorError::unreachable(format!("could not inspect {reference}: {error}"))
        })?
        .repo_digests
        .unwrap_or_default();
    if local.is_empty() {
        return Ok(ActionResult::ok(format!(
            "{reference} was built here rather than pulled, so there is no registry version of \
             it to be behind."
        )));
    }

    let digest = current_digest(registry, &parsed).await?;
    Ok(if is_outdated(&local, &digest) {
        ActionResult::ok(format!(
            "Update available: {} now points at {digest}.",
            parsed.repository
        ))
        .with_payload(
            json!({ "available": true, "latestRef": format!("{}@{digest}", parsed.repository) }),
        )
    } else {
        ActionResult::ok(format!(
            "Up to date: {reference} is what {} is serving.",
            parsed.registry
        ))
        .with_payload(json!({ "available": false }))
    })
}

async fn pull_image(docker: &Docker, reference: &str) -> Result<ActionResult, ConnectorError> {
    let (repository, tag) = split_reference(reference);
    let options = CreateImageOptionsBuilder::new()
        .from_image(&repository)
        .tag(&tag)
        .build();

    let mut stream = docker.create_image(Some(options), None, None);
    while let Some(progress) = stream.next().await {
        match progress {
            // The stream reports pull failures *inside* successful frames, so a
            // `Ok` that carries an `error_detail` is a failed pull, not a
            // successful one — see `updates::apply_update`, which learned the
            // same lesson.
            Ok(frame) => {
                if let Some(detail) = frame.error_detail.and_then(|detail| detail.message) {
                    return Ok(ActionResult::failed(format!(
                        "Docker could not pull {repository}:{tag}: {}",
                        detail.trim()
                    )));
                }
            }
            Err(error) => return refusal_or(error, "pull", &format!("{repository}:{tag}")),
        }
    }
    Ok(ActionResult::ok(format!("Pulled {repository}:{tag}.")))
}

/// Removes every image nothing is using, and says what that gave back.
///
/// `dangling=false` rather than the default. Docker's *default* prune removes
/// only images with no tag at all, which on a homelab host is a handful of
/// build leftovers; the button says "prune unused" and the table's own pills
/// say which images are unused, so removing exactly those is the only
/// behaviour that matches what the user is looking at.
async fn prune_images(docker: &Docker) -> Result<ActionResult, ConnectorError> {
    let options = PruneImagesOptionsBuilder::new()
        .filters(&prune_filters())
        .build();
    match docker.prune_images(Some(options)).await {
        Ok(response) => {
            let removed = response.images_deleted.unwrap_or_default().len();
            let reclaimed = response.space_reclaimed.unwrap_or_default();
            Ok(ActionResult::ok(if removed == 0 {
                "Nothing to prune — every image here is in use.".to_owned()
            } else {
                format!(
                    "Pruned {removed} unused image layer(s), reclaiming {}.",
                    human_bytes(reclaimed)
                )
            })
            // The raw count for anything that wants to do arithmetic; the
            // message is for reading, and the two must not be the same number
            // in two formats with only one of them authoritative.
            .with_payload(json!({ "removed": removed, "spaceReclaimedBytes": reclaimed })))
        }
        Err(error) => refusal_or(error, "prune", "unused images"),
    }
}

/// The filter that makes prune mean what the button says.
///
/// Docker's *default* image prune removes only images with no tag at all —
/// `dangling=true`. `dangling=false` is the documented spelling of "every image
/// no container is using", which is exactly the set the table's own `Unused`
/// pills mark. A button whose effect did not match the pills beside it would be
/// the worst kind of destructive control: one you cannot predict by looking.
fn prune_filters() -> HashMap<&'static str, Vec<&'static str>> {
    HashMap::from([("dangling", vec!["false"])])
}

/// A byte count as a person would say it.
///
/// Only for the human half of an `ActionResult`; every number Loom *reports* —
/// a data point, a `Bytes` column, this action's payload — stays a raw count,
/// so the client formats it in the viewer's own locale. This exists because a
/// message is prose, and "reclaiming 3892314112" is not a sentence.
fn human_bytes(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value.abs() >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

async fn delete_volume(docker: &Docker, name: &str) -> Result<ActionResult, ConnectorError> {
    // Again no `force`: forcing a volume out from under a running container
    // destroys data that container is still writing to.
    match docker
        .remove_volume(name, None::<RemoveVolumeOptions>)
        .await
    {
        Ok(()) => Ok(ActionResult::ok(format!("Removed volume {name}."))),
        Err(error) => refusal_or(error, "remove", name),
    }
}

async fn create_volume(
    docker: &Docker,
    name: &str,
    driver: &str,
) -> Result<ActionResult, ConnectorError> {
    let request = VolumeCreateRequest {
        name: Some(name.to_owned()),
        driver: Some(driver.to_owned()),
        ..Default::default()
    };
    match docker.create_volume(request).await {
        Ok(volume) => Ok(ActionResult::ok(format!(
            "Created volume {} at {}.",
            volume.name, volume.mountpoint
        ))),
        Err(error) => refusal_or(error, "create", name),
    }
}

async fn delete_network(docker: &Docker, id: &str) -> Result<ActionResult, ConnectorError> {
    match docker.remove_network(id).await {
        Ok(()) => Ok(ActionResult::ok(format!("Removed network {id}."))),
        Err(error) => refusal_or(error, "remove", id),
    }
}

async fn create_network(
    docker: &Docker,
    name: &str,
    driver: &str,
) -> Result<ActionResult, ConnectorError> {
    let request = NetworkCreateRequest {
        name: name.to_owned(),
        driver: Some(driver.to_owned()),
        ..Default::default()
    };
    match docker.create_network(request).await {
        Ok(response) => Ok(ActionResult::ok(format!(
            "Created network {name} ({}).",
            response.id
        ))),
        Err(error) => refusal_or(error, "create", name),
    }
}

/// Splits a bollard error the way the trait draws the line.
///
/// A `DockerResponseServerError` is the daemon *answering* and declining — "the
/// volume is in use", "bridge is a pre-defined network" — which is
/// `success: false` carrying Docker's own words. Loom does not paraphrase them:
/// the daemon knows why it refused and Loom is guessing. Anything else never
/// got an answer, and is a transport error.
fn refusal_or(
    error: bollard::errors::Error,
    verb: &str,
    subject: &str,
) -> Result<ActionResult, ConnectorError> {
    match error {
        bollard::errors::Error::DockerResponseServerError {
            status_code,
            message,
        } => Ok(ActionResult::failed(format!(
            "Docker refused to {verb} {subject} ({status_code}): {}",
            message.trim()
        ))
        .with_payload(json!({ "statusCode": status_code }))),
        other => Err(ConnectorError::unreachable(format!(
            "could not {verb} {subject}: {other}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bollard::models::{ContainerSummaryNetworkSettings, EndpointSettings, MountPoint};
    use bollard::query_parameters::PruneImagesOptionsBuilder;
    use std::collections::HashMap;

    fn container(
        name: &str,
        image_id: &str,
        volumes: &[&str],
        networks: &[&str],
    ) -> ContainerSummary {
        ContainerSummary {
            id: Some(format!("id-of-{name}")),
            names: Some(vec![format!("/{name}")]),
            image_id: Some(image_id.to_owned()),
            mounts: Some(
                volumes
                    .iter()
                    .map(|volume| MountPoint {
                        typ: Some(MOUNT_TYPE_VOLUME.to_owned()),
                        name: Some((*volume).to_owned()),
                        ..Default::default()
                    })
                    .collect(),
            ),
            network_settings: Some(ContainerSummaryNetworkSettings {
                networks: Some(
                    networks
                        .iter()
                        .map(|network| ((*network).to_owned(), EndpointSettings::default()))
                        .collect::<HashMap<_, _>>(),
                ),
            }),
            ..Default::default()
        }
    }

    #[test]
    fn usage_cross_references_containers_against_all_three_inventories() {
        let usage = Usage::from_containers(vec![
            container("web", "sha256:aaa", &["site-data"], &["frontend"]),
            container("api", "sha256:aaa", &["api-data"], &["frontend", "backend"]),
            container("db", "sha256:bbb", &["api-data"], &["backend"]),
        ]);

        // Two containers share an image; both are named, sorted.
        assert_eq!(usage.for_image("sha256:aaa"), "api, web");
        assert_eq!(usage.for_image("sha256:bbb"), "db");
        // An inventory entry nothing uses reads as empty, not as an error.
        assert_eq!(usage.for_image("sha256:ccc"), "");

        assert_eq!(usage.for_volume("api-data"), "api, db");
        assert_eq!(usage.for_volume("site-data"), "web");
        assert_eq!(usage.for_volume("unused"), "");

        assert_eq!(usage.for_network("frontend"), "api, web");
        assert_eq!(usage.for_network("backend"), "api, db");
        assert_eq!(usage.for_network("none"), "");
    }

    #[test]
    fn a_bind_mount_is_not_a_volume() {
        let mut summary = container("web", "sha256:aaa", &[], &[]);
        summary.mounts = Some(vec![MountPoint {
            typ: Some("bind".to_owned()),
            // A bind mount can carry a name-shaped source; it still is not a
            // row in the volume table, and attributing it to one would report
            // a volume as in use that nothing is using.
            name: Some("site-data".to_owned()),
            ..Default::default()
        }]);

        assert_eq!(
            Usage::from_containers(vec![summary]).for_volume("site-data"),
            ""
        );
    }

    #[test]
    fn a_container_is_listed_once_per_thing_it_uses() {
        // The same network twice — legal, and it must not double the name.
        let mut summary = container("web", "sha256:aaa", &["data", "data"], &[]);
        summary.network_settings = Some(ContainerSummaryNetworkSettings {
            networks: Some(HashMap::from([(
                "frontend".to_owned(),
                EndpointSettings::default(),
            )])),
        });
        let usage = Usage::from_containers(vec![summary]);
        assert_eq!(usage.for_volume("data"), "web");
        assert_eq!(usage.for_network("frontend"), "web");
    }

    #[test]
    fn references_split_into_the_two_columns_the_table_shows() {
        assert_eq!(
            split_reference("nginx:1.27"),
            ("nginx".into(), "1.27".into())
        );
        assert_eq!(split_reference("nginx"), ("nginx".into(), "latest".into()));
        assert_eq!(
            split_reference("owner/app:v2"),
            ("owner/app".into(), "v2".into())
        );
        // A registry port is not a tag.
        assert_eq!(
            split_reference("registry.example.com:5000/app"),
            ("registry.example.com:5000/app".into(), "latest".into())
        );
        assert_eq!(
            split_reference("registry.example.com:5000/app:v1"),
            ("registry.example.com:5000/app".into(), "v1".into())
        );
    }

    #[test]
    fn subnets_are_best_effort_and_never_a_placeholder() {
        use bollard::models::{Ipam, IpamConfig};

        assert_eq!(subnets(None), "");
        assert_eq!(subnets(Some(&Ipam::default())), "");
        assert_eq!(
            subnets(Some(&Ipam {
                config: Some(vec![
                    IpamConfig {
                        subnet: Some("192.0.2.0/24".to_owned()),
                        ..Default::default()
                    },
                    IpamConfig {
                        subnet: Some("2001:db8::/64".to_owned()),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            })),
            "192.0.2.0/24, 2001:db8::/64"
        );
    }

    /// Builds the rows one repository's images would produce, then summarizes.
    fn summarized(rows: Vec<ResourceItem>) -> Vec<ResourceItem> {
        let mut rows = rows;
        attach_group_summary(&mut rows);
        rows
    }

    fn image_row(repository: &str, tag: &str, id: &str, size: i64, used_by: &str) -> ResourceItem {
        ResourceItem::new(format!("{repository}:{tag}"))
            .with_field("repository", repository)
            .with_field("tag", tag)
            .with_field("size", size)
            .with_field("usedBy", used_by)
            .with_field("usage", row_usage(!used_by.is_empty()))
            .with_field(IMAGE_ID_FIELD, id)
    }

    #[test]
    fn a_group_total_counts_images_not_rows() {
        // One 2 GB image carrying three tags. Summing the rows — the obvious
        // client-side implementation — would report 6 GB of disk that is not
        // there, which is why this is the connector's job.
        let rows = summarized(vec![
            image_row("nginx", "1.27", "sha256:aaa", 2_000_000_000, ""),
            image_row("nginx", "latest", "sha256:aaa", 2_000_000_000, ""),
            image_row("nginx", "stable", "sha256:aaa", 2_000_000_000, ""),
        ]);
        assert_eq!(rows[0].fields[GROUP_SIZE_FIELD], json!(2_000_000_000i64));
        // And every row carries the same answer, so a client reads it off any
        // one of them without knowing which.
        assert!(rows
            .iter()
            .all(|row| row.fields[GROUP_SIZE_FIELD] == json!(2_000_000_000i64)));
    }

    #[test]
    fn a_group_verdict_distinguishes_none_some_and_all() {
        let label = |rows: &[ResourceItem]| rows[0].fields[GROUP_USAGE_FIELD]["label"].clone();
        let tone = |rows: &[ResourceItem]| rows[0].fields[GROUP_USAGE_FIELD]["tone"].clone();

        let none = summarized(vec![
            image_row("app", "1", "sha256:a", 10, ""),
            image_row("app", "2", "sha256:b", 10, ""),
        ]);
        assert_eq!(label(&none), json!("Unused"));
        // Caution, not negative: unused images are reclaimable disk, not a
        // fault, and a healthy host should not look broken.
        assert_eq!(tone(&none), json!("caution"));

        let all = summarized(vec![
            image_row("app", "1", "sha256:a", 10, "web"),
            image_row("app", "2", "sha256:b", 10, "api"),
        ]);
        assert_eq!(label(&all), json!("In use"));
        assert_eq!(tone(&all), json!("positive"));

        let some = summarized(vec![
            image_row("app", "1", "sha256:a", 10, "web"),
            image_row("app", "2", "sha256:b", 10, ""),
        ]);
        assert_eq!(label(&some), json!("Some unused"));

        // Two tags of one *used* image are one used image, not two — the same
        // distinct-image view the size total needs.
        let one_image_two_tags = summarized(vec![
            image_row("app", "1", "sha256:a", 10, "web"),
            image_row("app", "latest", "sha256:a", 10, "web"),
        ]);
        assert_eq!(label(&one_image_two_tags), json!("In use"));
    }

    #[test]
    fn the_scratch_image_id_never_reaches_a_client() {
        let rows = summarized(vec![image_row("app", "1", "sha256:a", 10, "web")]);
        assert!(!rows[0].fields.contains_key(IMAGE_ID_FIELD));
    }

    #[test]
    fn prune_asks_for_unused_images_not_merely_dangling_ones() {
        // Docker's default prune keeps every tagged image, however unused. The
        // table marks unused images with a pill and the button says "prune
        // unused"; this filter is what makes those agree.
        let options = PruneImagesOptionsBuilder::new()
            .filters(&prune_filters())
            .build();
        assert_eq!(
            options.filters,
            Some(HashMap::from([(
                "dangling".to_owned(),
                vec!["false".to_owned()]
            )]))
        );
    }

    #[test]
    fn byte_counts_in_a_message_read_like_prose() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(1024), "1.0 KiB");
        assert_eq!(human_bytes(3_892_314_112), "3.6 GiB");
    }

    #[test]
    fn short_ids_read_the_way_docker_prints_them() {
        assert_eq!(short_id("sha256:0123456789abcdef0123"), "0123456789ab");
        assert_eq!(short_id("0123456789abcdef"), "0123456789ab");
    }

    #[test]
    fn every_declared_kind_is_host_only_and_fully_described() {
        for kind in resource_kinds() {
            assert_eq!(
                kind.applicable_target,
                ApplicableTarget::HostOnly,
                "{} should be host-only",
                kind.kind
            );
            for action in kind.row_actions.iter().chain(&kind.kind_actions) {
                assert!(
                    action.description.is_some(),
                    "{} has no description",
                    action.id
                );
                assert!(owns_action(&action.id), "{} is not routed", action.id);
                // Every property a client will render a field for needs a
                // human title; the raw camelCase key is not a label. An action
                // that takes nothing declares no properties at all, which is
                // not the same as declaring an untitled one.
                let properties = action.params_schema["properties"]
                    .as_object()
                    .cloned()
                    .unwrap_or_default();
                for (name, property) in &properties {
                    assert!(
                        property["title"].is_string(),
                        "{}.{name} has no title",
                        action.id
                    );
                }
            }
        }
        assert_eq!(images_kind().group_by_key.as_deref(), Some("repository"));
        // The group summary's keys are group-level fields, deliberately *not*
        // columns: they belong on the heading, never in a row.
        let images = images_kind();
        let summary: Vec<&str> = images
            .group_summary
            .iter()
            .map(|column| column.key.as_str())
            .collect();
        assert_eq!(summary, vec![GROUP_USAGE_FIELD, GROUP_SIZE_FIELD]);
        for key in &summary {
            assert!(
                !images.columns.iter().any(|column| column.key == *key),
                "{key} is both a column and a group summary"
            );
        }
        // A grouped kind with no summary is still legal; the other two use it.
        assert!(volumes_kind().group_summary.is_empty());
        assert_eq!(volumes_kind().group_by_key, None);
        assert_eq!(networks_kind().group_by_key, None);
    }
}
