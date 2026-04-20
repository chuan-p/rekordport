import { copyFileSync, mkdirSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const repoRoot = path.resolve(__dirname, "..");
const sourceIcon = path.join(repoRoot, "src-tauri", "icons", "128x128.png");
const publicDir = path.join(repoRoot, "public");
const destIcon = path.join(publicDir, "app-icon.png");

mkdirSync(publicDir, { recursive: true });
copyFileSync(sourceIcon, destIcon);
