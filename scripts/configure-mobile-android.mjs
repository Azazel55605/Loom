import { copyFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const repositoryRoot = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const mobileRoot = resolve(repositoryRoot, "apps/mobile");
const manifestPath = resolve(
  mobileRoot,
  "src-tauri/gen/android/app/src/main/AndroidManifest.xml",
);
const generatedPolicyPath = resolve(
  mobileRoot,
  "src-tauri/gen/android/app/src/main/res/xml/network_security_config.xml",
);
const policySourcePath = resolve(
  mobileRoot,
  "android/network_security_config.xml",
);

let manifest;
try {
  manifest = await readFile(manifestPath, "utf8");
} catch (error) {
  if (error instanceof Error && "code" in error && error.code === "ENOENT") {
    throw new Error(
      "Android project is missing; run `pnpm --filter mobile tauri android init` first.",
    );
  }
  throw error;
}

if (!manifest.includes("android:networkSecurityConfig=")) {
  manifest = manifest.replace(
    /<application\b/,
    '<application\n        android:networkSecurityConfig="@xml/network_security_config"',
  );
  await writeFile(manifestPath, manifest, "utf8");
}

await mkdir(dirname(generatedPolicyPath), { recursive: true });
await copyFile(policySourcePath, generatedPolicyPath);

console.log("Applied Loom's Android network security policy.");
