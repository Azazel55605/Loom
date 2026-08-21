import { appDataDir, join } from "@tauri-apps/api/path";
import { Stronghold, type Store as StrongholdStore } from "@tauri-apps/plugin-stronghold";

import { mobileSettingsStore } from "@/adapters/mobileSettings";
import type {
  StoredTokens,
  TokenStorageAdapter,
} from "@loom/ui-kit/lib/token-store";

const VAULT_KEY_SETTING = "strongholdVaultKey";
const SNAPSHOT_FILENAME = "loom-mobile.hold";
const CLIENT_NAME = "loom-mobile";
const TOKEN_RECORD = "auth.tokens";

type VaultRuntime = {
  stronghold: Stronghold;
  store: StrongholdStore;
};

let vaultPromise: Promise<VaultRuntime> | null = null;

function generateVaultKey(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(32));
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join("");
}

async function getOrCreateVaultKey(): Promise<string> {
  const settings = await mobileSettingsStore();
  const existing = await settings.get<unknown>(VAULT_KEY_SETTING);
  if (typeof existing === "string" && existing !== "") return existing;

  const generated = generateVaultKey();
  await settings.set(VAULT_KEY_SETTING, generated);
  await settings.save();
  return generated;
}

async function vaultRuntime(): Promise<VaultRuntime> {
  vaultPromise ??= (async () => {
    const [directory, password] = await Promise.all([
      appDataDir(),
      getOrCreateVaultKey(),
    ]);
    const stronghold = await Stronghold.load(
      await join(directory, SNAPSHOT_FILENAME),
      password,
    );

    try {
      return {
        stronghold,
        store: (await stronghold.loadClient(CLIENT_NAME)).getStore(),
      };
    } catch {
      const store = (await stronghold.createClient(CLIENT_NAME)).getStore();
      await stronghold.save();
      return { stronghold, store };
    }
  })();
  return vaultPromise;
}

function parseStoredTokens(value: Uint8Array | null): StoredTokens | null {
  if (value === null) return null;

  try {
    const parsed = JSON.parse(new TextDecoder().decode(value)) as Partial<StoredTokens>;
    if (
      typeof parsed.accessToken === "string" &&
      typeof parsed.refreshToken === "string" &&
      typeof parsed.expiresAt === "string"
    ) {
      return parsed as StoredTokens;
    }
  } catch {
    // A corrupt vault record is treated as signed out and removed below.
  }

  return null;
}

/** Auth tokens are encrypted in a Stronghold snapshot, never Tauri Store. */
export const mobileTokenStorage: TokenStorageAdapter = {
  async getTokens() {
    const runtime = await vaultRuntime();
    const value = await runtime.store.get(TOKEN_RECORD);
    const tokens = parseStoredTokens(value);
    if (value !== null && tokens === null) {
      await runtime.store.remove(TOKEN_RECORD);
      await runtime.stronghold.save();
    }
    return tokens;
  },

  async setTokens(tokens) {
    const runtime = await vaultRuntime();
    const encoded = new TextEncoder().encode(JSON.stringify(tokens));
    await runtime.store.insert(TOKEN_RECORD, Array.from(encoded));
    await runtime.stronghold.save();
  },

  async clearTokens() {
    const runtime = await vaultRuntime();
    await runtime.store.remove(TOKEN_RECORD);
    await runtime.stronghold.save();
  },
};
