import { mobileSettingsStore } from "@/adapters/mobileSettings";
import { PersistentServerProfileManager } from "@loom/ui-kit/lib/server-profile";

export const mobileServerProfileManager = new PersistentServerProfileManager({
  getStore: mobileSettingsStore,
  legacyBaseUrlKey: "serverUrl",
});
