# 0014 — Widget bindings are a tagged enum, and status details are data-point-keyed

- Status: accepted
- Date: 2026-08-23

## Context

Two shapes introduced by [0011](./0011-connector-instance-registry.md) turned
out to be underspecified once the dashboard placements of
[0013](./0013-dashboard-sharing-model.md) started storing them and the status
push of [0012](./0012-connector-status-push.md) started streaming them. Both
gaps are the same kind of mistake — a field that carried a shape only by
convention — and both were cheapest to fix before any widget renderer was
written against them.

**`WidgetBinding` was flat.** It held one `data_point_id`, one `widget_type`,
and a free-form `config`, with `widget_type` drawn from a single eleven-variant
enum covering both the read-only primitives (`StatTile`, `ProgressBar`,
`MetricChart`, `Gauge`, `StatusDot`, `LogStream`) and the controls (`Button`,
`Toggle`, `Slider`, `TextField`, `Selector`). The field's doc comment conceded
the problem outright: the id meant a `DataPointDescriptor::id` "or, for a
control widget, a `ConnectorAction::id`". Those are different identifier spaces
that happen to share a type. The consequences were concrete:

- A control binding could not be validated. The backend's placement check
  resolved every binding against `data_points()`, so an action binding either
  named a real action and was rejected, or named a data point and was accepted
  while being impossible to invoke. In practice action bindings were simply not
  expressible, which is why nothing in the tree had written one.
- A renderer had to infer which namespace an id lived in by matching on the
  widget type — a rule enforced nowhere, checked by no type, and silently
  wrong the first time a new widget was added on the other side of the line.

**`ConnectorStatus::details` was documented as unstructured.** "Intentionally
unstructured", it said, on the same struct whose `data_points()` descriptors
promised their values would arrive there keyed by id. Every implementation
already did that, and 0012 pushes those payloads to clients as the render input
for saved layouts. The field was a contract that no one had written down, so a
connector could have broken it without breaking a test.

## Decision

**`WidgetBinding` is an externally tagged enum with a `Display` and an `Action`
variant**, each naming its own id field:

```json
{ "display": { "dataPointId": "load", "widgetType": "gauge", "config": {} } }
{ "action":  { "actionId": "restart", "widgetType": "button", "config": {} } }
```

`WidgetType` splits along the same line into `DisplayWidgetType` (the six
read-only primitives) and `ActionWidgetType` (the five controls), so a display
binding cannot name a button and an action binding cannot name a chart. Both
enums stay closed, for the reason the original one was: an unrecognised widget
type is a blank space in a dashboard with no explanation, so adding a primitive
is deliberately a change to Core and to every renderer.

External tagging matches `ConnectorError` and `DisplayWidgetType::MetricChart`,
which is the wire convention already in force across this crate.

Placement validation in `web-backend` resolves each binding against the
namespace its tag names: `display` against `data_points()`, `action` against
`actions()`. A 400 names the invalid ids and says which kind each one is, so
"unknown data point restart" cannot send someone looking in the wrong half of
the connector.

**`ConnectorStatus::details` is a formalized data-point-keyed object.** It stays
`serde_json::Value` in Rust — a connector's telemetry does not fit one struct,
and it may add keys that are not data points — but the shape of the data-point
entries is now specified rather than conventional: keyed by `data_point_id`,
with each value shaped by that data point's declared `value_type`, and a
`TimeSeries` serializing as an array of `{ "timestamp", "value" }` objects
oldest-first. `ConnectorStatus::data_point_value` is the accessor, and it
returns `None` rather than guessing when `details` is not an object at all.

The reason for pinning the shape rather than leaving it loose is 0012: a widget
must be able to consume the status frames arriving on the WebSocket directly. A
separate "current values" endpoint keyed the way widgets need would mean two
sources of the same reading, drifting from each other between polls.

## Consequences

- This is a **breaking change to a wire type** that is stored, not just
  transmitted. `dashboard_placements.widget_bindings` holds serialized
  `WidgetBinding` JSON, so rows written in the flat shape no longer deserialize.
  There are no such rows outside development databases — placements landed in
  the same development cycle and were never released — so no migration is
  provided. A stored placement that fails to deserialize surfaces as an error on
  read rather than being silently dropped.
- Action bindings become validatable for the first time, and the `DebugConnector`
  default layout now ships both kinds, so the action half of a renderer has
  something to be built against before any real connector exists.
- Connector authors take on a stated obligation they were already meeting
  informally: `status().details` must key every declared data point. The trait's
  own object-safety test asserts it for every in-tree connector.
- The two widget enums have to be kept aligned with the renderers by hand.
  Nothing in the type system stops a connector from binding a `Slider` to an
  action whose `params_schema` takes no number; that mismatch is still caught at
  execution time by `ConnectorError::InvalidParams`, not at placement time.
- A future typed `config` (0011 left it free-form on purpose) can now be
  introduced per variant rather than as one struct spanning every widget, which
  was the shape that made typing it unattractive in the first place.
