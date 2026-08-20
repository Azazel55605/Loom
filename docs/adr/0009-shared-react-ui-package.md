# 0009 — Shared React UI and client logic live in a pnpm package

- Status: accepted
- Date: 2026-08-20

## Context

ADR 0001 anticipated a shared UI kit inside `crates/core`. The implemented
clients use React and TypeScript, while Core is a Rust library with a deliberately
narrow connector/business-logic boundary. Putting React source in the Rust
crate would couple unrelated build systems without making either one reusable.

Web/frontend already contains the working UI, API types, authentication flow,
and settings screens that Desktop and later Mobile need. Copying those files
would immediately create three implementations of the same behavior.

Two concerns are genuinely platform-specific: sensitive token persistence and
backend base-URL discovery. Browser `localStorage` and `import.meta.env` are not
valid assumptions for every Tauri client.

## Decision

The shared React UI and client logic live in the private pnpm workspace package
`packages/ui-kit` (`@loom/ui-kit`). Web/frontend consumes its TypeScript source
through package exports; Desktop and Mobile will consume the same package as
they are integrated.

The package owns components, design tokens, API wire types and transport,
authentication context, permission helpers, and shared settings panels. It has
no dependency on React Router. Platform shells inject navigation controls and
callbacks.

The API/auth runtime is constructed with two asynchronous adapters:

- `TokenStorageAdapter` persists, reads, and clears token pairs.
- `BaseUrlProvider` resolves the backend URL or proxy prefix.

Appearance preferences remain in `localStorage`: they are non-sensitive and a
Tauri webview supports that storage model.

This supersedes only ADR 0001's proposed location for the future UI kit. Its
Cargo workspace and Rust crate boundaries remain accepted.

## Consequences

Web/frontend becomes a thin browser integration layer: routing, browser adapter
implementations, and bootstrap providers. Shared modules must not read
`import.meta.env` or authentication-token `localStorage` directly.

Tailwind must scan `packages/ui-kit/src`, and clients share the package's preset
and stylesheet so moving a component does not change its generated classes or
tokens.

`pnpm build` and `pnpm test` now include packages as well as apps. The Rust Core
crate remains independent of the React toolchain and retains its library-only,
no-authorization boundary.
