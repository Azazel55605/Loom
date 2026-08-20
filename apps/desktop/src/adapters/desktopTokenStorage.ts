import {
  deletePasswords,
  getPasswords,
  setPasswords,
} from "tauri-plugin-keyring-store-api";

import type {
  StoredTokens,
  TokenStorageAdapter,
} from "@loom/ui-kit/lib/token-store";

/** Stable OS-credential-store account owned by Loom Desktop. */
const TOKEN_ACCOUNT = "auth.tokens";

function parseStoredTokens(value: string | null): StoredTokens | null {
  if (value === null) return null;

  try {
    const parsed = JSON.parse(value) as Partial<StoredTokens>;
    if (
      typeof parsed.accessToken === "string" &&
      typeof parsed.refreshToken === "string" &&
      typeof parsed.expiresAt === "string"
    ) {
      return parsed as StoredTokens;
    }
  } catch {
    // A corrupt credential is treated as signed out and removed by TokenStore.
  }

  return null;
}

/** Auth tokens live in Keychain, Credential Manager, or Secret Service. */
export const desktopTokenStorage: TokenStorageAdapter = {
  async getTokens() {
    const [value] = await getPasswords([TOKEN_ACCOUNT]);
    const tokens = parseStoredTokens(value ?? null);
    if (value != null && tokens === null) {
      await deletePasswords([TOKEN_ACCOUNT]);
    }
    return tokens;
  },

  async setTokens(tokens) {
    await setPasswords([{ account: TOKEN_ACCOUNT, secret: JSON.stringify(tokens) }]);
  },

  async clearTokens() {
    await deletePasswords([TOKEN_ACCOUNT]);
  },
};
