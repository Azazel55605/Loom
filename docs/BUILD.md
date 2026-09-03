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

This installs every workspace package under `apps/` and `packages/`. Rust
dependencies are fetched by Cargo on first build; no separate step.

## Per-component builds

| Component | Command | Output |
| --- | --- | --- |
| core (library) | `pnpm build:core` | Compiled rlib inside `target/release/`. No standalone artifact — Core is a library and never runs on its own. |
| connector-docker (library) | `pnpm build:connector-docker` | Compiled rlib inside `target/release/`. Like Core, a library: it is linked into web-backend, which is what decides that this build has a Docker connector in it. |
| connector-pihole | `pnpm build:connector-pihole` | Compiled rlib inside `target/release/`. Pi-hole v6 session-authenticated REST client and host-level connector. |
| connector-truenas | `pnpm build:connector-truenas` | Compiled rlib inside `target/release/`. TLS-only JSON-RPC transport plus the minimal host-level TrueNAS connector. |
| web-backend | `pnpm build:web-backend` | `target/release/loom-web-backend` binary |
| web-frontend | `pnpm build:web-frontend` | `apps/web-frontend/dist/` static site |
| desktop | `pnpm build:desktop` | Platform installers in `apps/desktop/src-tauri/target/release/bundle/` |
| mobile | `pnpm build:mobile` | Unsigned arm64 debug APK under `apps/mobile/src-tauri/gen/android/app/build/outputs/` |

The Cargo packages are named `loom-core`, `loom-connector-docker`,
`loom-connector-pihole`, `loom-connector-truenas`, and `loom-web-backend` — a crate named `core` would
collide with Rust's built-in `core`, and the `loom-` prefix is carried through for consistency. The
`pnpm build:*` scripts already use the correct names; prefer them over
hand-written `cargo` commands.

Every real connector is its own crate, and adding one adds a row here — see
[`adr/0017`](./adr/0017-connector-crates-and-async-factories.md).

### Docker sub-target migration note

The Docker connector now models one daemon connection as one connector
instance, with containers selected as placement sub-targets. There is no
automatic migration from pre-release per-container `docker-container` or
container-mode `docker` instances. Delete and recreate those instances and
their placements against one host-level `docker` instance. A stale
`containerName` key in stored configuration is ignored safely while the row is
being recreated; it does not select a container under the new model.

For the LinuxServer TCP Docker socket proxy, Loom needs ping/version and
container access, plus the separate logs gate for the logs data point. Each
container action uses its corresponding start, stop, restart, pause, or
unpause gate; those narrow gates intentionally work while the broad `POST`
permission remains disabled. The optional info and system gates complete the
host summary. Denying the system disk-usage call leaves the connector usable
but Degraded, with the refusal reported in its status; it does not make
connector creation or actions wait for a full inventory.

### `pnpm build` builds frontend apps and shared packages only

```sh
pnpm build     # === pnpm -r --if-present build
```

`pnpm -r` iterates **pnpm workspace packages** — `apps/*` and `packages/*`.
The Rust crates are Cargo workspace members, not pnpm packages, so **`pnpm
build` does not build `core` or `web-backend`.** For a full build:

```sh
pnpm build:core && pnpm build:web-backend && pnpm build
# or, for the Rust half in one go:
cargo build --release --workspace
```

`--if-present` means a package without a `build` script is skipped silently
rather than failing the run, so this stays correct as `apps/` grows.

> Every component is now scaffolded, so `pnpm -r` commands no longer skip
> anything. If a future `apps/*` or `packages/*` package has no `build` or `test` script,
> `--if-present` still skips it silently — a passing exit code is not by itself
> proof that a given component built.

## Running the web frame locally

Two processes, in two terminals:

```sh
# 1. Backend. Creates ./data/loom.db and migrates it on first run.
cargo run --package loom-web-backend

# 2. Frontend.
pnpm --filter web-frontend dev
```

No flags, no environment, no database to provision — per
[ADR 0004](./adr/0004-zero-config-startup.md). The backend logs the database
path it resolved at startup; if that line is not what you expect, check
`LOOM_DATA_DIR`.

Then open **`http://localhost:3000`**.

**On a fresh database you land on `/setup`, not `/login`.** That is expected:
the backend reports `setupComplete: false`, and setup outranks authentication
because an instance with no administrator has nothing to log in against. Create
the administrator — the password must be at least 8 characters — and you are
sent on to the login screen. Sign in with those credentials and you land on a
dashboard with no connectors on it. Connectors are added rather than shipped:
`GET /connector-types` lists what this build can create (today, the
`DebugConnector` fixture) and `POST /connector-instances` adds one.

Unlike the removed stub, **setup now persists**: it runs once per database, not
once per backend start. Restarting the backend keeps you set up and keeps you
signed in.

### Data directory permissions in containers

The backend runs unprivileged in its image (uid 10001), and its database lives
on the `loom-data` volume at `/data`. The image creates `/data` **owned by that
user**, which is what makes the volume writable: Docker seeds a new named volume
from the image directory, ownership included, but when the mount path does not
exist in the image it creates the mountpoint as `root:root` instead — and the
server then cannot create `loom.db`.

If you see this at startup:

```
Error: the data directory /data is not writable by this process ...
```

the volume predates that fix and kept its old ownership — Docker only seeds a
volume that is empty. Recreate it:

```sh
docker compose -f docker-compose.local.yml down -v
```

`-v` deletes the volume, and with it every account and session. That is the
intent here; on a real deployment it is not something to run casually.

### Starting over

Setup cannot be re-run against a database that already has a user — that is the
point of the 409. To get a clean first-run experience, delete the database:

```sh
rm -rf ./data          # or "$LOOM_DATA_DIR"
```

The next start recreates and re-migrates it. This is also the fastest way to
recover from a forgotten administrator password, since there is no password
reset yet.

The frontend holds an access token (15 minutes) and a refresh token (7 days)
and renews the pair silently, so a signed-in session survives a reload and does
not interrupt you every quarter of an hour. Signing out revokes the refresh
token on the backend rather than only forgetting it locally.

> **This build authenticates but does not yet authorize.** Logins are real and
> passwords are hashed with argon2id, but no middleware checks permissions, so
> the connector routes are reachable by anyone who can reach the port. Do not
> put an instance somewhere untrusted on the assumption that logging in is
> required to use it. See
> [`API_CONTRACT.md`](./API_CONTRACT.md#known-temporary-behavior).

### How the frontend reaches the backend

The browser requests `/api/*` on the frontend's own origin; the Vite dev server
proxies that to the backend and strips the `/api` prefix, exactly as the
production nginx does. So `/api/auth/login` in the browser arrives at the
backend as `/auth/login`, and the backend's own paths carry no `/api` prefix.
See [ADR 0006](./adr/0006-frontend-api-same-origin.md) and the path table in
[`API_CONTRACT.md`](./API_CONTRACT.md).

Nothing needs configuring for the default case. Two escape hatches exist:

The dev server uses port **3000**, the same port Compose publishes the frontend
on, so the app is at one URL regardless of how it is running. The backend is on
**8080** either way. Those are the only two ports involved; `strictPort` makes
the dev server fail loudly rather than drift to 3001 if something already holds
3000.

| Variable | Where it is read | Effect |
| --- | --- | --- |
| `LOOM_BACKEND_ORIGIN` | Vite dev server, nginx | Proxy target. Defaults to `http://localhost:8080` in dev. Set it when the backend is not on the default port. |
| `VITE_API_URL` | Frontend bundle, at build time | Absolute API base, bypassing the proxy entirely. Makes requests cross-origin and therefore subject to the CORS policy in [ADR 0010](./adr/0010-desktop-secure-storage-and-network-config.md). For deployments without the proxy; not needed for local development. |

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

Desktop persists authentication tokens in the operating system credential
store through `tauri-plugin-keyring-store`: macOS Keychain, Windows Credential
Manager, or Linux Secret Service. The plugin's Linux backend is Rust-native and
adds no OpenSSL/libsecret build dependency, but a Secret Service provider such
as GNOME Keyring or KWallet must be running for login persistence to work.
Headless Linux sessions commonly have no unlocked service; builds and frontend
checks still work there, while live secure-storage verification requires a
normal logged-in desktop session.

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
pnpm build:mobile      # arm64 phone/tablet APK
```

Output lands in `apps/mobile/src-tauri/gen/android/app/build/outputs/`:
the APK is under `apk/universal/debug/` (Tauri's variant name), but contains
only the `arm64-v8a` native library. Debug builds deliberately do not also
generate an AAB.

The default does not build a universal APK. A universal debug APK duplicated
the Rust shared library for several ABIs and could exceed 500 MB. Loom supports
arm64 Android phones and tablets only; Chromebook and x86 emulator APKs are out
of scope. The Cargo dev profile also strips native DWARF from the packaged
library while retaining debug assertions, overflow checks, and the unoptimised
debug profile. For the uncommon case where source-level Rust debugging is
required, restore full symbols for that invocation:

```sh
CARGO_PROFILE_DEV_STRIP=none pnpm build:mobile
```

That symbol-bearing APK is expected to be much larger. JavaScript/WebView
debugging and ordinary `adb logcat` use do not require the removed DWARF data.

### Extra prerequisites

Beyond the shared Tauri dependencies, mobile needs:

- **JDK 21** (Gradle / Android Gradle Plugin).
- **Android SDK** with a platform and build-tools, plus the **NDK**.
- The arm64 Rust Android target:

  ```sh
  rustup target add aarch64-linux-android
  ```

- `ANDROID_HOME` and `NDK_HOME` exported. Point them at your own SDK install —
  paths are machine-specific and intentionally not committed anywhere in this
  repo. `pnpm build:mobile` automatically selects an installed JDK 21 and sets
  `JAVA_HOME` plus `PATH` for the Tauri build subprocess. An already-correct
  `JAVA_HOME` takes precedence; otherwise standard OS Java locations are
  searched. Set `JAVA_HOME` yourself only when the JDK is installed elsewhere.

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

The generated project also needs Loom's runtime network policy and launcher
icons. Their canonical sources are `apps/mobile/android/network_security_config.xml`
and `apps/mobile/src-tauri/icons/android/`; apply them after every init (the
standard build and CI do this automatically):

```sh
pnpm --filter mobile android:configure
```

The configure step installs Loom's legacy, round, and adaptive launcher icons
and removes the unused Android/Tauri template artwork. It also applies Android's
four-way `fullUser` orientation mode while respecting the device rotation-lock
setting. Finally, it removes the previous generated debug APK before packaging;
otherwise repeated incremental Gradle builds can retain an unreferenced copy of
the native library and nearly double the artifact size. Android's version code
must be incremented through `versions.json` for each distributed APK so package
installers do not reuse metadata cached for an older build.

The policy permits HTTP because a user-selected homelab server may not have
TLS, and trusts both system CAs and CAs the device owner explicitly installed.
It does not bypass certificate or hostname verification. Prefer HTTPS whenever
the server supports it.

### Emulator and device testing

An Android emulator reaches a backend running on its host through
`http://10.0.2.2:8080`; `localhost` inside the emulator is the emulator itself.
A physical device needs an address it can reach on the same network. Complete
setup, login, dashboard, and Settings flows against that server, then restart
the app and confirm the server selection and login survive.

For a debug install, inspect the app-private data with `adb shell run-as
dev.loom.mobile`. The settings file may contain the server URL and Stronghold
bootstrap material, but must not contain `accessToken` or `refreshToken`.
Those values belong only in the encrypted `loom-mobile.hold` snapshot.

### Signing

Debug builds are signed with Gradle's debug keystore and are **not** release
artifacts. **Release signing is not implemented** — no keystore, no Play Store
publishing. It is a deliberate follow-up, mirroring how desktop code-signing was
deferred, and needs a keystore in GitHub Secrets plus a Gradle signing config.
`.github/workflows/release-mobile.yml` therefore builds the same arm64 debug
APK only, avoiding an oversized multi-ABI artifact.

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

**Some Rust tests need a Docker daemon.** `crates/connector-docker` has
integration tests that create a real throwaway container and point the connector
at it. If no daemon is reachable they **print why and pass** rather than
failing, so a checkout with no Docker still runs a clean `cargo test
--workspace`; GitHub-hosted runners do have one, so they run for real in CI.
Use `--nocapture` to see which ran and which were skipped:

```sh
cargo test -p loom-connector-docker -- --nocapture
```

Point them elsewhere — or at a deliberately dead endpoint, to check the skip
path still works — with `LOOM_TEST_DOCKER_HOST`. It is read only by those tests;
the connector itself never reads the environment.

CI runs the full Rust gate — `cargo fmt --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, and `cargo test --workspace`. See
[`AGENT_INSTRUCTIONS.md`](./AGENT_INSTRUCTIONS.md).

## Version bumps

See [`VERSIONING.md`](./VERSIONING.md) for how to bump and sync versions across
components.
