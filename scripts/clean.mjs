#!/usr/bin/env node
// Remove build output from every component.
//
//   node scripts/clean.mjs             build artifacts (default)
//   node scripts/clean.mjs --deep      the above, plus generated projects and node_modules
//   node scripts/clean.mjs --dry-run   report what would be removed, delete nothing
//
// `cargo clean` alone is not enough here: the repo holds THREE separate Cargo
// workspaces (the root one, plus a detached workspace under each Tauri app), and
// the Android build's Gradle output is not Cargo's at all. See docs/BUILD.md.
//
// Zero dependencies, and no shell built-ins, so it behaves the same on Windows.

import { existsSync, readdirSync, rmSync, statSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { dirname, join, relative } from "node:path";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

// Every Cargo workspace root. Each owns its own target/ directory.
const CARGO_WORKSPACES = [".", "apps/desktop/src-tauri", "apps/mobile/src-tauri"];

// Build output that is not Cargo's.
const ARTIFACT_DIRS = [
  "apps/web-frontend/dist",
  "apps/desktop/dist",
  "apps/mobile/dist",
  // Gradle. `gen/android` itself is kept unless --deep, since regenerating it
  // needs the Android SDK; only its build output is removed here.
  "apps/mobile/src-tauri/gen/android/build",
  "apps/mobile/src-tauri/gen/android/app/build",
  "apps/mobile/src-tauri/gen/android/app/.cxx",
  "apps/mobile/src-tauri/gen/android/.gradle",
];

// Only with --deep. These cost real time to rebuild: `gen/` needs
// `tauri android init` plus an Android SDK, node_modules needs a pnpm install.
const DEEP_DIRS = [
  "apps/desktop/src-tauri/gen",
  "apps/mobile/src-tauri/gen",
  "node_modules",
  "apps/web-frontend/node_modules",
  "apps/desktop/node_modules",
  "apps/mobile/node_modules",
];

/** Total size of a directory tree, in bytes. Symlinks are counted as 0. */
function dirSize(path) {
  let total = 0;
  const stack = [path];
  while (stack.length > 0) {
    const current = stack.pop();
    let entries;
    try {
      entries = readdirSync(current, { withFileTypes: true });
    } catch {
      continue; // vanished or unreadable — not worth failing the run over
    }
    for (const entry of entries) {
      const full = join(current, entry.name);
      if (entry.isDirectory()) stack.push(full);
      else if (entry.isFile()) {
        try {
          total += statSync(full).size;
        } catch {
          /* ignore */
        }
      }
    }
  }
  return total;
}

function formatBytes(bytes) {
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  return `${value.toFixed(value >= 10 || unit === 0 ? 0 : 1)} ${units[unit]}`;
}

const dryRun = process.argv.includes("--dry-run");
const deep = process.argv.includes("--deep");

let reclaimed = 0;

/** Remove a directory, reporting its size. */
function remove(path, label) {
  const full = join(REPO_ROOT, path);
  if (!existsSync(full)) return;

  const size = dirSize(full);
  reclaimed += size;
  const verb = (dryRun ? "would remove" : "removed").padEnd(13);
  console.log(`  ${verb} ${label ?? path}  (${formatBytes(size)})`);
  if (!dryRun) rmSync(full, { recursive: true, force: true });
}

console.log(dryRun ? "Dry run — nothing will be deleted.\n" : "Cleaning build output.\n");

// Prefer `cargo clean`: it knows about target directories this script does not
// (custom CARGO_TARGET_DIR, for instance). Fall back to deleting target/ when
// Cargo is unavailable, so the script still works without a Rust toolchain.
for (const workspace of CARGO_WORKSPACES) {
  const manifest = join(REPO_ROOT, workspace, "Cargo.toml");
  if (!existsSync(manifest)) continue;

  const targetDir = join(workspace, "target");
  const full = join(REPO_ROOT, targetDir);
  if (!existsSync(full)) continue;

  const size = dirSize(full);
  reclaimed += size;
  const label = `${relative(REPO_ROOT, full) || "target"} (cargo)`;

  if (dryRun) {
    console.log(`  ${"would remove".padEnd(13)} ${label}  (${formatBytes(size)})`);
    continue;
  }

  try {
    execFileSync("cargo", ["clean", "--manifest-path", manifest], { stdio: "ignore" });
    console.log(`  ${"cargo clean".padEnd(13)} ${label}  (${formatBytes(size)})`);
  } catch {
    rmSync(full, { recursive: true, force: true });
    console.log(`  ${"removed".padEnd(13)} ${label}  (${formatBytes(size)})  [cargo unavailable]`);
  }
}

for (const dir of ARTIFACT_DIRS) remove(dir);

if (deep) {
  console.log("");
  for (const dir of DEEP_DIRS) remove(dir);
}

// Apparent size: Cargo hardlinks aggressively, so `du` may report less.
console.log(`\nReclaimed ~${formatBytes(reclaimed)} (apparent size).`);
if (deep) {
  console.log(
    "Run `pnpm install` to restore dependencies, and " +
      "`pnpm --filter mobile tauri android init` before the next Android build.",
  );
}
