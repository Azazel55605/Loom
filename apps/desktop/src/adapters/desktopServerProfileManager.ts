import { desktopSettingsStore } from "@/adapters/desktopSettings";
import { PersistentServerProfileManager } from "@loom/ui-kit/lib/server-profile";

export const desktopServerProfileManager = new PersistentServerProfileManager({
  getStore: desktopSettingsStore,
  legacyBaseUrlKey: "serverUrl",
});
