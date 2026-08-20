import { fetch as tauriFetch } from "@tauri-apps/plugin-http";

import type { HttpTransport } from "@loom/ui-kit/lib/api";

/**
 * Native Desktop transport. Tauri's HTTP plugin is not subject to the webview's
 * TLS/CORS implementation and can opt into invalid certificate chains for a
 * user-selected homelab server. Hostname verification remains enabled.
 */
export function createDesktopHttpTransport(
  allowInvalidCertificates: boolean,
): HttpTransport {
  return {
    fetch: (input, init) =>
      tauriFetch(input, {
        ...init,
        danger: allowInvalidCertificates
          ? {
              acceptInvalidCerts: true,
              acceptInvalidHostnames: false,
            }
          : undefined,
      }),
  };
}
