import { appDataDir, join } from "@tauri-apps/api/path";
import { Stronghold, type Store as StrongholdStore } from "@tauri-apps/plugin-stronghold";

import { mobileSettingsStore } from "@/adapters/mobileSettings";
import { mobileServerProfileManager } from "@/adapters/mobileServerProfileManager";
import { ProfileTokenStorage } from "@loom/ui-kit/lib/profile-token-storage";

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

/** Auth tokens remain encrypted in Stronghold and are isolated by profile id. */
export const mobileTokenStorage = new ProfileTokenStorage(
  mobileServerProfileManager,
  {
    async get(key) {
      const value = await (await vaultRuntime()).store.get(key);
      return value === null ? null : new TextDecoder().decode(value);
    },
    async set(key, value) {
      const runtime = await vaultRuntime();
      const encoded = new TextEncoder().encode(value);
      await runtime.store.insert(key, Array.from(encoded));
      await runtime.stronghold.save();
    },
    async remove(key) {
      const runtime = await vaultRuntime();
      await runtime.store.remove(key);
      await runtime.stronghold.save();
    },
  },
  TOKEN_RECORD,
);

mobileServerProfileManager.setTokenCleaner(mobileTokenStorage);
