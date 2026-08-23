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
import { BootErrorScreen, BootScreen } from "@loom/ui-kit/components/BootScreen";
import { ConnectorStatusSocket } from "@loom/ui-kit/lib/connector-socket";
import type { StoredTokens, TokenStorageAdapter } from "@loom/ui-kit/lib/token-store";
import { useConnectionBootstrap } from "@loom/ui-kit/lib/use-connection-bootstrap";
import type { WebSocketTransport } from "@loom/ui-kit/lib/websocket-transport";

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
  webSocketTransport,
  tokenStorage,
  bootstrapBaseUrl,
  onChangeServer,
  children,
}: {
  baseUrlProvider: BaseUrlProvider;
  httpTransport?: HttpTransport;
  webSocketTransport: WebSocketTransport;
  tokenStorage: TokenStorageAdapter;
  bootstrapBaseUrl: string;
  onChangeServer?: () => void | Promise<void>;
  children: React.ReactNode;
}) {
  const [client] = React.useState(() =>
    createApiClient({ baseUrlProvider, httpTransport, tokenStorage }),
  );
  const [connectorSocket] = React.useState(
    () => new ConnectorStatusSocket(client, webSocketTransport),
  );
  const [runtimeReady, setRuntimeReady] = React.useState(false);
  const healthCheck = React.useCallback(
    async (signal?: AbortSignal) => (await client.getHealth(signal)).status === "ok",
    [client],
  );
  const bootstrap = useConnectionBootstrap(bootstrapBaseUrl, healthCheck);
  const session = React.useSyncExternalStore<StoredTokens | null>(
    client.tokenStore.subscribe,
    client.tokenStore.getSnapshot,
    client.tokenStore.getServerSnapshot,
  );
  const [user, setUser] = React.useState<CurrentUser | null>(null);
  const [isRestoring, setIsRestoring] = React.useState(true);

  React.useEffect(() => {
    let cancelled = false;
    if (bootstrap.phase !== "connected") return;
    void client.initialize().finally(() => {
      if (!cancelled) setRuntimeReady(true);
    });
    return () => {
      cancelled = true;
    };
  }, [bootstrap.phase, client]);

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

  if (bootstrap.phase === "idle" || bootstrap.phase === "checking") {
    return <BootScreen baseUrl={bootstrapBaseUrl} />;
  }
  if (bootstrap.phase === "error") {
    return (
      <BootErrorScreen
        baseUrl={bootstrapBaseUrl}
        message={bootstrap.error}
        onRetry={bootstrap.retry}
        onChangeServer={onChangeServer}
      />
    );
  }

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
