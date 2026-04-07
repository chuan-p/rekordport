import { spawnSync } from "node:child_process";
import { existsSync, readdirSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "..");
const tauriDir = path.join(repoRoot, "src-tauri");
const tauriTargetDir = path.join(tauriDir, "target");
const args = process.argv.slice(2);
const command = args[0];

function parseBooleanEnv(name, env = process.env) {
  const value = env[name];
  if (value == null) return null;

  const normalized = String(value).trim().toLowerCase();
  if (["1", "true", "yes", "on"].includes(normalized)) return true;
  if (["0", "false", "no", "off"].includes(normalized)) return false;

  throw new Error(`Unsupported boolean value for ${name}: ${value}`);
}

function hasOfficialAppleSigningConfig(env = process.env) {
  return [
    "APPLE_SIGNING_IDENTITY",
    "APPLE_API_KEY",
    "APPLE_API_ISSUER",
    "APPLE_API_KEY_PATH",
    "APPLE_ID",
    "APPLE_PASSWORD",
    "APPLE_TEAM_ID",
  ].some((name) => Boolean(env[name]));
}

function shouldAdHocSignBundledApps(env = process.env) {
  const explicit = parseBooleanEnv("RKB_AD_HOC_SIGN_BUNDLED_APPS", env);
  if (explicit != null) {
    return explicit;
  }

  return !hasOfficialAppleSigningConfig(env);
}

function removeIfExists(targetPath) {
  if (existsSync(targetPath)) {
    rmSync(targetPath, { recursive: true, force: true });
  }
}

function cleanStaleSidecarDirs() {
  const candidateDirs = [
    path.join(tauriTargetDir, "debug", "sidecars"),
    path.join(tauriTargetDir, "release", "sidecars"),
  ];

  if (existsSync(tauriTargetDir)) {
    for (const entry of readdirSync(tauriTargetDir, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      candidateDirs.push(path.join(tauriTargetDir, entry.name, "debug", "sidecars"));
      candidateDirs.push(path.join(tauriTargetDir, entry.name, "release", "sidecars"));
    }
  }

  for (const dir of candidateDirs) {
    removeIfExists(dir);
  }
}

function adHocSignBundledApps(env = process.env) {
  if (process.platform !== "darwin") return;
  if (!shouldAdHocSignBundledApps(env)) return;

  const bundleDirs = [
    path.join(tauriTargetDir, "release", "bundle", "macos"),
  ];

  if (existsSync(tauriTargetDir)) {
    for (const entry of readdirSync(tauriTargetDir, { withFileTypes: true })) {
      if (!entry.isDirectory()) continue;
      bundleDirs.push(path.join(tauriTargetDir, entry.name, "release", "bundle", "macos"));
    }
  }

  for (const macosBundleDir of bundleDirs) {
    if (!existsSync(macosBundleDir)) continue;

    for (const entry of readdirSync(macosBundleDir, { withFileTypes: true })) {
      if (!entry.isDirectory() || !entry.name.endsWith(".app")) continue;
      const appPath = path.join(macosBundleDir, entry.name);
      const signed = spawnSync("codesign", ["--force", "--deep", "--sign", "-", appPath], {
        cwd: repoRoot,
        env,
        stdio: "inherit",
      });
      if (signed.status !== 0) {
        process.exit(signed.status ?? 1);
      }
    }
  }
}

function extractTargetTriple(cliArgs) {
  for (let index = 0; index < cliArgs.length; index += 1) {
    const value = cliArgs[index];
    if (value === "--target") {
      return cliArgs[index + 1] ?? null;
    }
    if (value.startsWith("--target=")) {
      return value.slice("--target=".length) || null;
    }
  }
  return null;
}

const targetTriple = extractTargetTriple(args);
const childEnv = {
  ...process.env,
  ...(targetTriple ? { RKB_TARGET_TRIPLE: targetTriple } : {}),
  ...(command === "build" ? { RKB_REQUIRE_TARGET_SIDECARS: "true" } : {}),
};

const prepare = spawnSync(process.execPath, [path.join(repoRoot, "tools", "prepare-tauri-config.mjs")], {
  cwd: repoRoot,
  env: childEnv,
  stdio: "inherit",
});

if (prepare.status !== 0) {
  process.exit(prepare.status ?? 1);
}

cleanStaleSidecarDirs();

const rest = args.slice(1);
const tauriArgs = command
  ? [command, "--config", "src-tauri/tauri.generated.conf.json", ...rest]
  : ["--help"];
const tauriBin = path.join(
  repoRoot,
  "node_modules",
  ".bin",
  process.platform === "win32" ? "tauri.cmd" : "tauri"
);

const tauri = spawnSync(tauriBin, tauriArgs, {
  cwd: repoRoot,
  env: childEnv,
  stdio: "inherit",
  shell: process.platform === "win32",
});

if (tauri.status === 0 && command === "build") {
  adHocSignBundledApps(childEnv);
}

process.exit(tauri.status ?? 1);
