# Building Loom

Canonical reference for build and test commands. Use these rather than ad-hoc
invocations, so every contributor and CI job builds the same way.

## Prerequisites

| Tool | Needed for | Notes |
| --- | --- | --- |
| Rust (stable) | `core`, `web-backend`, desktop/mobile Tauri shells | Workspace pins `rust-version = "1.82"`. Install via [rustup](https://rustup.rs/). |
| Node 22.13+ | all frontend builds | Required by the pinned pnpm 11, which uses `node:sqlite`. Node 20 fails with `ERR_UNKNOWN_BUILTIN_MODULE`. |
| pnpm | all frontend builds, script orchestration | `corepack enable` — the version is pinned by `packageManager` in the root `package.json`. |
| Docker + Docker Compose | running the containerized stack | Only needed for container workflows, not for local builds. |
| Tauri system dependencies | desktop and mobile builds | OS-specific (webview, build tools, Android SDK/NDK). Follow [Tauri's prerequisites guide](https://tauri.app/start/prerequisites/) — deliberately not duplicated here, since per-OS package lists go stale quickly. |

## Install dependencies

Once, from the repo root:

```sh
pnpm install
```

This installs for every workspace package under `apps/`. Rust dependencies are
fetched by Cargo on first build; no separate step.

## Per-component builds

| Component | Command | Output |
| --- | --- | --- |
| core (library) | `pnpm build:core` | Compiled rlib inside `target/release/`. No standalone artifact — Core is a library and never runs on its own. |
| web-backend | `pnpm build:web-backend` | `target/release/loom-web-backend` binary |
| web-frontend | `pnpm build:web-frontend` | `apps/web-frontend/dist/` static site |
| desktop | `pnpm build:desktop` | Platform installers in `apps/desktop/src-tauri/target/release/bundle/` |
| mobile | `pnpm build:mobile` | Android APK/AAB via Gradle |

The Cargo packages are named `loom-core` and `loom-web-backend` — a crate named
`core` would collide with Rust's built-in `core`. The `pnpm build:*` scripts
already use the correct names; prefer them over hand-written `cargo` commands.

### `pnpm build` builds the frontend apps only

```sh
pnpm build     # === pnpm -r --if-present build
```

`pnpm -r` iterates **pnpm workspace packages** — that is, `apps/*`. The Rust
crates are Cargo workspace members, not pnpm packages, so **`pnpm build` does
not build `core` or `web-backend`.** For a full build:

```sh
pnpm build:core && pnpm build:web-backend && pnpm build
# or, for the Rust half in one go:
cargo build --release --workspace
```

`--if-present` means a package without a `build` script is skipped silently
rather than failing the run, so this stays correct as `apps/` grows.

## Components not yet scaffolded

`apps/desktop` and `apps/mobile` do not exist yet. Their scripts are already
wired up, and running one today prints:

```
No projects matched the filters in "<repo root>"
```

**This exits 0 — it is a silent no-op, not an error.** Don't read a passing
exit code from `pnpm build:desktop` as "the desktop app built". The same
applies to `pnpm build` and `pnpm test`: they succeed while doing nothing for
components that don't exist. Once the apps are scaffolded, the commands start
working with no changes to the root `package.json`.

## Desktop local testing

```sh
pnpm --filter desktop tauri dev
```

Launches the desktop app in dev mode with hot reload. This is the primary
iteration loop, and it is **not** the same as `pnpm build:desktop`, which
produces installable platform packages and is much slower. Use `tauri dev`
while working; use `build:desktop` when you need an artifact.

This relies on the desktop package exposing a `"tauri": "tauri"` script, which
Tauri's own scaffold generates — verify it exists rather than adding a second one.

## Docker containers

Containers are built through Docker Compose, **not** the pnpm scripts above.

```sh
# Build both images from local source
docker compose -f docker-compose.local.yml build

# Run published images from GHCR instead of building
docker compose up
```

`docker compose up` uses `docker-compose.yml` and needs **no `.env` file** —
every variable has a working default, so an unmodified Compose file boots. Copy
`.env.example` to `.env` only to override the image namespace, pin tags, or
change runtime defaults. See
[`adr/0004-zero-config-startup.md`](./adr/0004-zero-config-startup.md).

Both Compose files mount a `loom-data` volume at `/data`, where the backend will
persist generated secrets and instance config once that system exists.

Note that the frontend image bakes `VITE_API_URL` in at build time; see
`apps/web-frontend/Dockerfile`.

## Testing

```sh
pnpm test      # === pnpm -r --if-present test
```

Runs each frontend package's own `test` script recursively — `web-frontend`
today, plus `desktop` and `mobile` once they exist. No unit tests exist yet, so
`web-frontend`'s `test` is currently a correctness gate of `tsc --noEmit &&
eslint .`; real tests join that script as they are written.

**This is frontend-only.** Rust tests run separately:

```sh
cargo test --workspace
```

CI runs the full Rust gate — `cargo fmt --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, and `cargo test --workspace`. See
[`AGENT_INSTRUCTIONS.md`](./AGENT_INSTRUCTIONS.md).

## Version bumps

See [`VERSIONING.md`](./VERSIONING.md) for how to bump and sync versions across
components.
