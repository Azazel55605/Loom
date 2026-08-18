# `apps/`

Placeholder. The non-Rust-workspace deliverables live here once they are
scaffolded:

- **`web-frontend/`** — browser SPA, deployed independently of the backend.
- **`desktop/`** — Tauri desktop client.
- **`mobile/`** — Tauri mobile client.

All three are clients of `crates/web-backend` and talk to it over its HTTP API.
Desktop and mobile additionally share the UI kit that will live in
`crates/core`.

They will be scaffolded in a later task — nothing here is buildable yet. See
[`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md) for how these fit together.
