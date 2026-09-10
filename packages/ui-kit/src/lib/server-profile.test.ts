import { describe, expect, it } from "vitest";

import { ProfileTokenStorage } from "@loom/ui-kit/lib/profile-token-storage";
import {
  PersistentServerProfileManager,
  ProfileBaseUrlProvider,
  type ServerProfileStore,
} from "@loom/ui-kit/lib/server-profile";
import type { StoredTokens } from "@loom/ui-kit/lib/token-store";

class MemoryStore implements ServerProfileStore {
  readonly values = new Map<string, unknown>();
  saves = 0;

  async get<T>(key: string): Promise<T | undefined> {
    return this.values.get(key) as T | undefined;
  }

  async set(key: string, value: unknown): Promise<void> {
    this.values.set(key, value);
  }

  async save(): Promise<void> {
    this.saves += 1;
  }
}

class MemorySecrets {
  readonly values = new Map<string, string>();

  async get(key: string): Promise<string | null> {
    return this.values.get(key) ?? null;
  }

  async set(key: string, value: string): Promise<void> {
    this.values.set(key, value);
  }

  async remove(key: string): Promise<void> {
    this.values.delete(key);
  }
}

const tokens = (name: string): StoredTokens => ({
  accessToken: `${name}-access`,
  refreshToken: `${name}-refresh`,
  expiresAt: "2030-01-01T00:00:00.000Z",
});

function fixture(legacyBaseUrl?: string) {
  const store = new MemoryStore();
  if (legacyBaseUrl !== undefined) store.values.set("serverUrl", legacyBaseUrl);
  let sequence = 0;
  let clock = 0;
  const manager = new PersistentServerProfileManager({
    getStore: async () => store,
    legacyBaseUrlKey: "serverUrl",
    generateId: () => `profile-${++sequence}`,
    now: () => new Date(++clock * 1_000).toISOString(),
  });
  const secrets = new MemorySecrets();
  const tokenStorage = new ProfileTokenStorage(manager, secrets);
  manager.setTokenCleaner(tokenStorage);
  return { manager, secrets, store, tokenStorage };
}

describe("server profile persistence", () => {
  it("migrates a legacy URL exactly once and makes the derived profile active", async () => {
    const { manager, store } = fixture("https://legacy.example.com:8443");

    const first = await manager.listProfiles();
    expect(first).toEqual([
      {
        id: "profile-1",
        label: "legacy.example.com",
        baseUrl: "https://legacy.example.com:8443",
        lastUsedAt: "1970-01-01T00:00:01.000Z",
      },
    ]);
    expect(await manager.getActiveProfileId()).toBe("profile-1");

    store.values.set("serverUrl", "https://changed.example.com");
    expect(await manager.listProfiles()).toEqual(first);
  });

  it("isolates tokens and URL resolution when switching between profiles", async () => {
    const { manager, tokenStorage } = fixture();
    const baseUrlProvider = new ProfileBaseUrlProvider(manager);
    const first = await manager.addProfile("First", "https://first.example.com");
    await tokenStorage.setTokens(tokens("first"));
    const second = await manager.addProfile("Second", "https://second.example.com");
    await tokenStorage.setTokens(tokens("second"));

    await manager.setActiveProfileId(first.id);
    expect(await baseUrlProvider.getBaseUrl()).toBe("https://first.example.com");
    expect(await tokenStorage.getTokens()).toEqual(tokens("first"));

    await manager.setActiveProfileId(second.id);
    expect(await baseUrlProvider.getBaseUrl()).toBe("https://second.example.com");
    expect(await tokenStorage.getTokens()).toEqual(tokens("second"));
  });

  it("falls back to the most recently used remaining profile", async () => {
    const { manager } = fixture();
    const first = await manager.addProfile("First", "https://first.example.com");
    const second = await manager.addProfile("Second", "https://second.example.com");
    const third = await manager.addProfile("Third", "https://third.example.com");
    await manager.setActiveProfileId(first.id);
    await manager.setActiveProfileId(second.id);

    await manager.removeProfile(second.id);

    expect(await manager.getActiveProfileId()).toBe(first.id);
    expect((await manager.listProfiles()).map(({ id }) => id)).toEqual([first.id, third.id]);
  });

  it("removing a profile clears only its credentials", async () => {
    const { manager, secrets, tokenStorage } = fixture();
    const first = await manager.addProfile("First", "https://first.example.com");
    await tokenStorage.setTokens(tokens("first"));
    const second = await manager.addProfile("Second", "https://second.example.com");
    await tokenStorage.setTokens(tokens("second"));

    await manager.removeProfile(second.id);

    expect(secrets.values.has(`${second.id}:auth.tokens`)).toBe(false);
    expect(secrets.values.has(`${first.id}:auth.tokens`)).toBe(true);
    expect(await manager.getActiveProfileId()).toBe(first.id);
    expect(await tokenStorage.getTokens()).toEqual(tokens("first"));
  });

  it("moves the legacy unscoped credential into only the migrated profile", async () => {
    const { manager, secrets, tokenStorage } = fixture("https://legacy.example.com");
    secrets.values.set("auth.tokens", JSON.stringify(tokens("legacy")));
    const [legacy] = await manager.listProfiles();

    expect(await tokenStorage.getTokens()).toEqual(tokens("legacy"));
    expect(secrets.values.has("auth.tokens")).toBe(false);
    expect(secrets.values.get(`${legacy.id}:auth.tokens`)).toBe(JSON.stringify(tokens("legacy")));

    await manager.addProfile("Other", "https://other.example.com");
    expect(await tokenStorage.getTokens()).toBeNull();
  });

  it("clears an unmigrated legacy credential when its profile is removed", async () => {
    const { manager, secrets } = fixture("https://legacy.example.com");
    secrets.values.set("auth.tokens", JSON.stringify(tokens("legacy")));
    const [legacy] = await manager.listProfiles();

    await manager.removeProfile(legacy.id);

    expect(secrets.values.has("auth.tokens")).toBe(false);
    expect(await manager.getActiveProfileId()).toBeNull();
    expect(await manager.listProfiles()).toEqual([]);
  });
});
