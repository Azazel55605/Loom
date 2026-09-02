# ADR 0032: Connector status carries optional per-target health

## Context

Real mixed-health TrueNAS usage exposed a generic gap in the connector status
contract. A connector had one aggregate health value even when its independently
addressable pools, datasets, containers, or stacks had different states. A
dashboard placement for a healthy pool therefore inherited a degraded host
badge, while deriving health from arbitrary detail strings in the frontend
would make every connector invent its own client-side convention.

## Decision

`ConnectorStatus` includes `targetHealth`, keyed exactly like the first level of
`details`: the empty string identifies the host/aggregate view and every other
key is a connector-declared sub-target id. The field defaults to an empty map
when deserializing old values, and clients fall back to aggregate `health` when
an entry is absent.

Connectors own the mapping from service-native state to Loom's four
`HealthState` values. Dashboard placement headers prefer their target entry,
but action availability continues to use aggregate connector reachability. A
stopped Docker container can therefore show Down without disabling its Start
action.

## Consequences

- Mixed-health connectors can report honest per-placement badges without
  connector-specific UI code.
- Existing connectors and serialized statuses remain compatible through the
  empty-map fallback.
- `health` remains the aggregate service verdict used by list views and
  connector-level controls; `targetHealth` does not replace it.
- Connector implementations must keep their target ids aligned across
  descriptors, details, actions, and health entries.
