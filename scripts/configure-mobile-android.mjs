import { copyFile, mkdir, readFile, readdir, writeFile } from "node:fs/promises";
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
const iconSourceDirectory = resolve(mobileRoot, "src-tauri/icons/android");
const generatedResourcesDirectory = resolve(
  mobileRoot,
  "src-tauri/gen/android/app/src/main/res",
);

async function copyDirectoryContents(sourceDirectory, targetDirectory) {
  await mkdir(targetDirectory, { recursive: true });
  const entries = await readdir(sourceDirectory, { withFileTypes: true });
  await Promise.all(
    entries.map(async (entry) => {
      const sourcePath = resolve(sourceDirectory, entry.name);
      const targetPath = resolve(targetDirectory, entry.name);
      if (entry.isDirectory()) {
        await copyDirectoryContents(sourcePath, targetPath);
      } else {
        await copyFile(sourcePath, targetPath);
      }
    }),
  );
}

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
await copyDirectoryContents(iconSourceDirectory, generatedResourcesDirectory);

console.log("Applied Loom's Android network security policy and launcher icons.");
