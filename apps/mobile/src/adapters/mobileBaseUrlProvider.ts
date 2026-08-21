import { mobileSettingsStore } from "@/adapters/mobileSettings";
import type { BaseUrlProvider } from "@loom/ui-kit/lib/api";

const SERVER_URL_KEY = "serverUrl";

/** Non-sensitive runtime server configuration persisted by Tauri Store. */
class MobileBaseUrlProvider implements BaseUrlProvider {
  async getBaseUrl(): Promise<string> {
    const value = await (await mobileSettingsStore()).get<unknown>(SERVER_URL_KEY);
    return typeof value === "string" ? value : "";
  }

  async setBaseUrl(baseUrl: string): Promise<void> {
    const store = await mobileSettingsStore();
    await store.set(SERVER_URL_KEY, baseUrl);
    await store.save();
  }
}

export const mobileBaseUrlProvider = new MobileBaseUrlProvider();
