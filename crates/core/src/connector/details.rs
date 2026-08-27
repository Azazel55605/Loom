//! Helpers for the target-aware [`ConnectorStatus`](super::ConnectorStatus) detail shape.
//!
//! Every details payload is an object nested first by target and then by data
//! point id. The empty string is the reserved host/aggregate key; a real
//! sub-target uses its own stable id:
//!
//! ```json
//! {
//!   "": { "totalContainers": 4 },
//!   "web": { "cpuPercent": 12.5 }
//! }
//! ```
//!
//! Keeping this convention behind helpers prevents connectors and consumers
//! from inventing subtly different sentinel or nesting rules.

use serde_json::{Map, Value};

/// The reserved details key for connector-level/aggregate readings.
pub const HOST_TARGET_KEY: &str = "";

fn target_key(target_id: Option<&str>) -> &str {
    target_id.unwrap_or(HOST_TARGET_KEY)
}

/// Writes one data-point value at the connector or sub-target scope.
///
/// A malformed/non-object payload is replaced with the required object shape;
/// connector telemetry is more useful than preserving a scalar that no widget
/// can address.
pub fn set_detail(details: &mut Value, target_id: Option<&str>, data_point_id: &str, value: Value) {
    if !details.is_object() {
        *details = Value::Object(Map::new());
    }

    let targets = details
        .as_object_mut()
        .expect("details was normalized to an object");
    let target = targets
        .entry(target_key(target_id).to_owned())
        .or_insert_with(|| Value::Object(Map::new()));
    if !target.is_object() {
        *target = Value::Object(Map::new());
    }
    target
        .as_object_mut()
        .expect("target details was normalized to an object")
        .insert(data_point_id.to_owned(), value);
}

/// Reads one data-point value from the connector or sub-target scope.
pub fn get_detail<'a>(
    details: &'a Value,
    target_id: Option<&str>,
    data_point_id: &str,
) -> Option<&'a Value> {
    details
        .as_object()?
        .get(target_key(target_id))?
        .as_object()?
        .get(data_point_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn host_and_sub_target_details_round_trip_without_colliding() {
        let mut details = Value::Null;
        set_detail(&mut details, None, "load", json!(10));
        set_detail(&mut details, Some("fixture-a"), "load", json!(20));
        set_detail(&mut details, Some("fixture-b"), "enabled", json!(true));

        assert_eq!(get_detail(&details, None, "load"), Some(&json!(10)));
        assert_eq!(
            get_detail(&details, Some("fixture-a"), "load"),
            Some(&json!(20))
        );
        assert_eq!(
            get_detail(&details, Some("fixture-b"), "enabled"),
            Some(&json!(true))
        );
        assert_eq!(get_detail(&details, Some("fixture-b"), "load"), None);
        assert_eq!(details[HOST_TARGET_KEY]["load"], json!(10));
    }
}
