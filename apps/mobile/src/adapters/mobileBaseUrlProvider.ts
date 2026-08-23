import { mobileSettingsStore } from "@/adapters/mobileSettings";
import type { ServerConnection } from "@loom/ui-kit/components/ConnectToServer";
import type { BaseUrlProvider } from "@loom/ui-kit/lib/api";

const SERVER_URL_KEY = "serverUrl";
const ALLOW_INVALID_CERTIFICATES_KEY = "allowInvalidCertificates";

/** Non-sensitive runtime server configuration persisted by Tauri Store. */
class MobileBaseUrlProvider implements BaseUrlProvider {
  async getBaseUrl(): Promise<string> {
    return (await this.getConnection()).baseUrl;
  }

  async getConnection(): Promise<ServerConnection> {
    const store = await mobileSettingsStore();
    const [baseUrl, allowInvalidCertificates] = await Promise.all([
      store.get<unknown>(SERVER_URL_KEY),
      store.get<unknown>(ALLOW_INVALID_CERTIFICATES_KEY),
    ]);
    return {
      baseUrl: typeof baseUrl === "string" ? baseUrl : "",
      allowInvalidCertificates: allowInvalidCertificates === true,
    };
  }

  async setConnection(connection: ServerConnection): Promise<void> {
    const store = await mobileSettingsStore();
    await Promise.all([
      store.set(SERVER_URL_KEY, connection.baseUrl),
      store.set(
        ALLOW_INVALID_CERTIFICATES_KEY,
        connection.allowInvalidCertificates,
      ),
    ]);
    await store.save();
  }
}

export const mobileBaseUrlProvider = new MobileBaseUrlProvider();
