import { fetch as tauriFetch } from "@tauri-apps/plugin-http";

import type { HttpTransport } from "@loom/ui-kit/lib/api";

/**
 * Native Android transport for runtime-selected Loom servers. The certificate
 * exception is explicit and per connection; hostname verification stays on.
 */
export function createMobileHttpTransport(
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

export const mobileInvalidCertificateWebSocketNote =
  "This exception covers API requests. Live status over WSS still requires a certificate Android trusts.";
