import { load, type Store } from "@tauri-apps/plugin-store";

const STORE_PATH = "desktop-settings.json";

let storePromise: Promise<Store> | null = null;

/** One app-local store for non-sensitive Desktop runtime configuration. */
export function desktopSettingsStore(): Promise<Store> {
  storePromise ??= load(STORE_PATH, { autoSave: false });
  return storePromise;
}
