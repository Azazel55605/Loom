# 0001 — Monorepo with Cargo workspace crate boundaries

- Status: accepted
- Date: 2026-08-18

## Context

Loom will end up with several deliverables — a server, a browser frontend, and
Tauri desktop and mobile clients — that share connector logic, business rules,
and eventually UI components. Splitting these across repositories early would
mean version-pinning shared code and coordinating releases before we even know
what the shared surface looks like.

## Decision

Keep everything in one repository, organised as a Cargo workspace. The first
boundary is between two crates:

- `crates/core` (package `loom-core`) — a library only: connector trait and
  implementations, business logic, later a shared UI kit.
- `crates/web-backend` — the binary, the one running server, depending on Core
  via a workspace path dependency.

Non-Rust deliverables live under `apps/`. Shared metadata (edition, license,
version, dependency versions) is declared once at the workspace root.

## Consequences

The crate boundary is what lets the parts move at different speeds: the server
can be redeployed without touching client code, and Core can be refactored
behind its public API without a release dance. Path dependencies mean a change
to Core is compiled and tested against the backend in the same CI run, so
breakage surfaces immediately rather than at integration time.

The cost is that everything shares one commit history and one CI pipeline, and
that the boundary is only as good as our discipline about what goes in Core —
see [0003](./0003-auth-model-vpn-vs-external.md) for the rule that keeps
enforcement logic out of it. If a component later genuinely needs its own
release cadence, extracting a crate from a workspace is a well-trodden path.
