import { readdirSync } from "node:fs";
import { delimiter, join } from "node:path";
import { spawnSync } from "node:child_process";

const requestedMajor = 21;
const javaExecutable = process.platform === "win32" ? "java.exe" : "java";

function javaMajor(javaPath) {
  const result = spawnSync(javaPath, ["-version"], {
    encoding: "utf8",
    windowsHide: true,
  });
  const output = `${result.stdout ?? ""}\n${result.stderr ?? ""}`;
  const match = output.match(/version\s+"(?:1\.)?(\d+)/i);
  return match === null ? null : Number.parseInt(match[1], 10);
}

function javaHomeFromExecutable(javaPath) {
  const result = spawnSync(javaPath, ["-XshowSettings:properties", "-version"], {
    encoding: "utf8",
    windowsHide: true,
  });
  const output = `${result.stdout ?? ""}\n${result.stderr ?? ""}`;
  return output.match(/^\s*java\.home\s*=\s*(.+)$/m)?.[1]?.trim() ?? null;
}

function childDirectories(parent) {
  try {
    return readdirSync(parent, { withFileTypes: true })
      .filter((entry) => entry.isDirectory() || entry.isSymbolicLink())
      .map((entry) => join(parent, entry.name));
  } catch {
    return [];
  }
}

function candidateHomes() {
  const candidates = [];
  if (process.env.JAVA_HOME) candidates.push(process.env.JAVA_HOME);

  const activeHome = javaHomeFromExecutable(javaExecutable);
  if (activeHome) candidates.push(activeHome);

  if (process.platform === "darwin") {
    const result = spawnSync("/usr/libexec/java_home", ["-v", String(requestedMajor)], {
      encoding: "utf8",
      windowsHide: true,
    });
    if (result.status === 0 && result.stdout.trim()) {
      candidates.push(result.stdout.trim());
    }
    for (const bundle of childDirectories("/Library/Java/JavaVirtualMachines")) {
      candidates.push(join(bundle, "Contents", "Home"));
    }
  } else if (process.platform === "win32") {
    for (const parent of [
      join(process.env.ProgramFiles ?? "C:\\Program Files", "Java"),
      join(process.env.ProgramFiles ?? "C:\\Program Files", "Eclipse Adoptium"),
    ]) {
      candidates.push(...childDirectories(parent));
    }
  } else {
    for (const parent of ["/usr/lib/jvm", "/usr/java", "/opt/java"]) {
      candidates.push(...childDirectories(parent));
    }
  }

  if (process.env.SDKMAN_DIR) {
    candidates.push(...childDirectories(join(process.env.SDKMAN_DIR, "candidates", "java")));
  }

  return [...new Set(candidates)];
}

function findJavaHome() {
  for (const home of candidateHomes()) {
    if (javaMajor(join(home, "bin", javaExecutable)) === requestedMajor) {
      return home;
    }
  }
  return null;
}

const commandArguments = process.argv.slice(2);
if (commandArguments.length === 0) {
  console.error("Usage: node scripts/run-with-java-21.mjs <command> [arguments...]");
  process.exit(2);
}

const javaHome = findJavaHome();
if (javaHome === null) {
  console.error(
    "JDK 21 was not found. Install it or set JAVA_HOME to its installation directory.",
  );
  process.exit(1);
}

console.log(`Using JDK 21 from ${javaHome}`);

const [command, ...args] = commandArguments;
const executable =
  process.platform === "win32" && command === "pnpm" ? "pnpm.cmd" : command;
const result = spawnSync(executable, args, {
  env: {
    ...process.env,
    JAVA_HOME: javaHome,
    PATH: `${join(javaHome, "bin")}${delimiter}${process.env.PATH ?? ""}`,
  },
  stdio: "inherit",
  windowsHide: false,
});

if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 1);
