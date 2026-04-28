import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  chmodSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  realpathSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "..");
const tauriDir = path.join(repoRoot, "src-tauri");
const baseConfigPath = path.join(tauriDir, "tauri.conf.json");
const generatedConfigPath = path.join(tauriDir, "tauri.generated.conf.json");
const generatedBinDir = path.join(tauriDir, "gen", "bin");
const generatedResourceDir = path.join(tauriDir, "gen", "resources");

function fallbackTargetTriple() {
  const platform = process.platform;
  const arch = process.arch;

  if (platform === "darwin" && arch === "arm64") return "aarch64-apple-darwin";
  if (platform === "darwin" && arch === "x64") return "x86_64-apple-darwin";
  if (platform === "win32" && arch === "x64") return "x86_64-pc-windows-msvc";
  if (platform === "win32" && arch === "arm64") return "aarch64-pc-windows-msvc";
  if (platform === "linux" && arch === "x64") return "x86_64-unknown-linux-gnu";

  throw new Error(`Unsupported build host ${platform}/${arch}. Set RKB_TARGET_TRIPLE explicitly.`);
}

function parseBooleanEnv(name, env = process.env) {
  const value = env[name];
  if (value == null) return null;

  const normalized = String(value).trim().toLowerCase();
  if (["1", "true", "yes", "on"].includes(normalized)) return true;
  if (["0", "false", "no", "off"].includes(normalized)) return false;

  throw new Error(`Unsupported boolean value for ${name}: ${value}`);
}

function detectTargetTriple() {
  if (process.env.RKB_TARGET_TRIPLE) {
    return process.env.RKB_TARGET_TRIPLE;
  }

  try {
    const output = execFileSync("rustc", ["-vV"], {
      cwd: repoRoot,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "ignore"],
    });
    const hostLine = output
      .split("\n")
      .map((line) => line.trim())
      .find((line) => line.startsWith("host: "));

    if (hostLine) {
      return hostLine.slice("host: ".length).trim();
    }
  } catch {
    // Fall back to Node's platform/arch mapping below.
  }

  return fallbackTargetTriple();
}

function sidecarFilename(command, targetTriple) {
  return targetTriple.includes("windows")
    ? `${command}-${targetTriple}.exe`
    : `${command}-${targetTriple}`;
}

function shouldAutoFetchWindowsSidecars(env = process.env) {
  const explicit = parseBooleanEnv("RKB_AUTO_FETCH_WINDOWS_SIDECARS", env);
  return explicit ?? true;
}

function shouldRequireTargetSidecars(env = process.env) {
  const explicit = parseBooleanEnv("RKB_REQUIRE_TARGET_SIDECARS", env);
  return explicit ?? false;
}

const pinnedWindowsSidecarSha256 = new Map([
  ["x86_64-pc-windows-msvc:ffmpeg", "d2bcaee1792a39e2bfd2c04a3d88daf53d4e857a6583fed68c03562106f745bd"],
  ["aarch64-pc-windows-msvc:ffmpeg", "a29d83d01d3a07cfe060af439c803a082a508fd92c662a74d0ee946888ee4c1a"],
  ["x86_64-pc-windows-msvc:sqlcipher", "19f16d2629adedc6ddc2aeebd2da165d61aa0d645a61d2de373396c04ad0031f"],
]);

function verifyWindowsSidecarHash(command, targetTriple, filePath) {
  if (!targetTriple.includes("windows")) return;
  const expected = pinnedWindowsSidecarSha256.get(`${targetTriple}:${command}`);
  if (!expected) {
    throw new Error(`No pinned SHA-256 is configured for ${command} ${targetTriple}.`);
  }
  const actual = createHash("sha256").update(readFileSync(filePath)).digest("hex");
  if (actual !== expected) {
    throw new Error(
      `Refusing to stage ${path.relative(repoRoot, filePath)}: SHA-256 mismatch for ${command} ${targetTriple}. expected=${expected} actual=${actual}`
    );
  }
}

function ensureWindowsSidecars(targetTriple) {
  if (!targetTriple.includes("windows")) return;
  if (process.platform !== "win32") return;
  if (!shouldAutoFetchWindowsSidecars()) return;

  const commands = ["sqlcipher", "ffmpeg"];
  const missing = commands.filter((command) => {
    const sourcePath = path.join(tauriDir, "bin", sidecarFilename(command, targetTriple));
    return !existsSync(sourcePath);
  });

  if (missing.length === 0) return;

  const scriptPath = path.join(repoRoot, "tools", "fetch-windows-sidecars.ps1");
  const shells = ["powershell.exe", "pwsh"];
  let lastError = null;

  console.log(
    `[tauri-config] missing Windows sidecars for ${targetTriple}: ${missing.join(", ")}`
  );
  console.log("[tauri-config] fetching Windows sidecars automatically...");

  for (const shellCommand of shells) {
    try {
      execFileSync(
        shellCommand,
        ["-NoProfile", "-ExecutionPolicy", "Bypass", "-File", scriptPath],
        {
          cwd: repoRoot,
          env: {
            ...process.env,
            RKB_WINDOWS_TARGET_TRIPLE: targetTriple,
          },
          stdio: "inherit",
        }
      );
      return;
    } catch (error) {
      if (error?.code === "ENOENT") {
        lastError = error;
        continue;
      }
      throw error;
    }
  }

  throw lastError ?? new Error("Unable to launch PowerShell to fetch Windows sidecars.");
}

function isMacTarget(targetTriple) {
  return targetTriple.includes("apple-darwin");
}

function normalizeRelative(value) {
  return value.split(path.sep).join("/");
}

function ensureCleanDir(dirPath) {
  rmSync(dirPath, { recursive: true, force: true });
  mkdirSync(dirPath, { recursive: true });
}

function copyResolvedFile(sourcePath, destinationPath) {
  const resolvedSource = realpathSync(sourcePath);
  mkdirSync(path.dirname(destinationPath), { recursive: true });
  if (existsSync(destinationPath)) {
    chmodSync(destinationPath, 0o644);
    rmSync(destinationPath, { force: true });
  }
  copyFileSync(resolvedSource, destinationPath);
  chmodSync(destinationPath, statSync(resolvedSource).mode | 0o200);
  return resolvedSource;
}

function runTool(command, args) {
  return execFileSync(command, args, {
    cwd: repoRoot,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

function parseMacVersion(value) {
  return value
    .split(".")
    .map((part) => Number.parseInt(part, 10))
    .filter((part) => Number.isFinite(part));
}

function compareMacVersions(left, right) {
  const leftParts = parseMacVersion(left);
  const rightParts = parseMacVersion(right);
  const maxLength = Math.max(leftParts.length, rightParts.length);

  for (let index = 0; index < maxLength; index += 1) {
    const leftPart = leftParts[index] ?? 0;
    const rightPart = rightParts[index] ?? 0;
    if (leftPart < rightPart) return -1;
    if (leftPart > rightPart) return 1;
  }

  return 0;
}

function macSidecarMaxMinOs(targetTriple) {
  if (process.env.RKB_MACOS_SIDECAR_MAX_MINOS) {
    return process.env.RKB_MACOS_SIDECAR_MAX_MINOS;
  }
  if (targetTriple === "aarch64-apple-darwin") {
    return "13.0";
  }
  if (targetTriple === "x86_64-apple-darwin") {
    return "10.15";
  }
  return null;
}

function macBinaryMinOsVersion(filePath) {
  const output = runTool("otool", ["-l", filePath]);
  const buildVersionMatch = output.match(/LC_BUILD_VERSION[\s\S]*?\n\s*minos\s+(\d+(?:\.\d+)+|\d+)/);
  if (buildVersionMatch) {
    return buildVersionMatch[1];
  }

  const versionMinMatch = output.match(/LC_VERSION_MIN_MACOSX[\s\S]*?\n\s*version\s+(\d+(?:\.\d+)+|\d+)/);
  if (versionMinMatch) {
    return versionMinMatch[1];
  }

  return null;
}

function ensureMacSidecarMinOs(filePath, targetTriple) {
  const maxMinOs = macSidecarMaxMinOs(targetTriple);
  if (!maxMinOs) {
    return;
  }

  const actualMinOs = macBinaryMinOsVersion(filePath);
  if (!actualMinOs) {
    throw new Error(`Unable to determine minimum macOS version for ${filePath}.`);
  }

  if (compareMacVersions(actualMinOs, maxMinOs) > 0) {
    throw new Error(
      [
        `Refusing to bundle macOS sidecar ${path.relative(repoRoot, filePath)}.`,
        `minimum macOS version is ${actualMinOs}, but ${targetTriple} bundles must stay at or below ${maxMinOs}.`,
        "Rebuild the sidecar with a lower MACOSX_DEPLOYMENT_TARGET or override RKB_MACOS_SIDECAR_MAX_MINOS if you intentionally want a newer floor.",
      ].join(" ")
    );
  }
}

function adHocSignMacBinary(filePath) {
  runTool("codesign", ["--force", "--sign", "-", filePath]);
}

function macDependencies(filePath) {
  const output = runTool("otool", ["-L", filePath]);
  return output
    .split("\n")
    .slice(1)
    .map((line) => line.trim())
    .filter(Boolean)
    .map((line) => line.replace(/\s+\(compatibility version.*$/, "").trim());
}

function isMacSystemDependency(dependencyPath) {
  return (
    dependencyPath.startsWith("/usr/lib/") ||
    dependencyPath.startsWith("/System/Library/") ||
    dependencyPath.startsWith("/Library/Apple/")
  );
}

function copyMacSidecarBundle({ command, sourcePath, targetTriple, stagedBinaryPath, stagedResourceRoot }) {
  const stagedDeps = new Map();
  const usedNames = new Map();
  const sourceExecutable = copyResolvedFile(sourcePath, stagedBinaryPath);
  ensureMacSidecarMinOs(sourceExecutable, targetTriple);
  const queuedPaths = [];
  const queuedSet = new Set();

  const enqueueDependency = (dependencyPath) => {
    if (!dependencyPath || dependencyPath.startsWith("@") || isMacSystemDependency(dependencyPath)) {
      return;
    }

    const resolvedDependency = realpathSync(dependencyPath);
    if (queuedSet.has(resolvedDependency)) {
      return;
    }

    const fileName = path.basename(resolvedDependency);
    const existing = usedNames.get(fileName);
    if (existing && existing !== resolvedDependency) {
      throw new Error(
        `Cannot bundle ${command} for ${targetTriple}: duplicate dylib name ${fileName} from ${existing} and ${resolvedDependency}.`
      );
    }

    usedNames.set(fileName, resolvedDependency);
    queuedSet.add(resolvedDependency);
    queuedPaths.push(resolvedDependency);
  };

  for (const dependency of macDependencies(sourceExecutable)) {
    enqueueDependency(dependency);
  }

  while (queuedPaths.length > 0) {
    const dependencyPath = queuedPaths.shift();
    const fileName = path.basename(dependencyPath);
    const stagedPath = path.join(stagedResourceRoot, fileName);
    copyResolvedFile(dependencyPath, stagedPath);
    ensureMacSidecarMinOs(stagedPath, targetTriple);
    stagedDeps.set(dependencyPath, {
      fileName,
      stagedPath,
      originalDependencies: macDependencies(dependencyPath),
    });

    for (const childDependency of macDependencies(dependencyPath)) {
      enqueueDependency(childDependency);
    }
  }

  for (const { fileName, stagedPath, originalDependencies } of stagedDeps.values()) {
    runTool("install_name_tool", ["-id", `@loader_path/${fileName}`, stagedPath]);
    for (const originalDependency of originalDependencies) {
      if (originalDependency.startsWith("@") || isMacSystemDependency(originalDependency)) {
        continue;
      }
      const resolvedDependency = realpathSync(originalDependency);
      const bundled = stagedDeps.get(resolvedDependency);
      if (!bundled) {
        throw new Error(
          `Missing staged dependency for ${resolvedDependency} while patching ${stagedPath}.`
        );
      }
      runTool("install_name_tool", [
        "-change",
        originalDependency,
        `@loader_path/${bundled.fileName}`,
        stagedPath,
      ]);
    }
    adHocSignMacBinary(stagedPath);
  }

  for (const originalDependency of macDependencies(sourceExecutable)) {
    if (originalDependency.startsWith("@") || isMacSystemDependency(originalDependency)) {
      continue;
    }
    const resolvedDependency = realpathSync(originalDependency);
    const bundled = stagedDeps.get(resolvedDependency);
    if (!bundled) {
      throw new Error(
        `Missing staged dependency for ${resolvedDependency} while patching ${stagedBinaryPath}.`
      );
    }
    runTool("install_name_tool", [
      "-change",
      originalDependency,
      `@executable_path/../Resources/sidecars/${bundled.fileName}`,
      stagedBinaryPath,
    ]);
  }

  adHocSignMacBinary(stagedBinaryPath);

  return {
    dylibCount: stagedDeps.size,
  };
}

function mergeResourceConfig(baseResources, nextResources) {
  if (!nextResources || Object.keys(nextResources).length === 0) {
    return baseResources ?? null;
  }

  if (baseResources == null) {
    return nextResources;
  }

  if (Array.isArray(baseResources)) {
    throw new Error(
      "bundle.resources must be an object when generated sidecar resources are needed."
    );
  }

  return {
    ...baseResources,
    ...nextResources,
  };
}

function normalizeWindowConfigForTarget(config, targetTriple) {
  if (isMacTarget(targetTriple)) {
    return config;
  }

  const windows = Array.isArray(config.app?.windows) ? config.app.windows : [];
  const normalizedWindows = windows.map((windowConfig) => {
    const nextWindow = { ...windowConfig };
    delete nextWindow.titleBarStyle;
    delete nextWindow.hiddenTitle;
    delete nextWindow.trafficLightPosition;
    return nextWindow;
  });

  return {
    ...config,
    app: {
      ...config.app,
      windows: normalizedWindows,
    },
  };
}

const targetTriple = detectTargetTriple();
const config = normalizeWindowConfigForTarget(
  JSON.parse(readFileSync(baseConfigPath, "utf8")),
  targetTriple
);
const externalBins = Array.isArray(config.bundle?.externalBin) ? config.bundle.externalBin : [];
const bundledBins = [];
const skippedBins = [];
const sidecarSummaries = [];
let generatedResources = {};

ensureWindowsSidecars(targetTriple);
ensureCleanDir(generatedBinDir);
ensureCleanDir(generatedResourceDir);

for (const entry of externalBins) {
  const command = path.basename(entry);
  const sourcePath = path.join(tauriDir, path.dirname(entry), sidecarFilename(command, targetTriple));
  if (!existsSync(sourcePath)) {
    skippedBins.push(path.relative(repoRoot, sourcePath));
    continue;
  }

  const stagedBinaryPath = path.join(generatedBinDir, sidecarFilename(command, targetTriple));
  let dylibCount = 0;

  if (isMacTarget(targetTriple)) {
    const stagedResourceRoot = path.join(generatedResourceDir, targetTriple, "sidecars");
    mkdirSync(stagedResourceRoot, { recursive: true });
    const staged = copyMacSidecarBundle({
      command,
      sourcePath,
      targetTriple,
      stagedBinaryPath,
      stagedResourceRoot,
    });
    dylibCount = staged.dylibCount;
  } else {
    verifyWindowsSidecarHash(command, targetTriple, sourcePath);
    copyResolvedFile(sourcePath, stagedBinaryPath);
  }

  bundledBins.push("gen/bin/" + command);
  sidecarSummaries.push(
    `${command} -> ${normalizeRelative(path.relative(repoRoot, realpathSync(sourcePath)))}${
      dylibCount > 0 ? ` (+${dylibCount} bundled dylibs)` : ""
    }`
  );
}

if (shouldRequireTargetSidecars() && skippedBins.length > 0) {
  throw new Error(
    [
      `Missing required sidecars for ${targetTriple}.`,
      `Expected: ${skippedBins.join(", ")}.`,
      "Release builds must bundle their target sidecars; add them to src-tauri/bin or provide them through the Windows fetch step.",
    ].join(" ")
  );
}

if (isMacTarget(targetTriple)) {
  const stagedResourceRoot = path.join(generatedResourceDir, targetTriple, "sidecars");
  if (existsSync(stagedResourceRoot)) {
    generatedResources = {
      [normalizeRelative(path.relative(tauriDir, stagedResourceRoot))]: "sidecars/",
    };
  }
}

const nextConfig = {
  ...config,
  bundle: {
    ...config.bundle,
    externalBin: bundledBins,
    resources: mergeResourceConfig(config.bundle?.resources, generatedResources),
  },
};

writeFileSync(generatedConfigPath, `${JSON.stringify(nextConfig, null, 2)}\n`);

console.log(`[tauri-config] target triple: ${targetTriple}`);
if (sidecarSummaries.length > 0) {
  for (const summary of sidecarSummaries) {
    console.log(`[tauri-config] prepared sidecar: ${summary}`);
  }
} else {
  console.log("[tauri-config] no sidecars found for this target; build will rely on PATH/env at runtime");
}
if (skippedBins.length > 0) {
  console.log(`[tauri-config] skipped missing sidecars: ${skippedBins.join(", ")}`);
}
if (Object.keys(generatedResources).length > 0) {
  console.log(
    `[tauri-config] bundled sidecar resources: ${Object.keys(generatedResources).length} files`
  );
}
