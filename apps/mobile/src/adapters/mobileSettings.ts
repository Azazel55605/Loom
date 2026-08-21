import { load, type Store } from "@tauri-apps/plugin-store";

const STORE_PATH = "mobile-settings.json";

let storePromise: Promise<Store> | null = null;

/** One app-local store for non-sensitive runtime configuration. */
export function mobileSettingsStore(): Promise<Store> {
  storePromise ??= load(STORE_PATH, { autoSave: false });
  return storePromise;
}
