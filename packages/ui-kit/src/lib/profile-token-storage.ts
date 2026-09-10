import type { StoredTokens, TokenStorageAdapter } from "@loom/ui-kit/lib/token-store";

export interface ProfileTokenSecretStore {
  get(key: string): Promise<string | null>;
  set(key: string, value: string): Promise<void>;
  remove(key: string): Promise<void>;
}

export interface ActiveServerProfileReader {
  getActiveProfileId(): Promise<string | null>;
  getLegacyMigratedProfileId(): Promise<string | null>;
}

function parseTokens(value: string | null): StoredTokens | null {
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
    // Corrupt credentials are removed below and treated as signed out.
  }
  return null;
}

/** Profile namespacing shared by the OS-credential and Stronghold adapters. */
export class ProfileTokenStorage implements TokenStorageAdapter {
  private queue: Promise<void> = Promise.resolve();

  constructor(
    private readonly profiles: ActiveServerProfileReader,
    private readonly secrets: ProfileTokenSecretStore,
    private readonly legacyRecordKey = "auth.tokens",
  ) {}

  async getTokens(): Promise<StoredTokens | null> {
    const profileId = await this.profiles.getActiveProfileId();
    if (profileId === null) return null;
    const legacyProfileId = await this.profiles.getLegacyMigratedProfileId();

    return this.exclusive(async () => {
      const key = this.profileRecordKey(profileId);
      let value = await this.secrets.get(key);

      if (value === null && profileId === legacyProfileId) {
        value = await this.secrets.get(this.legacyRecordKey);
        if (value !== null) {
          await this.secrets.set(key, value);
          await this.secrets.remove(this.legacyRecordKey);
        }
      }

      const tokens = parseTokens(value);
      if (value !== null && tokens === null) await this.secrets.remove(key);
      return tokens;
    });
  }

  async setTokens(tokens: StoredTokens): Promise<void> {
    const profileId = await this.profiles.getActiveProfileId();
    if (profileId === null) throw new Error("Cannot store tokens without an active server profile.");
    return this.exclusive(() =>
      this.secrets.set(this.profileRecordKey(profileId), JSON.stringify(tokens)),
    );
  }

  async clearTokens(): Promise<void> {
    const profileId = await this.profiles.getActiveProfileId();
    if (profileId === null) return;
    return this.exclusive(() => this.secrets.remove(this.profileRecordKey(profileId)));
  }

  clearTokensForProfile(profileId: string): Promise<void> {
    return this.exclusive(() => this.secrets.remove(this.profileRecordKey(profileId)));
  }

  clearLegacyTokens(): Promise<void> {
    return this.exclusive(() => this.secrets.remove(this.legacyRecordKey));
  }

  private profileRecordKey(profileId: string): string {
    return `${profileId}:${this.legacyRecordKey}`;
  }

  private exclusive<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.queue.then(operation, operation);
    this.queue = result.then(() => undefined, () => undefined);
    return result;
  }
}
