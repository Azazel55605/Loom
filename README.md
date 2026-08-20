# Loom

<!-- Placeholder: replace <owner> with the GitHub owner once this repo has a remote. -->
[![CI](https://github.com/YOUR_GITHUB_OWNER/Loom/actions/workflows/ci.yml/badge.svg)](https://github.com/YOUR_GITHUB_OWNER/Loom/actions/workflows/ci.yml)

A modular, extensible homelab management platform — not just a dashboard. Loom
can *act* on your services through their APIs, not merely show you whether
they're up.

> **Early work in progress.** Architecture is still being finalized, and there
> is no usable functionality yet — the workspace currently builds a server with
> a single `/health` route. See [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md)
> and [`docs/adr/`](docs/adr/) for what has been decided so far, and what
> hasn't.

## Layout

| Path                 | What it is                                                    |
| -------------------- | ------------------------------------------------------------- |
| `crates/core`        | `loom-core` — shared Rust library for connectors and business logic. Never runs standalone. |
| `crates/web-backend` | The one running server. Owns auth, access control, features. All clients talk to it. |
| `apps/`              | Web frontend and Tauri desktop/mobile clients. |
| `packages/ui-kit`    | Shared React components, API/auth logic, and design tokens consumed by clients. |
| `docs/`              | Architecture notes, ADRs, versioning, and agent instructions.  |
| `versions.json`      | Single source of truth for every product's version — see [`docs/VERSIONING.md`](docs/VERSIONING.md). |

## Building from source

Requires a stable Rust toolchain (1.82+).

```sh
cargo build --workspace
cargo test --workspace
```

To run the server:

```sh
cargo run -p loom-web-backend
curl http://127.0.0.1:8080/health
# {"status":"ok","core_version":"0.1.0"}
```

Configuration is read from the environment; copy [`.env.example`](.env.example)
to `.env` for local defaults. Real secrets are never committed.

Before opening a pull request, run what CI runs:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm versions:sync:check
```

Version bumps go through `versions.json` — edit the entry, run
`pnpm versions:sync`, never hand-edit a manifest. See
[`docs/VERSIONING.md`](docs/VERSIONING.md).

## License

Licensed under the MIT license ([LICENSE](LICENSE)).

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in this work by you shall be licensed as above, without any
additional terms or conditions.
