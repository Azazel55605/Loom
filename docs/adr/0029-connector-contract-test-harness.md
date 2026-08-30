# 0029. Connector implementations share one public contract test harness

- Status: accepted
- Date: 2026-08-30
- Amends: [0025](./0025-capabilities-are-part-of-adding-a-feature.md)

> **Numbering.** This work was requested as ADR 0021, but 0021 is already the
> accepted resource-browser decision. It takes the next free number rather
> than overwriting history.

## Context

The `Connector` trait defines more than Rust method signatures. Its metadata,
data points, actions, layouts, status details, resource kinds, setup guide and
connection-test capabilities are separate values that must agree with one
another. Rust can prove that a connector returns each value; it cannot prove
that a layout names a real data point or that a new resource kind is represented
in the setup guide.

Those relationships were tested independently inside connector crates. The
tests consequently drifted with the implementations. The production example
was Docker: images, volumes and networks were added as resource kinds without
adding the socket-proxy toggles and capability requirements they needed. The
old connector-local test continued to pass because it encoded the old feature
set too.

## Decision

`crates/connector-test-kit` is a private workspace crate containing the public
async assertion
`assert_connector_contract(&dyn Connector, &[Option<String>])`. Connector
crates use it only as a dev-dependency. The caller supplies `None` for the host
and real fixture target ids for every sub-target scope it wants checked; the
harness remains connector-agnostic and uses only Core's public trait.

The harness checks these cross-method invariants:

- metadata and schema have the minimum valid shapes, including string typing
  for properties marked `x-loom-sensitive`;
- data-point and action `(id, target_id)` pairs are unique and every default
  layout binding resolves in its own target scope;
- a healthy or degraded status carries at least one declared reading for every
  checked scope;
- resource kinds have stable, non-conflicting definitions and usable columns;
- an action id reused across scopes or resource tables keeps the same parameter
  schema, disruptive meaning and snapshot contract;
- setup-guide requirement keys appear in a non-empty live capability report,
  and actions/resource kinds have a matching declared capability;
- connectors claiming sub-target support can enumerate non-empty ids, labels
  and kinds.

The capability match uses the stable descriptor and capability ids, normalized
across kebab-case, camelCase and plural nouns. An action may also be covered by
an explicit umbrella capability containing `action` (for example Debug's
`perform-actions`). Resource kinds deliberately have no umbrella escape: their
noun must appear in a guide capability. This is the generic form of the check
that would have caught Docker's missing image, volume and network declarations.
It is a naming convention, not an authorization mechanism; the backend remains
the authority for permissions.

Core's DebugConnector runs the harness against its host and both stable fixture
targets. Docker runs it in the existing availability-gated live suite against
the host, the test-created container, and a real stack target when the daemon
has one. The Docker test keeps the suite usable without a daemon while still
running against a real daemon in CI.

## Consequences

A new connector can prove its public descriptor graph with one test instead of
recreating a partial checklist. Adding a descriptor that is not wired through
layouts, readings or capabilities now fails with the connector id, target and
missing id in the assertion message.

The harness exposed two omissions immediately. Debug's `gadgets` resource kind
had no `view-gadgets` capability, and Docker's stack-members browser relied on
container listing without declaring its own user-facing capability. Both now
declare and report those read capabilities.

Some behavior remains connector-specific and stays in connector-local tests:
the harness does not execute mutating actions, validate service-specific JSON
value shapes, or invent fixture targets. It complements those tests rather than
replacing them.
