# 0016 — Connector sub-targets

- Status: accepted; extended by [ADR 0027](./0027-docker-stacks.md)
- Date: 2026-08-27
- Supersedes: [0019 — Docker host and container views share one connector type](./0019-docker-connector-merge.md)

## Context

One authenticated connection can expose several addressable views. A Docker
daemon is the immediate case: the host aggregate and every container share one
socket and one authority boundary. Creating a connector instance per container
duplicates configuration and falsely presents those views as independent
connections.

The existing discovery contract solves a different problem. `discover()`
suggests whole new connector instances. A container behind an already-saved
Docker connection is instead an addressable view inside that instance and must
not be represented as a discovery proposal.

## Decision

A connector may opt into sub-targets and cheaply enumerate `SubTarget { id,
label }` values. Data-point and action descriptors carry an optional
`target_id`; null addresses the host/aggregate view and a value addresses that
exact sub-target. Dashboard placements persist the same optional target and
validate both target existence and binding identity against the live connector.

Status details use `details[targetKey][dataPointId]`, with the empty string as
the host sentinel and the sub-target id otherwise. Core supplies helpers so
connectors do not construct this convention independently.

Action dispatch deliberately changes to `execute_action(action_id, target_id,
params)`. This is a breaking connector-trait change: the target must remain
explicit all the way from the HTTP request to the connector rather than being
smuggled through configuration or params.

Docker configuration contains only `dockerHost`. One instance represents one
daemon; containers are its sub-targets. Persisted unknown configuration keys
are ignored so a stale `containerName` does not make an old row crash during
manual cleanup. No old instance or placement is migrated automatically.

## Consequences

- Discovery and sub-target enumeration coexist, with intentionally separate
  meanings and endpoints.
- A placement cannot bind data or actions from a target other than its own.
- The Docker poll fetches inspect, one-shot stats, and log detail for every
  container with bounded concurrency. CPU deltas use counters retained from
  the previous poll instead of waiting for Docker to produce two samples for
  every container. Daemon-wide disk usage and version are cached for one
  minute because `/system/df` can be large and slow through a socket proxy.
  Fetching only targets referenced by active placements remains a possible
  future optimization.
- Connector creation, replacement, action responses, and server startup do
  not wait for a status inventory. They schedule one on the background poller;
  until it completes, a new or replaced connector has a nullable status. This
  keeps a slow remote endpoint from turning an already-accepted operation into
  a client-side network timeout.
- `connectors.control` remains scoped to a connector instance. A grant over one
  Docker instance can control every container within that daemon connection.
  Per-target permission scoping is a possible future enhancement; it is not
  part of this decision.
- Existing pre-release per-container Docker instances and placements require
  manual deletion and recreation.
