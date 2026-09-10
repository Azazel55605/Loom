/** One device-local Loom server connection. Authentication remains in the
 * platform's secure token store and is never included here. */
export interface ServerProfile {
  id: string;
  label: string;
  baseUrl: string;
  lastUsedAt: string | null;
}

/**
 * Device-local persistence and bookkeeping for Loom server profiles.
 *
 * Switching the active profile does not probe reachability, refresh a session,
 * or reset application state. The caller that initiates a switch must run the
 * ordinary connection bootstrap afterward.
 *
 * Removing the active profile selects the remaining profile with the newest
 * `lastUsedAt`, or leaves no active profile when the list becomes empty.
 */
export interface ServerProfileManager {
  listProfiles(): Promise<ServerProfile[]>;
  getActiveProfileId(): Promise<string | null>;
  setActiveProfileId(id: string): Promise<void>;
  addProfile(label: string, baseUrl: string): Promise<ServerProfile>;
  removeProfile(id: string): Promise<void>;
  renameProfile(id: string, label: string): Promise<void>;
}

export interface ServerProfileStore {
  get<T>(key: string): Promise<T | undefined>;
  set(key: string, value: unknown): Promise<void>;
  save(): Promise<void>;
}

export interface ServerProfileTokenCleaner {
  clearTokensForProfile(profileId: string): Promise<void>;
  clearLegacyTokens?(): Promise<void>;
}

type ManagerOptions = {
  getStore: () => Promise<ServerProfileStore>;
  legacyBaseUrlKey: string;
  now?: () => string;
  generateId?: () => string;
};

const PROFILES_KEY = "serverProfiles";
const ACTIVE_PROFILE_KEY = "activeServerProfileId";
const MIGRATED_KEY = "serverProfilesMigrated";
const LEGACY_PROFILE_KEY = "legacyServerProfileId";

function isProfile(value: unknown): value is ServerProfile {
  if (typeof value !== "object" || value === null) return false;
  const candidate = value as Partial<ServerProfile>;
  return (
    typeof candidate.id === "string" &&
    candidate.id !== "" &&
    typeof candidate.label === "string" &&
    candidate.label !== "" &&
    typeof candidate.baseUrl === "string" &&
    candidate.baseUrl !== "" &&
    (candidate.lastUsedAt === null || typeof candidate.lastUsedAt === "string")
  );
}

function parseProfiles(value: unknown): ServerProfile[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  return value.filter((profile): profile is ServerProfile => {
    if (!isProfile(profile) || seen.has(profile.id)) return false;
    seen.add(profile.id);
    return true;
  });
}

export function serverProfileLabel(baseUrl: string): string {
  try {
    return new URL(baseUrl).hostname || baseUrl;
  } catch {
    return baseUrl;
  }
}

function mostRecentlyUsed(profiles: ServerProfile[]): ServerProfile | null {
  return profiles.reduce<ServerProfile | null>((newest, profile) => {
    if (newest === null) return profile;
    const newestTime = newest.lastUsedAt ? Date.parse(newest.lastUsedAt) : 0;
    const profileTime = profile.lastUsedAt ? Date.parse(profile.lastUsedAt) : 0;
    return profileTime > newestTime ? profile : newest;
  }, null);
}

/** Shared, testable persistence engine instantiated with each platform's Tauri Store. */
export class PersistentServerProfileManager implements ServerProfileManager {
  private readonly getStore: () => Promise<ServerProfileStore>;
  private readonly legacyBaseUrlKey: string;
  private readonly now: () => string;
  private readonly generateId: () => string;
  private queue: Promise<void> = Promise.resolve();
  private tokenCleaner: ServerProfileTokenCleaner | null = null;

  constructor(options: ManagerOptions) {
    this.getStore = options.getStore;
    this.legacyBaseUrlKey = options.legacyBaseUrlKey;
    this.now = options.now ?? (() => new Date().toISOString());
    this.generateId = options.generateId ?? (() => crypto.randomUUID());
  }

  setTokenCleaner(cleaner: ServerProfileTokenCleaner): void {
    this.tokenCleaner = cleaner;
  }

  listProfiles(): Promise<ServerProfile[]> {
    return this.exclusive(async () => {
      const store = await this.getStore();
      return [...(await this.loadProfiles(store))];
    });
  }

  getActiveProfileId(): Promise<string | null> {
    return this.exclusive(async () => {
      const store = await this.getStore();
      const profiles = await this.loadProfiles(store);
      const activeId = await store.get<unknown>(ACTIVE_PROFILE_KEY);
      return typeof activeId === "string" && profiles.some(({ id }) => id === activeId)
        ? activeId
        : null;
    });
  }

  setActiveProfileId(id: string): Promise<void> {
    return this.exclusive(async () => {
      const store = await this.getStore();
      const profiles = await this.loadProfiles(store);
      const index = profiles.findIndex((profile) => profile.id === id);
      if (index < 0) throw new Error("The selected server profile does not exist.");
      profiles[index] = { ...profiles[index], lastUsedAt: this.now() };
      await store.set(PROFILES_KEY, profiles);
      await store.set(ACTIVE_PROFILE_KEY, id);
      await store.save();
    });
  }

  addProfile(label: string, baseUrl: string): Promise<ServerProfile> {
    return this.exclusive(async () => {
      const normalizedLabel = label.trim();
      const normalizedBaseUrl = baseUrl.trim();
      if (!normalizedLabel || !normalizedBaseUrl) {
        throw new Error("A server profile needs both a label and a base URL.");
      }
      const store = await this.getStore();
      const profiles = await this.loadProfiles(store);
      const profile: ServerProfile = {
        id: this.generateId(),
        label: normalizedLabel,
        baseUrl: normalizedBaseUrl,
        lastUsedAt: this.now(),
      };
      await store.set(PROFILES_KEY, [...profiles, profile]);
      await store.set(ACTIVE_PROFILE_KEY, profile.id);
      await store.save();
      return profile;
    });
  }

  removeProfile(id: string): Promise<void> {
    return this.exclusive(async () => {
      const store = await this.getStore();
      const profiles = await this.loadProfiles(store);
      if (!profiles.some((profile) => profile.id === id)) return;

      await this.tokenCleaner?.clearTokensForProfile(id);
      if (id === await store.get<unknown>(LEGACY_PROFILE_KEY)) {
        await this.tokenCleaner?.clearLegacyTokens?.();
      }
      const remaining = profiles.filter((profile) => profile.id !== id);
      const activeId = await store.get<unknown>(ACTIVE_PROFILE_KEY);
      await store.set(PROFILES_KEY, remaining);
      if (activeId === id) {
        await store.set(ACTIVE_PROFILE_KEY, mostRecentlyUsed(remaining)?.id ?? null);
      }
      await store.save();
    });
  }

  renameProfile(id: string, label: string): Promise<void> {
    return this.updateProfile(id, { label });
  }

  /** Adapter-only compatibility seam for editing the currently-active connection. */
  updateProfile(
    id: string,
    update: { label?: string; baseUrl?: string },
  ): Promise<void> {
    return this.exclusive(async () => {
      const label = update.label?.trim();
      const baseUrl = update.baseUrl?.trim();
      if (update.label !== undefined && !label) throw new Error("A server profile label cannot be empty.");
      if (update.baseUrl !== undefined && !baseUrl) throw new Error("A server profile URL cannot be empty.");
      const store = await this.getStore();
      const profiles = await this.loadProfiles(store);
      const index = profiles.findIndex((profile) => profile.id === id);
      if (index < 0) throw new Error("The selected server profile does not exist.");
      profiles[index] = {
        ...profiles[index],
        ...(label === undefined ? {} : { label }),
        ...(baseUrl === undefined ? {} : { baseUrl }),
        lastUsedAt: this.now(),
      };
      await store.set(PROFILES_KEY, profiles);
      await store.save();
    });
  }

  /** Adapter-only bridge for the existing "choose another server" bootstrap flow. */
  clearActiveProfileId(): Promise<void> {
    return this.exclusive(async () => {
      const store = await this.getStore();
      await this.loadProfiles(store);
      await store.set(ACTIVE_PROFILE_KEY, null);
      await store.save();
    });
  }

  /** Identifies the sole profile allowed to claim the old unscoped credential. */
  getLegacyMigratedProfileId(): Promise<string | null> {
    return this.exclusive(async () => {
      const store = await this.getStore();
      await this.loadProfiles(store);
      const profileId = await store.get<unknown>(LEGACY_PROFILE_KEY);
      return typeof profileId === "string" ? profileId : null;
    });
  }

  private async loadProfiles(store: ServerProfileStore): Promise<ServerProfile[]> {
    const current = parseProfiles(await store.get<unknown>(PROFILES_KEY));
    if ((await store.get<unknown>(MIGRATED_KEY)) === true) return current;

    const legacyBaseUrl = await store.get<unknown>(this.legacyBaseUrlKey);
    let profiles = current;
    if (profiles.length === 0 && typeof legacyBaseUrl === "string" && legacyBaseUrl.trim()) {
      const baseUrl = legacyBaseUrl.trim();
      const profile: ServerProfile = {
        id: this.generateId(),
        label: serverProfileLabel(baseUrl),
        baseUrl,
        lastUsedAt: this.now(),
      };
      profiles = [profile];
      await store.set(PROFILES_KEY, profiles);
      await store.set(ACTIVE_PROFILE_KEY, profile.id);
      await store.set(LEGACY_PROFILE_KEY, profile.id);
    }
    await store.set(MIGRATED_KEY, true);
    await store.save();
    return profiles;
  }

  private exclusive<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.queue.then(operation, operation);
    this.queue = result.then(() => undefined, () => undefined);
    return result;
  }
}

/** BaseUrlProvider implementation that keeps the API client profile-agnostic. */
export class ProfileBaseUrlProvider {
  constructor(private readonly profiles: ServerProfileManager) {}

  async getBaseUrl(): Promise<string> {
    const [allProfiles, activeProfileId] = await Promise.all([
      this.profiles.listProfiles(),
      this.profiles.getActiveProfileId(),
    ]);
    return allProfiles.find(({ id }) => id === activeProfileId)?.baseUrl ?? "";
  }
}
