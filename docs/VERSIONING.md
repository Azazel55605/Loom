# Versioning

[`versions.json`](../versions.json) at the repo root is the **single source of
truth** for every product's version. It has one entry per product:

```json
{
  "core": "0.1.0",
  "web-backend": "0.1.0",
  "web-frontend": "0.1.0",
  "desktop": "0.1.0",
  "mobile": "0.1.0"
}
```

## This is not a monorepo-wide single version

Each entry is tracked **independently**. Bumping `desktop` does not touch
`core`, `mobile`, or anything else — the sync script only writes the manifests
belonging to the product whose entry changed.

That independence is the entire reason this exists. The products ship on their
own cadences: a frontend fix should not force a server version bump, and mobile
in particular does not map cleanly onto anyone else's numbering — app stores
impose their own monotonic build counters (Android `versionCode`, iOS
`CFBundleVersion`) that have no relationship to the semver of the backend it
talks to. A single shared version number would have made those constraints
collide.

> Those platform-specific fields are **not** handled yet. They are integers,
> not semver strings, so they cannot be synced by copying a value. They will get
> explicit handling in the script's target map once the mobile platforms are
> actually scaffolded.

## Bumping a version

1. Edit the product's entry in `versions.json`.
2. Run `pnpm versions:sync`.
3. Commit `versions.json` together with the manifests it changed.

```sh
pnpm versions:sync          # propagate versions.json into every manifest
pnpm versions:sync:check    # verify only; nonzero exit if anything drifted
```

Both are thin wrappers over `node scripts/sync-versions.mjs`, which has no
third-party dependencies and runs from a bare checkout.

**Never hand-edit a version field** in `Cargo.toml`, `package.json`, or
`tauri.conf.json`. Those files are outputs. A manual edit is drift, and CI will
reject it.

## What gets written where

| Product | Files |
| --- | --- |
| `core` | `crates/core/Cargo.toml` → `package.version` |
| `web-backend` | `crates/web-backend/Cargo.toml` → `package.version` |
| `web-frontend` | `apps/web-frontend/package.json` → `version` |
| `desktop` | `apps/desktop/package.json` → `version`, `apps/desktop/src-tauri/Cargo.toml` → `package.version`, `apps/desktop/src-tauri/tauri.conf.json` → `version` |
| `mobile` | `apps/mobile/package.json` → `version`, `apps/mobile/src-tauri/Cargo.toml` → `package.version`, `apps/mobile/src-tauri/tauri.conf.json` → `version` |

The mapping lives in a `TARGETS` object at the top of
[`scripts/sync-versions.mjs`](../scripts/sync-versions.mjs). Adding a product or
a manifest means adding a line there and nothing else.

Most of `apps/` does not exist yet. Missing target files are **skipped
silently** and reported in a one-line summary — they are never an error, in
either mode, so this works before the clients are scaffolded.

## CI

CI runs `pnpm versions:sync:check` and **fails the build on any drift** between
`versions.json` and a manifest that exists. Missing files still don't count as
failures.

Because the crates carry independent versions, they use literal
`version = "x.y.z"` rather than inheriting `version.workspace = true` from the
workspace root — workspace inheritance would force every crate to share one
number, which is exactly what this system exists to avoid. The sync script
errors out if it finds a crate that has reverted to inheritance.
