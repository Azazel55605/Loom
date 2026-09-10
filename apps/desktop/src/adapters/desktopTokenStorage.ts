import {
  deletePasswords,
  getPasswords,
  setPasswords,
} from "tauri-plugin-keyring-store-api";

import { desktopServerProfileManager } from "@/adapters/desktopServerProfileManager";
import { ProfileTokenStorage } from "@loom/ui-kit/lib/profile-token-storage";

/** Stable OS-credential-store account owned by Loom Desktop. */
const TOKEN_ACCOUNT = "auth.tokens";

/** Auth tokens remain in Keychain, Credential Manager, or Secret Service;
 * only their account name is now namespaced by server profile. */
export const desktopTokenStorage = new ProfileTokenStorage(
  desktopServerProfileManager,
  {
    async get(key) {
      const [value] = await getPasswords([key]);
      return value ?? null;
    },
    async set(key, value) {
      await setPasswords([{ account: key, secret: value }]);
    },
    async remove(key) {
      await deletePasswords([key]);
    },
  },
  TOKEN_ACCOUNT,
);

desktopServerProfileManager.setTokenCleaner(desktopTokenStorage);
