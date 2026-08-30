//! Reusable black-box contract assertions for Loom connectors.
//!
//! This crate is test infrastructure, not a connector runtime dependency. A
//! connector crate adds it under `[dev-dependencies]` and passes one configured
//! connector plus the host/sub-target scopes that are known to be valid in that
//! fixture. The assertions deliberately use only the public [`Connector`]
//! contract so they exercise the same descriptor relationships clients rely on.

use std::collections::{HashMap, HashSet};

use loom_core::connector::{
    details::get_detail, Connector, ConnectorAction, HealthState, ResourceKindDescriptor,
    WidgetBinding,
};

/// Assert that one configured connector obeys Loom's public connector contract.
///
/// `known_target_ids` should contain `None` for the host view and one
/// `Some(id)` for every real sub-target scope the fixture wants checked. The
/// function panics with connector- and target-specific messages so failures are
/// actionable in an ordinary `cargo test` run.
///
/// The action/resource-to-capability assertion is intentionally shared because
/// a real Docker regression added images, volumes and networks without adding
/// their setup-guide capability declarations. Keeping that relationship only
/// in connector-local tests had allowed implementation and test to drift
/// together.
pub async fn assert_connector_contract(
    connector: &dyn Connector,
    known_target_ids: &[Option<String>],
) {
    // Yield once so this test-only crate genuinely exercises the caller's
    // Tokio runtime rather than merely carrying it as an unused dependency.
    tokio::task::yield_now().await;

    let metadata = connector.metadata();
    let connector_id = metadata.id.as_str();
    assert_nonempty(connector_id, "metadata.id", connector_id);
    assert_nonempty(&metadata.name, "metadata.name", connector_id);
    if let Some(icon) = metadata.icon.as_deref() {
        assert!(
            icon.starts_with("brand:") || icon.starts_with("lucide:"),
            "connector `{connector_id}` metadata.icon `{icon}` must start with `brand:` or `lucide:`"
        );
    }
    assert!(
        metadata.min_size.0 > 0 && metadata.min_size.1 > 0,
        "connector `{connector_id}` metadata.min_size must have non-zero width and height"
    );

    let schema = connector.config_schema();
    let schema_object = schema.as_object().unwrap_or_else(|| {
        panic!("connector `{connector_id}` config_schema must be a JSON object")
    });
    let properties = schema_object
        .get("properties")
        .and_then(|value| value.as_object())
        .unwrap_or_else(|| {
            panic!("connector `{connector_id}` config_schema must contain a properties object")
        });
    for (key, property) in properties {
        if property
            .get("x-loom-sensitive")
            .and_then(|value| value.as_bool())
            == Some(true)
        {
            assert_eq!(
                property.get("type").and_then(|value| value.as_str()),
                Some("string"),
                "connector `{connector_id}` sensitive config property `{key}` must have type `string`"
            );
        }
    }

    let data_points = connector.data_points();
    let actions = connector.actions().await;
    let mut data_point_keys = HashSet::new();
    for point in &data_points {
        assert_nonempty(&point.id, "data point id", connector_id);
        assert_nonempty(&point.label, "data point label", connector_id);
        assert!(
            data_point_keys.insert((point.id.as_str(), point.target_id.as_deref())),
            "connector `{connector_id}` declares duplicate data point `{}` for target {}",
            point.id,
            target_label(point.target_id.as_deref())
        );
    }

    let mut action_keys = HashSet::new();
    let mut action_meanings: HashMap<String, ConnectorAction> = HashMap::new();
    for action in &actions {
        validate_action(action, connector_id, "connector action");
        assert!(
            action_keys.insert((action.id.as_str(), action.target_id.as_deref())),
            "connector `{connector_id}` declares duplicate action `{}` for target {}",
            action.id,
            target_label(action.target_id.as_deref())
        );
        record_action_meaning(
            &mut action_meanings,
            action,
            connector_id,
            "connector action",
        );
    }

    let mut scopes = Vec::new();
    let mut seen_scopes = HashSet::new();
    for target in known_target_ids {
        assert!(
            seen_scopes.insert(target.as_deref()),
            "connector `{connector_id}` contract fixture contains duplicate target {}",
            target_label(target.as_deref())
        );
        scopes.push(target.as_deref());
    }

    for target in &scopes {
        let layout = connector.default_layout_for(*target);
        for binding in &layout.bindings {
            match binding {
                WidgetBinding::Display { data_point_id, .. } => assert!(
                    data_point_keys.contains(&(data_point_id.as_str(), *target)),
                    "connector `{connector_id}` layout for target {} binds missing data point `{data_point_id}`",
                    target_label(*target)
                ),
                WidgetBinding::Action { action_id, .. } => assert!(
                    action_keys.contains(&(action_id.as_str(), *target)),
                    "connector `{connector_id}` layout for target {} binds missing action `{action_id}`",
                    target_label(*target)
                ),
            }
        }
    }

    let status = connector.status().await;
    if let Ok(status) = status {
        if matches!(status.health, HealthState::Healthy | HealthState::Degraded) {
            for target in &scopes {
                let target_points: Vec<_> = data_points
                    .iter()
                    .filter(|point| point.target_id.as_deref() == *target)
                    .collect();
                assert!(
                    !target_points.is_empty(),
                    "connector `{connector_id}` reports {:?} but declares no data points for target {}",
                    status.health,
                    target_label(*target)
                );
                assert!(
                    target_points
                        .iter()
                        .any(|point| get_detail(&status.details, *target, &point.id).is_some()),
                    "connector `{connector_id}` reports {:?} but supplies no declared data point value for target {}",
                    status.health,
                    target_label(*target)
                );
            }
        }
    }

    let mut resource_kinds: HashMap<String, ResourceKindDescriptor> = HashMap::new();
    for target in &scopes {
        for kind in connector.resource_kinds(*target) {
            validate_resource_kind(&kind, connector_id, *target);
            for action in kind.row_actions.iter().chain(&kind.kind_actions) {
                validate_action(action, connector_id, "resource action");
                record_action_meaning(
                    &mut action_meanings,
                    action,
                    connector_id,
                    &format!("resource kind `{}`", kind.kind),
                );
            }

            if let Some(existing) = resource_kinds.get(&kind.kind) {
                assert_eq!(
                    existing,
                    &kind,
                    "connector `{connector_id}` gives resource kind `{}` conflicting meanings across targets",
                    kind.kind
                );
            } else {
                resource_kinds.insert(kind.kind.clone(), kind);
            }
        }
    }

    let connection_test = connector.test_connection().await;
    if let Some(guide) = connector.setup_guide() {
        let guide_capabilities: HashSet<&str> = guide
            .variants
            .iter()
            .flat_map(|variant| &variant.capability_requirements)
            .map(|requirement| requirement.capability_key.as_str())
            .collect();
        let reported_capabilities: HashSet<&str> = connection_test
            .capabilities
            .iter()
            .map(|capability| capability.key.as_str())
            .collect();

        if !reported_capabilities.is_empty() {
            for capability in &guide_capabilities {
                assert!(
                    reported_capabilities.contains(capability),
                    "connector `{connector_id}` setup guide declares capability `{capability}` but test_connection does not report it"
                );
            }
        }

        for action in &actions {
            assert_capability_declared(
                connector_id,
                "action",
                &action.id,
                &guide_capabilities,
                true,
            );
        }
        for kind in resource_kinds.values() {
            assert_capability_declared(
                connector_id,
                "resource kind",
                &kind.kind,
                &guide_capabilities,
                false,
            );
        }
    }

    if connector.supports_sub_targets() {
        let targets = connector.list_sub_targets().await.unwrap_or_else(|error| {
            panic!(
                "connector `{connector_id}` supports sub-targets but listing them failed: {error}"
            )
        });
        for target in targets {
            assert_nonempty(&target.id, "sub-target id", connector_id);
            assert_nonempty(&target.label, "sub-target label", connector_id);
            assert_nonempty(&target.kind, "sub-target kind", connector_id);
        }
    }
}

fn assert_nonempty(value: &str, field: &str, connector_id: &str) {
    assert!(
        !value.trim().is_empty(),
        "connector `{connector_id}` {field} must not be empty"
    );
}

fn target_label(target_id: Option<&str>) -> String {
    target_id.map_or_else(|| "<host>".to_owned(), |id| format!("`{id}`"))
}

fn validate_action(action: &ConnectorAction, connector_id: &str, context: &str) {
    assert_nonempty(&action.id, &format!("{context} id"), connector_id);
    assert_nonempty(&action.label, &format!("{context} label"), connector_id);
}

fn same_action_meaning(left: &ConnectorAction, right: &ConnectorAction) -> bool {
    left.params_schema == right.params_schema
        && left.is_disruptive == right.is_disruptive
        && left.snapshot_data_point_ids == right.snapshot_data_point_ids
}

fn record_action_meaning(
    meanings: &mut HashMap<String, ConnectorAction>,
    action: &ConnectorAction,
    connector_id: &str,
    context: &str,
) {
    if let Some(existing) = meanings.get(&action.id) {
        assert!(
            same_action_meaning(existing, action),
            "connector `{connector_id}` reuses action id `{}` for semantically different actions ({context})",
            action.id
        );
    } else {
        meanings.insert(action.id.clone(), action.clone());
    }
}

fn validate_resource_kind(
    kind: &ResourceKindDescriptor,
    connector_id: &str,
    target_id: Option<&str>,
) {
    assert_nonempty(&kind.kind, "resource kind id", connector_id);
    assert_nonempty(&kind.label, "resource kind label", connector_id);
    assert!(
        !kind.columns.is_empty(),
        "connector `{connector_id}` resource kind `{}` for target {} must declare at least one column",
        kind.kind,
        target_label(target_id)
    );
    for column in &kind.columns {
        assert_nonempty(
            &column.key,
            &format!("resource kind `{}` column key", kind.kind),
            connector_id,
        );
        assert_nonempty(
            &column.label,
            &format!("resource kind `{}` column label", kind.kind),
            connector_id,
        );
    }
}

fn assert_capability_declared(
    connector_id: &str,
    descriptor: &str,
    descriptor_id: &str,
    capabilities: &HashSet<&str>,
    action: bool,
) {
    let descriptor_tokens = identifier_tokens(descriptor_id);
    let covered = capabilities.iter().any(|capability| {
        let capability_tokens = identifier_tokens(capability);
        (action && capability_tokens.iter().any(|token| token == "action"))
            || descriptor_tokens
                .iter()
                .any(|token| capability_tokens.contains(token))
    });
    assert!(
        covered,
        "connector `{connector_id}` {descriptor} `{descriptor_id}` has no matching capability requirement in its setup guide"
    );
}

fn identifier_tokens(identifier: &str) -> HashSet<String> {
    let mut words = Vec::new();
    let mut current = String::new();
    for character in identifier.chars() {
        if character.is_ascii_uppercase() && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        if character.is_ascii_alphanumeric() {
            current.push(character.to_ascii_lowercase());
        } else if !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        words.push(current);
    }
    words
        .into_iter()
        .filter(|word| !matches!(word.as_str(), "list" | "read" | "view" | "perform"))
        .map(|word| {
            if word.len() > 3 && word.ends_with('s') && word != "status" {
                word[..word.len() - 1].to_owned()
            } else {
                word
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::identifier_tokens;

    #[test]
    fn capability_tokens_normalize_case_and_plural_words() {
        assert_eq!(
            identifier_tokens("pullImages"),
            ["pull".to_owned(), "image".to_owned()].into()
        );
        assert!(identifier_tokens("read-status").contains("status"));
    }
}
