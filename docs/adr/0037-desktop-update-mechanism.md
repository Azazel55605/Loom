# ADR 0037: Desktop update mechanism

## Status

Accepted.

## Context

Loom Desktop ships through several distribution systems with different trust
and installation models. Windows installers can use Tauri's signed updater.
The current macOS bundle is unsigned, while Linux packages are expected to be
owned by apt, dnf, an AUR helper, or Flatpak. Treating all of these as though
Loom owned their installation lifecycle would bypass the package manager or
promise an update path the package cannot safely provide.

Published desktop releases are assembled as GitHub drafts. A draft must remain
undiscoverable until every platform and packaging validation job succeeds.

## Decision

- Windows uses `tauri-plugin-updater` for an explicit check, download, and
  install flow. The user starts the download and separately confirms the
  install/restart boundary. Tauri validates every update using the public key
  embedded in the application; CI receives the private key and password only
  through `TAURI_SIGNING_PRIVATE_KEY` and
  `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` repository secrets.
- The updater endpoint, public key, Windows install mode, and
  `bundle.createUpdaterArtifacts` setting live in
  `tauri.windows.conf.json`. Tauri merges that override only for Windows
  targets, so local Linux builds and third-party packaging builds never need
  the updater signing key. The Rust-side updater plugin is also registered only
  on Windows: with no platform configuration, Tauri supplies a null plugin
  value that the updater cannot deserialize and non-Windows applications would
  panic during startup. Linux and macOS use the frontend's check-only release
  manifest path and do not need the native updater plugin. Its Tauri capability
  grant is likewise isolated in a Windows-only capability file so non-Windows
  builds never attempt to resolve a permission for an absent plugin.
- The release workflow generates `latest.json` only from the Windows job and
  prefers the NSIS artifact when both NSIS and MSI installers are present. Once
  every build and packaging validation succeeds and the versioned release is
  published, the workflow copies that manifest to the fixed, prerelease-marked
  `desktop-updater` metadata release. This provides a stable endpoint without
  moving or duplicating the signed installer itself.
- macOS and non-Flatpak Linux fetch the same published `latest.json` only to
  compare its version with the running application. They never invoke the
  updater's download or install APIs. macOS links to the release; Linux tells
  the user to update through their package manager and also offers the release
  link when a newer version exists.
- Flatpak is detected at runtime through `FLATPAK_ID`. It performs the same
  check-only comparison but points users exclusively to `flatpak update`, with
  no GitHub download link.
- Every platform performs one silent check on launch. An available release is
  represented by a small badge beside the Desktop version; installation is
  never automatic.
- The primary updater endpoint is
  `releases/download/desktop-updater/latest.json`, with GitHub's
  `releases/latest/download/latest.json` retained as a fallback for repositories
  that have not created the channel yet. GitHub deliberately excludes
  prereleases from `/releases/latest`; the fixed channel therefore allows Loom's
  intentionally published prereleases to be discovered. Draft releases remain
  undiscoverable because the channel is refreshed only after the versioned
  release is published.

## Consequences

Windows users get a signature-verified in-app update path without background
installation. Other packages keep their native ownership and remediation
instructions. Only Windows builds embed the public verification key or create
signed updater artifacts. Arch PKGBUILD, Flatpak, and ordinary non-Windows
developer builds remain independent of Loom's updater signing credentials.

The signing private key is a permanent compatibility boundary. If it is lost,
a new keypair must be generated and embedded in a newly distributed build;
existing installations cannot authenticate updates signed by that replacement
key and have no automatic recovery path.

This is ADR 0037 rather than 0025 because ADR 0025 already records the feature
capability registration decision and historical ADR numbers are immutable.
