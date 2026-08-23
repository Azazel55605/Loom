import * as React from "react";

import {
  ApiError,
  createApiClient,
  SessionExpiredError,
  type BaseUrlProvider,
  type HttpTransport,
  type PermissionGrant,
} from "@loom/ui-kit/lib/api";
import { ApiClientProvider } from "@loom/ui-kit/lib/api-context";
import { ConnectorStatusSocket } from "@loom/ui-kit/lib/connector-socket";
import type { StoredTokens, TokenStorageAdapter } from "@loom/ui-kit/lib/token-store";

const PROACTIVE_REFRESH_BUFFER_MS = 60_000;

export type CurrentUser = {
  id: string;
  username: string;
  permissions: PermissionGrant[];
};

type AuthContextValue = {
  isAuthenticated: boolean;
  user: CurrentUser | null;
  isRestoring: boolean;
  signIn: (username: string, password: string) => Promise<void>;
  signOut: () => Promise<void>;
  refresh: () => Promise<void>;
};

const AuthContext = React.createContext<AuthContextValue | null>(null);

/**
 * Owns the shared API/auth runtime while platform adapters own persistence and
 * backend discovery.
 */
export function AuthProvider({
  baseUrlProvider,
  httpTransport,
  tokenStorage,
  children,
}: {
  baseUrlProvider: BaseUrlProvider;
  httpTransport?: HttpTransport;
  tokenStorage: TokenStorageAdapter;
  children: React.ReactNode;
}) {
  const [client] = React.useState(() =>
    createApiClient({ baseUrlProvider, httpTransport, tokenStorage }),
  );
  const [connectorSocket] = React.useState(() => new ConnectorStatusSocket(client));
  const [runtimeReady, setRuntimeReady] = React.useState(false);
  const session = React.useSyncExternalStore<StoredTokens | null>(
    client.tokenStore.subscribe,
    client.tokenStore.getSnapshot,
    client.tokenStore.getServerSnapshot,
  );
  const [user, setUser] = React.useState<CurrentUser | null>(null);
  const [isRestoring, setIsRestoring] = React.useState(true);

  React.useEffect(() => {
    let cancelled = false;
    void client.initialize().finally(() => {
      if (!cancelled) setRuntimeReady(true);
    });
    return () => {
      cancelled = true;
    };
  }, [client]);

  // Deliberately **not** disposed from an effect cleanup.
  //
  // `dispose()` is terminal — it sets a flag that makes every later
  // `ensureConnected()` return early — and StrictMode runs an effect's cleanup
  // between its two development mounts. A `useEffect(() => () =>
  // connectorSocket.dispose())` therefore killed the socket permanently on the
  // first render in dev, and every view's `subscribe()` afterwards silently did
  // nothing: no status ever arrived, with no error to explain it. The same
  // would happen in production on any remount of this provider.
  //
  // Nothing leaks by leaving it alone. The socket closes its connection on its
  // own once its last listener unsubscribes, and the only other thing
  // `dispose()` releases is a token-store subscription on an object with
  // exactly this socket's lifetime.

  React.useEffect(() => {
    if (!runtimeReady) return;
    if (session === null) {
      setUser(null);
      setIsRestoring(false);
      return;
    }

    const controller = new AbortController();
    let cancelled = false;

    void (async () => {
      try {
        if (client.tokenStore.expiresWithin(PROACTIVE_REFRESH_BUFFER_MS)) {
          await client.refreshSession();
          return;
        }

        const current = await client.getSession(controller.signal);
        if (!cancelled) {
          setUser({
            id: current.userId,
            username: current.username,
            permissions: current.permissions,
          });
        }
      } catch (error: unknown) {
        if (cancelled) return;
        if (error instanceof DOMException && error.name === "AbortError") return;
        if (
          error instanceof SessionExpiredError ||
          (error instanceof ApiError && error.isUnauthorized)
        ) {
          await client.tokenStore.clear();
          setUser(null);
        }
      } finally {
        if (!cancelled) setIsRestoring(false);
      }
    })();

    return () => {
      cancelled = true;
      controller.abort();
    };
  }, [client, runtimeReady, session]);

  const signIn = React.useCallback(
    async (username: string, password: string) => {
      const response = await client.login(username, password);
      await client.tokenStore.setTokens({
        accessToken: response.accessToken,
        refreshToken: response.refreshToken,
        expiresAt: response.expiresAt,
      });
    },
    [client],
  );

  const signOut = React.useCallback(async () => {
    const current = client.tokenStore.getSnapshot();
    await client.tokenStore.clear();
    setUser(null);
    if (current !== null) {
      await client.logout(current.refreshToken).catch(() => undefined);
    }
  }, [client]);

  const refresh = React.useCallback(async () => {
    await client.refreshSession();
  }, [client]);

  const value = React.useMemo<AuthContextValue>(
    () => ({
      isAuthenticated: session !== null,
      user,
      isRestoring,
      signIn,
      signOut,
      refresh,
    }),
    [session, user, isRestoring, signIn, signOut, refresh],
  );

  return (
    <ApiClientProvider client={client} connectorSocket={connectorSocket}>
      <AuthContext.Provider value={value}>{children}</AuthContext.Provider>
    </ApiClientProvider>
  );
}

export function useAuth(): AuthContextValue {
  const context = React.useContext(AuthContext);
  if (context === null) {
    throw new Error("useAuth must be used within an AuthProvider");
  }
  return context;
}
