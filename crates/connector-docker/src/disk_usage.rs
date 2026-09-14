//! Compatibility reader for Docker's two `/system/df` response shapes.
//!
//! Engine API 1.52 added the aggregate `*Usage` objects while retaining the
//! legacy arrays. Older daemons expose only `LayersSize` plus the resource
//! arrays. Bollard's current generated model represents only the new objects,
//! so deserializing an older response through it silently discards every field.

use std::time::Duration;

use serde_json::Value;

use crate::config::{DockerConnectorConfig, READ_TIMEOUT_SECONDS};

#[derive(Debug, PartialEq)]
pub(crate) struct DiskUsageReading {
    pub(crate) total_size: Option<i64>,
    pub(crate) image_size: Option<i64>,
    pub(crate) images: Vec<Value>,
    pub(crate) containers: Vec<Value>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DiskUsageError {
    TimedOut,
    Unavailable(String),
    UnsupportedShape,
}

pub(crate) async fn read(
    config: &DockerConnectorConfig,
) -> Result<DiskUsageReading, DiskUsageError> {
    let builder = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(READ_TIMEOUT_SECONDS));

    let (client, url) = if let Some(path) = config.docker_host.strip_prefix("unix://") {
        #[cfg(unix)]
        {
            let client = builder
                .unix_socket(path)
                .build()
                .map_err(|error| DiskUsageError::Unavailable(error.to_string()))?;
            (client, "http://localhost/system/df".to_owned())
        }
        #[cfg(not(unix))]
        {
            let _ = (builder, path);
            return Err(DiskUsageError::Unavailable(
                "Unix Docker sockets are unavailable on this platform".to_owned(),
            ));
        }
    } else {
        let origin = config
            .docker_host
            .strip_prefix("tcp://")
            .map(|host| format!("http://{host}"))
            .unwrap_or_else(|| config.docker_host.trim_end_matches('/').to_owned());
        let client = builder
            .build()
            .map_err(|error| DiskUsageError::Unavailable(error.to_string()))?;
        (client, format!("{origin}/system/df"))
    };

    let response = client.get(url).send().await.map_err(request_error)?;
    let status = response.status();
    let body = response.bytes().await.map_err(request_error)?;
    if !status.is_success() {
        let detail = serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("message")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            });
        return Err(DiskUsageError::Unavailable(match detail {
            Some(detail) => format!("/system/df returned HTTP {status}: {detail}"),
            None => format!("/system/df returned HTTP {status}"),
        }));
    }

    let response = serde_json::from_slice(&body).map_err(|error| {
        DiskUsageError::Unavailable(format!("/system/df returned malformed JSON: {error}"))
    })?;
    parse(&response)
}

fn request_error(error: reqwest::Error) -> DiskUsageError {
    if error.is_timeout() {
        DiskUsageError::TimedOut
    } else {
        DiskUsageError::Unavailable(error.to_string())
    }
}

pub(crate) fn parse(response: &Value) -> Result<DiskUsageReading, DiskUsageError> {
    let Some(object) = response.as_object() else {
        return Err(DiskUsageError::UnsupportedShape);
    };

    let aggregate_keys = [
        "ImageUsage",
        "ContainerUsage",
        "VolumeUsage",
        "BuildCacheUsage",
    ];
    if aggregate_keys.iter().any(|key| object.contains_key(*key)) {
        let image_size = nested_size(response, "ImageUsage", "TotalSize");
        let total_size = aggregate_keys
            .iter()
            .filter_map(|key| nested_size(response, key, "TotalSize"))
            .reduce(i64::saturating_add);
        return Ok(DiskUsageReading {
            total_size,
            image_size,
            images: nested_items(response, "ImageUsage"),
            containers: nested_items(response, "ContainerUsage"),
        });
    }

    let legacy_keys = [
        "LayersSize",
        "Images",
        "Containers",
        "Volumes",
        "BuildCache",
    ];
    if !legacy_keys.iter().any(|key| object.contains_key(*key)) {
        return Err(DiskUsageError::UnsupportedShape);
    }

    let images = array(response, "Images");
    let containers = array(response, "Containers");
    let image_size = integer(response.get("LayersSize")).or_else(|| sum_rows(&images, &["Size"]));
    let container_size = sum_rows(&containers, &["SizeRw"]);
    let volume_size = sum_rows(&array(response, "Volumes"), &["UsageData", "Size"]);
    let build_cache_size = sum_rows(&array(response, "BuildCache"), &["Size"]);
    let total_size = [image_size, container_size, volume_size, build_cache_size]
        .into_iter()
        .flatten()
        .reduce(i64::saturating_add);

    Ok(DiskUsageReading {
        total_size,
        image_size,
        images,
        containers,
    })
}

fn nested_size(response: &Value, object: &str, field: &str) -> Option<i64> {
    integer(response.get(object)?.get(field))
}

fn nested_items(response: &Value, object: &str) -> Vec<Value> {
    response
        .get(object)
        .and_then(|value| value.get("Items"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn array(response: &Value, field: &str) -> Vec<Value> {
    response
        .get(field)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

fn sum_rows(rows: &[Value], path: &[&str]) -> Option<i64> {
    let mut found = false;
    let total = rows
        .iter()
        .filter_map(|row| {
            let value = path.iter().try_fold(row, |value, key| value.get(*key));
            integer(value).filter(|value| *value >= 0)
        })
        .inspect(|_| found = true)
        .fold(0_i64, i64::saturating_add);
    found.then_some(total)
}

fn integer(value: Option<&Value>) -> Option<i64> {
    value.and_then(|value| {
        value
            .as_i64()
            .or_else(|| value.as_u64().and_then(|value| i64::try_from(value).ok()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> Value {
        serde_json::from_str(match name {
            "26" => include_str!("../tests/fixtures/system_df_26.json"),
            "28" => include_str!("../tests/fixtures/system_df_28.json"),
            "29" => include_str!("../tests/fixtures/system_df_29.json"),
            _ => unreachable!("known fixture"),
        })
        .expect("fixture is valid JSON")
    }

    #[test]
    fn real_legacy_26_shape_uses_layers_and_available_row_sizes() {
        let reading = parse(&fixture("26")).expect("Docker 26 response");
        assert_eq!(reading.image_size, Some(1_000));
        assert_eq!(reading.total_size, Some(1_350));
        assert_eq!(reading.images.len(), 1);
        assert_eq!(reading.containers.len(), 2);
    }

    #[test]
    fn real_legacy_28_shape_uses_layers_and_all_legacy_resource_arrays() {
        let reading = parse(&fixture("28")).expect("Docker 28 response");
        assert_eq!(reading.image_size, Some(2_000));
        assert_eq!(reading.total_size, Some(2_750));
        assert_eq!(reading.images.len(), 1);
        assert_eq!(reading.containers.len(), 1);
    }

    #[test]
    fn real_29_shape_prefers_the_new_aggregate_objects() {
        let reading = parse(&fixture("29")).expect("Docker 29 response");
        assert_eq!(reading.image_size, Some(3_000));
        assert_eq!(reading.total_size, Some(4_000));
        assert_eq!(
            reading.images[0]["RepoTags"][0],
            "example.invalid/app:latest"
        );
        assert_eq!(reading.containers[0]["State"], "running");
    }

    #[test]
    fn an_unknown_success_shape_is_not_invented_as_zero() {
        assert_eq!(
            parse(&serde_json::json!({})),
            Err(DiskUsageError::UnsupportedShape)
        );
    }
}
