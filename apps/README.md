# `apps/`

The non-Rust-workspace deliverables live here:

- **`web-frontend/`** — browser SPA, deployed independently of the backend.
- **`desktop/`** — Tauri desktop client.
- **`mobile/`** — Tauri mobile client.

All three are clients of `crates/web-backend` and talk to it over its HTTP API.
Shared React UI and client logic live in `packages/ui-kit`; platform adapters
provide navigation, token storage, and backend URL resolution.

See [`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md) for how these fit together.
