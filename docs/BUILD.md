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
| Tauri system dependencies | desktop and mobile builds | OS-specific (webview, build tools). Follow [Tauri's prerequisites guide](https://tauri.app/start/prerequisites/) — deliberately not duplicated here, since per-OS package lists go stale quickly. |
| JDK 21 + Android SDK/NDK | mobile builds only | Needed by Gradle and Tauri's Android tooling; see [Mobile specifics](#mobile-specifics). Not required for any other component. |

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
| mobile | `pnpm build:mobile` | Unsigned APK + AAB via Gradle, under `apps/mobile/src-tauri/gen/android/app/build/outputs/` |

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

> Every component is now scaffolded, so `pnpm -r` commands no longer skip
> anything. If a future `apps/*` package has no `build` or `test` script,
> `--if-present` still skips it silently — a passing exit code is not by itself
> proof that a given component built.

## Desktop specifics

`pnpm build:desktop` runs `tauri build`, which compiles the Rust shell and emits
platform bundles under `apps/desktop/src-tauri/target/release/bundle/`. It needs
Tauri's system dependencies (see Prerequisites) — on Debian/Ubuntu that is
`libwebkit2gtk-4.1-dev`, `librsvg2-dev`, `libxdo-dev` and `libssl-dev`.

Bundle targets are `deb`, `rpm`, `msi`, `nsis`, `dmg` and `app`; Tauri keeps
whichever are valid for the host OS. **AppImage is deliberately not built** — it
is more trouble than it is worth, and the deb, rpm, Arch and Flatpak packages
cover Linux.

`apps/desktop/src-tauri` is deliberately **its own Cargo workspace**, detached
from the root one, so `cargo build --workspace` and CI's Rust job do not require
those libraries.

### Icons

`apps/desktop/icon.svg` is the source of truth for the app icon. Everything in
`apps/desktop/src-tauri/icons/` (png/ico/icns plus the Android and iOS sets) is
generated from it — never hand-edit those. To regenerate after changing the SVG:

```sh
rsvg-convert -w 1024 -h 1024 apps/desktop/icon.svg -o /tmp/loom-icon.png
pnpm --filter desktop tauri icon /tmp/loom-icon.png
```

Packaging beyond Tauri's own bundles lives in `packaging/` — Arch PKGBUILDs and
a Flatpak manifest, both exercised by `.github/workflows/release-desktop.yml`.
macOS builds are unsigned; see
[`DESKTOP_MACOS_UNSIGNED.md`](./DESKTOP_MACOS_UNSIGNED.md).

## Mobile specifics

**Android only — iOS is out of scope.**

```sh
pnpm build:mobile      # === pnpm --filter mobile tauri android build
```

Output lands in `apps/mobile/src-tauri/gen/android/app/build/outputs/`:
`apk/universal/<variant>/*.apk` and `bundle/universal<Variant>/*.aab`.

### Extra prerequisites

Beyond the shared Tauri dependencies, mobile needs:

- **JDK 21** (Gradle / Android Gradle Plugin).
- **Android SDK** with a platform and build-tools, plus the **NDK**.
- The four Rust Android targets:

  ```sh
  rustup target add aarch64-linux-android armv7-linux-androideabi \
      i686-linux-android x86_64-linux-android
  ```

- `ANDROID_HOME`, `NDK_HOME` and `JAVA_HOME` exported. Point them at your own
  SDK install — paths are machine-specific and intentionally not committed
  anywhere in this repo.

Without an Android SDK/NDK, `pnpm build:mobile` cannot run at all; nothing else
in the repo needs it.

### The Gradle project is generated, not committed

`apps/mobile/src-tauri/gen/` is gitignored. A fresh checkout has no Android
project until you generate one:

```sh
pnpm --filter mobile tauri android init
```

This derives `applicationId`, `versionCode` and `versionName` from
`tauri.conf.json`, which is why that file is the source of truth and the
generated Gradle files must never be hand-edited. See
[`VERSIONING.md`](./VERSIONING.md). CI runs the same `init` step for the same
reason.

### Signing

Debug builds are signed with Gradle's debug keystore and are **not** release
artifacts. **Release signing is not implemented** — no keystore, no Play Store
publishing. It is a deliberate follow-up, mirroring how desktop code-signing was
deferred, and needs a keystore in GitHub Secrets plus a Gradle signing config.
`.github/workflows/release-mobile.yml` therefore builds a debug APK only.

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
# Build both images from local source (tagged loom-*:local)
docker compose -f docker-compose.local.yml build

# Run published images from GHCR instead of building
docker compose up
```

`docker compose up` uses `docker-compose.yml` and needs **no `.env` file** —
every variable has a working default, so an unmodified Compose file boots. Copy
`.env.example` to `.env` only to override the image namespace, pin tags, or
change runtime defaults. See
[`adr/0004-zero-config-startup.md`](./adr/0004-zero-config-startup.md).

Locally built images are tagged `loom-web-backend:local` and
`loom-web-frontend:local`, so they never collide with the `:latest` images
`docker-compose.yml` pulls from GHCR.

Both Compose files mount a `loom-data` volume at `/data`, where the backend will
persist generated secrets and instance config once that system exists.

The frontend serves the API under its own origin: nginx proxies `/api` to
`LOOM_BACKEND_ORIGIN` (default `http://web-backend:8080`), so the browser never
makes a cross-origin request and no backend host is compiled into the bundle.
One image works at any hostname. See
[`adr/0006-frontend-api-same-origin.md`](./adr/0006-frontend-api-same-origin.md).

`pnpm dev` proxies `/api` the same way, targeting `http://localhost:8080` unless
`LOOM_BACKEND_ORIGIN` says otherwise.

## Cleaning build output

```sh
pnpm clean          # build artifacts from every component
pnpm clean:dry      # show what would go, delete nothing
pnpm clean:deep     # the above, plus generated projects and node_modules
```

Build output gets large fast — the two Tauri `target/` directories and the
Android Gradle output dominate, and a full local build of everything runs to
well over 10 GB.

`cargo clean` on its own is not enough. This repo contains **three separate
Cargo workspaces** — the root one, plus a detached workspace inside each Tauri
app — so a single `cargo clean` at the root leaves the desktop and mobile
targets untouched. The Gradle output under `gen/android` is not Cargo's at all
and is usually the third-largest item.

`pnpm clean` removes:

| | |
| --- | --- |
| `target/`, `apps/desktop/src-tauri/target/`, `apps/mobile/src-tauri/target/` | via `cargo clean` per workspace, falling back to deletion if Cargo is unavailable |
| `apps/*/dist/` | Vite output |
| `gen/android/{build,app/build,app/.cxx,.gradle}` | Gradle output |

It leaves `node_modules` and the generated `gen/android` project in place, so
the next build only recompiles — it does not re-download or re-scaffold.

`pnpm clean:deep` additionally removes `node_modules` and `src-tauri/gen/`.
After that you need `pnpm install` again, and
`pnpm --filter mobile tauri android init` (which requires the Android SDK)
before the next Android build. The script prints this reminder.

Reported sizes are apparent sizes; because Cargo hardlinks heavily, `du` may
show a smaller figure than the total reclaimed.

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
