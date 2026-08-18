# 0002 — Connector contract

- Status: **not yet finalized** — open, recorded to avoid relitigating
- Date: 2026-08-18

## Context

Loom acts on services, not just displays them, so it needs a contract for
"here is how to talk to service X". The question is how much power a connector
author gets, and how much of that we are willing to run in-process.

Two tiers have been discussed, and they are not mutually exclusive:

- **Config-only manifest tier.** A connector is declarative data — endpoints,
  auth style, request/response shapes, the actions it exposes. No third-party
  code executes. Easy to review, easy to ship, safe by construction, but capped
  at what the manifest schema can express.
- **WASM / Extism plugin tier.** A connector is a sandboxed WASM module loaded
  through [Extism](https://extism.org/), free to do arbitrary logic within the
  host functions we grant it. Far more expressive — handles services with
  awkward APIs, pagination, multi-step actions — at the cost of a sandbox
  boundary, a host ABI, and a real capability model to design.

## Decision

None yet. Both tiers remain candidates, and the likely shape is manifests for
the common case with a plugin escape hatch for the rest. We are explicitly
deferring until a handful of real connectors exist to design against, rather
than guessing at a contract first.

## Consequences

Connector work is blocked on this, and anything written before it lands should
be treated as throwaway. The upside of deciding late is that the contract gets
designed against real services instead of imagined ones. This ADR will be
superseded — not edited — once a decision is made, so the reasoning above
stays on the record.
