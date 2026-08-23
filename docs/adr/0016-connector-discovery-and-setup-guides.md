# 0016 — Connector discovery is instance-scoped; setup guides are type-scoped

- Status: accepted
- Date: 2026-08-23

## Context

Some configured services can enumerate child resources that Loom could manage.
The connection needed to do that belongs to a live connector instance, while
the instructions for configuring a connector belong to its type. Neither
capability should require connector-specific routes or frontend code.

ADR number 0012 was already assigned to connector status push before this
decision was recorded, so this decision uses the next available number rather
than rewriting accepted history.

## Decision

`Connector` gains three opt-in methods. `discoverable_type()` identifies the
type of resource a live instance can find, `discover()` returns suggested names
and configurations, and `setup_guide()` publishes descriptive setup content.
Their defaults are unsupported (`None`) or empty, so existing connectors do not
acquire behaviour merely by compiling against the expanded trait.

Discovery is **instance-scoped**. `POST /connector-instances/{id}/discover`
runs against the already-configured live object and requires
`connectors.manage`, matching the authority needed to turn a suggestion into a
new instance. Discovery returns proposals only; it never writes rows.

Setup guides are **type-scoped** and are snapshotted into the connector type
registry alongside the config schema. A guide is a description plus plain text
whose `{{fieldName}}` placeholders refer to camelCase schema property names.
Substitution happens in clients. Core does not interpret templates or produce
rendered markup.

The registry constructs one cheap default connector per registered type and
reads schema, guide, discovery type, and icon from that same object. This keeps
type-level descriptors together without duplicating documents as constants.

DebugConnector proves the complete path before a real integration exists. Its
self-referential discovery returns valid `debug` configurations, and its setup
guide references the real `simulatedHealth` schema property.

## Consequences

- Any connector can opt into discovery without a new route or registry shape.
- A client can decide whether to show discovery from instance detail rather
  than hardcoding connector type ids.
- Suggestions still pass through ordinary instance creation and factory
  validation before they become durable.
- Setup templates are deliberately small and non-executable; richer guide
  formats require a later decision rather than being smuggled into Core.
