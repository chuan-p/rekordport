import { execFileSync } from "node:child_process";
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
  if (platform === "linux" && arch === "x64") return "x86_64-unknown-linux-gnu";

  throw new Error(`Unsupported build host ${platform}/${arch}. Set RKB_TARGET_TRIPLE explicitly.`);
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

const targetTriple = detectTargetTriple();
const config = JSON.parse(readFileSync(baseConfigPath, "utf8"));
const externalBins = Array.isArray(config.bundle?.externalBin) ? config.bundle.externalBin : [];
const bundledBins = [];
const skippedBins = [];
const sidecarSummaries = [];
let generatedResources = {};

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
    copyResolvedFile(sourcePath, stagedBinaryPath);
  }

  bundledBins.push("gen/bin/" + command);
  sidecarSummaries.push(
    `${command} -> ${normalizeRelative(path.relative(repoRoot, realpathSync(sourcePath)))}${
      dylibCount > 0 ? ` (+${dylibCount} bundled dylibs)` : ""
    }`
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
