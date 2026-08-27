# 0019 — Docker host and container views share one connector type

- Status: superseded by [0016 — Connector sub-targets](./0016-connector-sub-targets.md)
- Date: 2026-08-27

## Context

A connection to a Docker daemon grants daemon-level authority whether Loom
shows host totals or focuses on one container. Modeling those views as
`docker-host` and `docker-container` integrations would imply two security and
connection boundaries where only one exists, duplicate setup choices, and make
a user create a host connector only to discover the container connector they
actually wanted.

The existing Docker connector already had the correct single-container
monitoring, history, logs, and lifecycle behavior. What it lacked was the host
view and a way for an add-connector form to use a candidate `dockerHost` to
discover a value for `containerName` before any instance had been persisted.
ADR 0016 made discovery instance-scoped, which serves bulk creation from an
existing source but cannot serve that pre-creation flow.

ADR number 0014 was already assigned to the accepted widget-binding model when
this decision was recorded, so this uses the next available number rather than
creating two ADRs with the same identity.

## Decision

There is one connector type, `docker`. Its required `dockerHost` identifies the
connection. Optional `containerName` selects the view:

- omitted or blank: host mode, with daemon-wide container/image counts, disk
  usage and Docker version, no actions, and container discovery;
- set to an exact name or id: container mode, preserving the existing status,
  resource history, logs, and lifecycle actions.

Construction always proves the daemon is reachable. Container mode additionally
inspects the named container; host mode has no container existence check.
Existing `docker-container` and `docker-host` rows are migrated to `docker`
without changing their configuration, because presence of `containerName`
already contains the mode distinction.

The connector contract also gains `discovery_target_field()`. A discovered
resource may carry `targetFieldValue`, allowing generic clients to assign a
result to that field without knowing Docker. Docker always declares
`containerName` as its target field, while only host-mode instances are
themselves discoverable.

Discovery now has a second, complementary endpoint:
`POST /connector-types/{typeId}/discover` constructs a connector from candidate
configuration, validates it through the normal factory, performs one discovery
pass, and discards it. It never writes an instance row or inserts into the
runtime map. The existing instance-scoped endpoint remains the path for
discovering multiple proposed instances from an already-persisted connection.

## Consequences

- The type picker has one Docker choice and one setup form.
- Host and container dashboards retain different data points, actions, layouts,
  and minimum-size decisions even though their metadata id is the same.
- Candidate discovery performs real connection validation and may return a 400
  before anything is saved.
- A connector may expose discovery capabilities that depend on candidate
  configuration, so the static type catalog is not the final authority for
  whether one particular candidate can discover.
- ADR 0016 remains valid for setup guides and instance-scoped discovery; its
  claim that discovery is only instance-scoped is extended by this decision.
