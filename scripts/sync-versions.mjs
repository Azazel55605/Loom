#!/usr/bin/env node
// Propagate versions.json into every per-product manifest.
//
// versions.json is the single source of truth. Each product tracks its version
// independently — bumping one must never touch another. See docs/VERSIONING.md.
//
//   node scripts/sync-versions.mjs           write versions into manifests
//   node scripts/sync-versions.mjs --check   verify only, nonzero exit on drift
//
// Zero dependencies on purpose: this runs in CI before any package install, so
// it must work from a bare checkout with nothing but Node.

import { readFileSync, writeFileSync, existsSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { dirname, join, relative } from "node:path";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");

// Product -> the manifests that carry its version.
//
// `kind` selects the reader/writer: "cargo" edits the `version` key of a
// Cargo.toml `[package]` section; "json" edits a top-level JSON key given by
// `field`. Adding a product or a file means adding a line here and nothing else.
//
// NOTE: Android `versionCode` and iOS `CFBundleVersion` are a separate concern —
// they are monotonically increasing integers, not semver, so they cannot be
// derived from versions.json by copying a string. They will need their own entry
// kind (deriving an integer from the semver, or tracking a build counter) once
// the mobile platforms are actually scaffolded. Not solved here.
const TARGETS = {
  core: [{ path: "crates/core/Cargo.toml", kind: "cargo" }],
  "web-backend": [{ path: "crates/web-backend/Cargo.toml", kind: "cargo" }],
  "web-frontend": [
    { path: "apps/web-frontend/package.json", kind: "json", field: "version" },
  ],
  desktop: [
    { path: "apps/desktop/package.json", kind: "json", field: "version" },
    { path: "apps/desktop/src-tauri/Cargo.toml", kind: "cargo" },
    { path: "apps/desktop/src-tauri/tauri.conf.json", kind: "json", field: "version" },
  ],
  mobile: [
    { path: "apps/mobile/package.json", kind: "json", field: "version" },
    { path: "apps/mobile/src-tauri/Cargo.toml", kind: "cargo" },
    { path: "apps/mobile/src-tauri/tauri.conf.json", kind: "json", field: "version" },
  ],
};

// --- Cargo.toml -------------------------------------------------------------
//
// Deliberately a targeted line edit rather than a TOML parse/serialize round
// trip: re-emitting the file would drop comments and normalise formatting that
// humans maintain. We only ever touch the `version = "..."` line inside
// `[package]`, and refuse to guess if it isn't exactly where we expect.

const PACKAGE_VERSION_LINE = /^(\s*version\s*=\s*)(["'])([^"']*)\2(.*)$/;

/** Locate the `version` line within the `[package]` section. */
function findCargoVersionLine(lines, file) {
  let section = null;
  for (let i = 0; i < lines.length; i++) {
    const header = lines[i].match(/^\s*\[([^\]]+)\]/);
    if (header) {
      section = header[1].trim();
      continue;
    }
    if (section !== "package") continue;
    const m = lines[i].match(PACKAGE_VERSION_LINE);
    if (m) return { index: i, current: m[3], match: m };
  }
  // A crate inheriting `version.workspace = true` cannot be versioned per
  // product, which is the whole point of this file. Fail loudly rather than
  // silently leaving it unmanaged.
  if (lines.some((l) => /^\s*version\.workspace\s*=/.test(l))) {
    throw new Error(
      `${file}: uses \`version.workspace = true\`; give it a literal ` +
        `version = "x.y.z" so it can be tracked independently.`,
    );
  }
  throw new Error(`${file}: no \`version\` key found in a [package] section.`);
}

function readCargoVersion(file) {
  const lines = readFileSync(file, "utf8").split("\n");
  return findCargoVersionLine(lines, file).current;
}

function writeCargoVersion(file, version) {
  const text = readFileSync(file, "utf8");
  const lines = text.split("\n");
  const { index, match } = findCargoVersionLine(lines, file);
  lines[index] = `${match[1]}"${version}"${match[4]}`;
  writeFileSync(file, lines.join("\n"));
}

// --- JSON manifests ---------------------------------------------------------
//
// Preserve the file's existing indentation so syncing never shows up as a
// whitespace-only diff.

function detectIndent(text) {
  const m = text.match(/\n([ \t]+)\S/);
  return m ? m[1] : "  ";
}

function readJsonField(file, field) {
  const value = JSON.parse(readFileSync(file, "utf8"))[field];
  return typeof value === "string" ? value : undefined;
}

function writeJsonField(file, field, version) {
  const text = readFileSync(file, "utf8");
  const data = JSON.parse(text);
  data[field] = version;
  const trailingNewline = text.endsWith("\n") ? "\n" : "";
  writeFileSync(file, JSON.stringify(data, null, detectIndent(text)) + trailingNewline);
}

// --- driver -----------------------------------------------------------------

function readVersion(target, file) {
  return target.kind === "cargo"
    ? readCargoVersion(file)
    : readJsonField(file, target.field);
}

function writeVersion(target, file, version) {
  if (target.kind === "cargo") writeCargoVersion(file, version);
  else writeJsonField(file, target.field, version);
}

function main() {
  const check = process.argv.includes("--check");
  const versionsPath = join(REPO_ROOT, "versions.json");
  const versions = JSON.parse(readFileSync(versionsPath, "utf8"));

  console.log(check ? "Checking versions.json:" : "Syncing versions.json:");
  const width = Math.max(...Object.keys(versions).map((k) => k.length));
  for (const [product, version] of Object.entries(versions)) {
    console.log(`  ${product.padEnd(width)}  ${version}`);
  }
  console.log("");

  const skipped = [];
  const mismatches = [];
  let updated = 0;
  let unchanged = 0;

  for (const [product, version] of Object.entries(versions)) {
    const targets = TARGETS[product];
    if (!targets) {
      console.error(`  ! ${product}: no target files mapped in this script`);
      process.exitCode = 1;
      continue;
    }

    for (const target of targets) {
      const file = join(REPO_ROOT, target.path);
      // apps/ is largely unscaffolded; a missing target is expected, never fatal.
      if (!existsSync(file)) {
        skipped.push(target.path);
        continue;
      }

      const current = readVersion(target, file);
      const label = `${relative(REPO_ROOT, file)}`;

      if (current === version) {
        unchanged++;
        console.log(`  unchanged  ${label}  (${version})`);
      } else if (check) {
        mismatches.push({ path: target.path, current, expected: version });
        console.log(`  MISMATCH   ${label}  ${current ?? "<unset>"} != ${version}`);
      } else {
        writeVersion(target, file, version);
        updated++;
        console.log(`  updated    ${label}  ${current ?? "<unset>"} -> ${version}`);
      }
    }
  }

  console.log("");
  if (check) {
    if (mismatches.length > 0) {
      console.error(
        `${mismatches.length} file(s) out of sync with versions.json. ` +
          `Run \`pnpm versions:sync\` and commit the result.`,
      );
      process.exitCode = 1;
    } else {
      console.log(`In sync: ${unchanged} file(s) match versions.json.`);
    }
  } else {
    console.log(`Done: ${updated} updated, ${unchanged} unchanged.`);
  }

  if (skipped.length > 0) {
    console.log(`Not yet scaffolded (${skipped.length} skipped): ${skipped.join(", ")}`);
  }
}

try {
  main();
} catch (err) {
  console.error(`sync-versions: ${err.message}`);
  process.exitCode = 1;
}
