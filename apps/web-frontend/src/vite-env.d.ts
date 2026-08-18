/// <reference types="vite/client" />

/** Injected by vite.config.ts from package.json — see docs/VERSIONING.md. */
declare const __APP_VERSION__: string;

interface ImportMetaEnv {
  /** Base URL of the web-backend API. Baked in at build time. */
  readonly VITE_API_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
