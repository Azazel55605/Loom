# 0007 — The in-process connector trait, separate from the connector transport

- Status: accepted
- Date: 2026-08-19

## Context

[0002](./0002-connector-contract-tbd.md) left the connector contract open: a
declarative manifest tier, a sandboxed WASM/Extism plugin tier, or both. That
question is about **how a connector definition arrives** — reviewed data, a
sandboxed module, or a compiled-in Rust type — and it is still open.

It is not, however, the only question. Whichever transport wins, something in
`loom-core` has to drive a connector and hand the result to `web-backend`, which
serializes it to the clients. The web frontend "frame" is the next thing to be
built, and it cannot be built against nothing: it needs the shape of a status, a
list of available actions, and the result of executing one. Waiting for 0002
before defining that shape would block all client work on a decision we
deliberately deferred until real services exist to design against.

The reverse ordering is also wrong. 0002 wants the transport designed against
real connectors; real connectors are easier to write once there is a host-side
trait to write them against.

## Decision

Define the **host-side** connector contract now, independently of the transport,
as a Rust trait in `crates/core/src/connector/`:

- `Connector` — `status`, `actions`, `execute_action`, `config_schema`,
  `metadata` — deliberately dyn-compatible, so `web-backend` can hold a
  heterogeneous registry of `dyn Connector` in shared state.
- Its wire types (`ConnectorStatus`, `ConnectorAction`, `ActionResult`,
  `ConnectorMetadata`, `ConnectorError`), all `Serialize`/`Deserialize` with
  `camelCase` field names, because they cross the HTTP boundary unchanged and
  are consumed by TypeScript clients.
- `config_schema` returns JSON Schema rather than a Rust type, so a connector
  can describe its own configuration to both a manifest validator and a
  form-rendering client without any of them knowing the connector exists.

This trait is what **both** tiers of 0002 would be driven through: a manifest
loader would produce an implementation backed by declarative data, and a plugin
host would produce one backed by a WASM module. Choosing between them does not
require changing this trait, only adding an implementation of it.

`DebugConnector` ships alongside it as a permanent fixture — configurable
health, latency, and failure mode, plus simulated data points and a default
widget layout — so client work does not depend on live infrastructure. See
`docs/API_CONTRACT.md` for the serialized shapes.

**Extended by [0011](./0011-connector-instance-registry.md)**, which adds the
presentation half of the trait (`display_fields`, `data_points`,
`default_layout`) and says where connector instances come from. The trait
defined here is unchanged in kind; 0011 only adds to it.

## Consequences

0002 is **narrowed, not superseded**. Its open question stands, and this ADR
does not answer it. What changes is the scope of its "anything written before it
lands should be treated as throwaway" consequence: that applies to connector
*definitions* and the loading mechanism, not to the host-side trait and its wire
types, which are now a deliberate interface with clients built against them.

The wire types are therefore harder to change than ordinary internal code. They
are public API in practice from the moment the frontend deserializes them, so
changes to field names or shapes should be deliberate and coordinated rather
than incidental. That cost is accepted in exchange for unblocking client work.

If designing the first real connectors shows the trait cannot express something
a real service needs — multi-step actions, streaming, pagination, long-running
jobs with progress — the trait changes and this ADR is superseded. That is the
expected way to find out, and it is cheaper than guessing now.

`config_schema` returning an unvalidated `serde_json::Value` means a malformed
schema is a runtime problem, not a compile-time one. Accepted for now: the
alternative is a typed schema builder, which is a lot of machinery before there
is a second consumer to justify it.
