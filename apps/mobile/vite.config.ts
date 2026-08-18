import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

import pkg from "./package.json" with { type: "json" };

// Set by `tauri android dev` so the dev server is reachable from the device or
// emulator rather than only from the host's loopback interface.
const host = process.env.TAURI_DEV_HOST;

// The mobile version is not hand-written anywhere in src/: it is injected here
// from package.json, which is written by scripts/sync-versions.mjs from
// versions.json. See docs/VERSIONING.md.
export default defineConfig({
  plugins: [react()],
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },
  resolve: {
    alias: {
      "@": path.resolve(import.meta.dirname, "./src"),
    },
  },
  clearScreen: false,
  server: {
    host: host || false,
    port: 1420,
    strictPort: true,
    hmr: host ? { protocol: "ws", host, port: 1421 } : undefined,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
});
