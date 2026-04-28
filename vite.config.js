import { defineConfig } from "vite";
import { readFileSync } from "node:fs";

const packageJson = JSON.parse(readFileSync(new URL("./package.json", import.meta.url), "utf8"));

export default defineConfig({
  base: "./",
  clearScreen: false,
  define: {
    "import.meta.env.VITE_APP_VERSION": JSON.stringify(packageJson.version),
  },
  optimizeDeps: {
    entries: ["index.html"],
    include: ["@tauri-apps/api/core", "@tauri-apps/api/event"],
  },
  server: {
    host: "127.0.0.1",
    port: 1420,
    strictPort: true,
    headers: {
      "Cache-Control": "no-store",
    },
  },
  build: {
    outDir: "dist",
    emptyOutDir: true,
  },
});
