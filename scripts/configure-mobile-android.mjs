import {
  copyFile,
  mkdir,
  readFile,
  readdir,
  rm,
  writeFile,
} from "node:fs/promises";
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
const generatedTemplateIconPaths = [
  resolve(generatedResourcesDirectory, "drawable/ic_launcher_background.xml"),
  resolve(generatedResourcesDirectory, "drawable-v24/ic_launcher_foreground.xml"),
];
const generatedDebugApkPath = resolve(
  mobileRoot,
  "src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk",
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
}
if (!manifest.includes("android:roundIcon=")) {
  manifest = manifest.replace(
    /<application\b/,
    '<application\n        android:roundIcon="@mipmap/ic_launcher_round"',
  );
}
if (!manifest.includes("android:screenOrientation=")) {
  manifest = manifest.replace(
    /<activity\b/,
    '<activity\n            android:screenOrientation="fullUser"',
  );
}
await writeFile(manifestPath, manifest, "utf8");

await mkdir(dirname(generatedPolicyPath), { recursive: true });
await copyFile(policySourcePath, generatedPolicyPath);
await copyDirectoryContents(iconSourceDirectory, generatedResourcesDirectory);
await Promise.all(
  generatedTemplateIconPaths.map((iconPath) => rm(iconPath, { force: true })),
);
// Gradle's incremental debug packager can leave an unreferenced previous copy
// of the native library in this ZIP. Removing this one generated artifact
// keeps repeated local builds as small as a clean build.
await rm(generatedDebugApkPath, { force: true });

console.log(
  "Applied Loom's Android policy, launcher icons, orientation, and clean APK output.",
);
