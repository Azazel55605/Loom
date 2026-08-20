import type { StoredTokens, TokenStorageAdapter } from "@loom/ui-kit/lib/token-store";

const STORAGE_KEY = "loom.auth.session";

export const webTokenStorage: TokenStorageAdapter = {
  async getTokens(): Promise<StoredTokens | null> {
    try {
      const raw = window.localStorage.getItem(STORAGE_KEY);
      return raw === null ? null : (JSON.parse(raw) as StoredTokens);
    } catch {
      return null;
    }
  },

  async setTokens(tokens: StoredTokens): Promise<void> {
    window.localStorage.setItem(STORAGE_KEY, JSON.stringify(tokens));
  },

  async clearTokens(): Promise<void> {
    window.localStorage.removeItem(STORAGE_KEY);
  },
};
