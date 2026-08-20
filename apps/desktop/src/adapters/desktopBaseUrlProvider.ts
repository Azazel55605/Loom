import { load, type Store } from "@tauri-apps/plugin-store";

import type { BaseUrlProvider } from "@loom/ui-kit/lib/api";

const STORE_PATH = "desktop-settings.json";
const SERVER_URL_KEY = "serverUrl";

let storePromise: Promise<Store> | null = null;

function settingsStore(): Promise<Store> {
  storePromise ??= load(STORE_PATH, { autoSave: false });
  return storePromise;
}

/** Non-sensitive runtime server configuration persisted by Tauri Store. */
class DesktopBaseUrlProvider implements BaseUrlProvider {
  async getBaseUrl(): Promise<string> {
    const value = await (await settingsStore()).get<unknown>(SERVER_URL_KEY);
    return typeof value === "string" ? value : "";
  }

  async setBaseUrl(baseUrl: string): Promise<void> {
    const store = await settingsStore();
    await store.set(SERVER_URL_KEY, baseUrl);
    await store.save();
  }
}

export const desktopBaseUrlProvider = new DesktopBaseUrlProvider();
