import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

import pkg from "./package.json" with { type: "json" };

// The desktop version is not hand-written anywhere in src/: it is injected here
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
  // Tauri expects a fixed dev port and surfaces Rust-side errors better when
  // Vite does not swallow them.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // src-tauri is compiled by Cargo; watching it would restart Vite on every
      // Rust build artifact change.
      ignored: ["**/src-tauri/**"],
    },
  },
});
