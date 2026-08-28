# 0020 — Connector setup variants and capability checks

- Status: accepted
- Date: 2026-08-28

## Context

A connector may support several independent upstream setup approaches. Those
approaches can expose different read-only or mutating capabilities, and a user
needs to understand both what their chosen configuration declares and what a
candidate connection can actually reach before persisting an instance. The
existing single description/template guide cannot express those alternatives.

ADR number 0015 already belongs to dashboard tile grouping, so this decision
uses the next available number rather than rewriting accepted history.

## Decision

This supersedes only the flat setup-guide shape from
[ADR 0016](./0016-connector-discovery-and-setup-guides.md); that ADR's discovery
decisions remain in force.

`SetupGuide` contains an ordered list of independent `SetupGuideVariant`
values. Each variant has its own description and plain-text template plus
optional UI-only toggles. Toggle values never become connector configuration
and are never persisted. They drive template rendering and declarative
capability requirements only.

A `CapabilityRequirement` names a capability and the toggle keys required to
unlock it. Version one is intentionally AND-only: all named toggles must be on.
The model will gain richer expressions only when a real connector needs them.

`Connector::test_connection()` is a lightweight, explicit reachability and
capability check for setup. It is distinct from the full recurring `status()`
poll. The default implementation maps ordinary health to reachability and
returns no fine-grained capabilities, so existing connectors gain useful basic
behaviour without an override.

`POST /connector-types/{typeId}/test-connection` follows type-scoped discovery:
the backend constructs a connector from candidate configuration, invokes the
check once, returns the result, and discards the connector without persistence
or runtime insertion.

Declarative requirements and live checks are separate mechanisms that share
stable capability keys and the same user-facing capability concept. A setup
toggle can explain what a proposed deployment should allow; a live check can
report what the candidate connection actually permits.

Destructive and write capabilities are never live-probed by performing their
actions. There is no generally safe no-op mutation. Their availability must be
reported declaratively from configuration or through non-mutating remote
metadata.

## Consequences

- Clients can present multiple setup paths without connector-specific UI.
- Toggle-driven guidance cannot accidentally alter persisted connector config.
- Candidate checks are useful before creation and leave no durable state.
- Connectors can opt into detailed capability reporting incrementally.
- Capability keys must remain stable wherever declarative and live results are
  compared.
- The first model cannot express OR logic; that limitation is explicit rather
  than hidden in client code.
