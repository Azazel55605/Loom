/** The access/refresh token pair persisted by a platform adapter. */
export type StoredTokens = {
  accessToken: string;
  refreshToken: string;
  /** RFC 3339 expiry for the access token. */
  expiresAt: string;
};

/** Platform-owned persistence for sensitive authentication state. */
export interface TokenStorageAdapter {
  getTokens(): Promise<StoredTokens | null>;
  setTokens(tokens: StoredTokens): Promise<void>;
  clearTokens(): Promise<void>;
}

type Listener = () => void;

function isStoredTokens(value: StoredTokens | null): value is StoredTokens {
  return (
    value !== null &&
    typeof value.accessToken === "string" &&
    typeof value.refreshToken === "string" &&
    typeof value.expiresAt === "string"
  );
}

/** Shared in-memory session state backed by an injected platform adapter. */
export class TokenStore {
  private current: StoredTokens | null = null;
  private initialized = false;
  private initialization: Promise<void> | null = null;
  private readonly listeners = new Set<Listener>();

  constructor(private readonly adapter: TokenStorageAdapter) {}

  initialize(): Promise<void> {
    if (this.initialized) return Promise.resolve();
    if (this.initialization !== null) return this.initialization;

    this.initialization = (async () => {
      try {
        const stored = await this.adapter.getTokens();
        this.current = isStoredTokens(stored) ? stored : null;
        if (stored !== null && !isStoredTokens(stored)) {
          await this.adapter.clearTokens();
        }
      } catch {
        this.current = null;
      } finally {
        this.initialized = true;
        this.emit();
      }
    })();

    return this.initialization;
  }

  isInitialized(): boolean {
    return this.initialized;
  }

  getSnapshot = (): StoredTokens | null => this.current;

  getServerSnapshot = (): StoredTokens | null => null;

  getAccessToken(): string | null {
    return this.current?.accessToken ?? null;
  }

  expiresWithin(bufferMs: number): boolean {
    if (this.current === null) return false;
    const expiry = Date.parse(this.current.expiresAt);
    return Number.isNaN(expiry) || expiry - Date.now() <= bufferMs;
  }

  async setTokens(tokens: StoredTokens): Promise<void> {
    this.current = tokens;
    this.emit();
    try {
      await this.adapter.setTokens(tokens);
    } catch {
      // The in-memory session remains usable when persistence is unavailable.
    }
  }

  async clear(): Promise<void> {
    this.current = null;
    this.emit();
    try {
      await this.adapter.clearTokens();
    } catch {
      // Local memory is already clear; persistence remains best-effort.
    }
  }

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  private emit(): void {
    for (const listener of this.listeners) listener();
  }
}
