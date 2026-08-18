import path from "node:path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

import pkg from "./package.json" with { type: "json" };

// The frontend version is not hand-written anywhere in src/: it is injected
// here from package.json, which is itself written by scripts/sync-versions.mjs
// from versions.json. See docs/VERSIONING.md.
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
  server: {
    // Mirrors the nginx /api proxy the production image uses, so `pnpm dev`
    // exercises the same same-origin path rather than a special case.
    // Override the target with LOOM_BACKEND_ORIGIN when the backend is not on
    // the default port.
    proxy: {
      "/api": {
        target: process.env.LOOM_BACKEND_ORIGIN ?? "http://localhost:8080",
        changeOrigin: true,
        rewrite: (path) => path.replace(/^\/api/, ""),
      },
    },
  },
});
