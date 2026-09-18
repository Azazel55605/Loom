import * as React from "react";

import {
  createApiClient,
  type BaseUrlProvider,
  type HttpTransport,
  type Account,
  type PermissionGrant,
} from "@loom/ui-kit/lib/api";
import { ApiClientProvider } from "@loom/ui-kit/lib/api-context";
import { BootErrorScreen, BootScreen } from "@loom/ui-kit/components/BootScreen";
import { ConnectorStatusSocket } from "@loom/ui-kit/lib/connector-socket";
import {
  INITIAL_RETRY_DELAY_MS,
  isSessionRejection,
  nextRetryDelayMs,
} from "@loom/ui-kit/lib/session-failure";
import type { StoredTokens, TokenStorageAdapter } from "@loom/ui-kit/lib/token-store";
import { useConnectionBootstrap } from "@loom/ui-kit/lib/use-connection-bootstrap";
import type { WebSocketTransport } from "@loom/ui-kit/lib/websocket-transport";

const PROACTIVE_REFRESH_BUFFER_MS = 60_000;

export type CurrentUser = {
  id: string;
  username: string;
  permissions: PermissionGrant[];
};

/** A verified login that has not replaced the active persisted session yet. */
export type AuthenticationCandidate = {
  account: Account;
  user: CurrentUser;
  tokens: StoredTokens;
};

/**
 * Whether the stored session is currently unverifiable for network reasons.
 *
 * `"reconnecting"` means the server never answered — the tokens are untouched
 * and a retry is scheduled. It is deliberately *not* reachable from a 401,
 * which clears the session instead and is observed as `isAuthenticated` going
 * false.
 */
export type SessionRecoveryState = "idle" | "reconnecting";

type AuthContextValue = {
  isAuthenticated: boolean;
  user: CurrentUser | null;
  isRestoring: boolean;
  sessionRecovery: SessionRecoveryState;
  /** The configured backend, for screens that name it while reconnecting. */
  serverBaseUrl: string;
  signIn: (username: string, password: string) => Promise<void>;
  authenticateWithoutPersisting: (
    username: string,
    password: string,
  ) => Promise<AuthenticationCandidate>;
  activateAuthentication: (candidate: AuthenticationCandidate) => Promise<void>;
  discardAuthentication: (candidate: AuthenticationCandidate) => Promise<void>;
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
  const [sessionRecovery, setSessionRecovery] =
    React.useState<SessionRecoveryState>("idle");
  const [recoveryAttempt, setRecoveryAttempt] = React.useState(0);
  const recoveryDelayMs = React.useRef(INITIAL_RETRY_DELAY_MS);

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
      setSessionRecovery("idle");
      recoveryDelayMs.current = INITIAL_RETRY_DELAY_MS;
      return;
    }

    const controller = new AbortController();
    let cancelled = false;
    let retryTimer: ReturnType<typeof setTimeout> | null = null;

    void (async () => {
      try {
        if (client.tokenStore.expiresWithin(PROACTIVE_REFRESH_BUFFER_MS)) {
          // The rotated tokens land in the store, which re-runs this effect and
          // reads the session below.
          await client.refreshSession();
          if (!cancelled) {
            setSessionRecovery("idle");
            recoveryDelayMs.current = INITIAL_RETRY_DELAY_MS;
          }
          return;
        }

        const current = await client.getSession(controller.signal);
        if (!cancelled) {
          setUser({
            id: current.userId,
            username: current.username,
            permissions: current.permissions,
          });
          setSessionRecovery("idle");
          recoveryDelayMs.current = INITIAL_RETRY_DELAY_MS;
        }
      } catch (error: unknown) {
        if (cancelled) return;
        if (error instanceof DOMException && error.name === "AbortError") return;
        if (isSessionRejection(error)) {
          // The server itself rejected the refresh token. Only this clears the
          // session and sends the app back to a sign-in prompt.
          await client.tokenStore.clear();
          setUser(null);
          setSessionRecovery("idle");
          recoveryDelayMs.current = INITIAL_RETRY_DELAY_MS;
          return;
        }
        // No answer arrived — DNS, a refused connection, a timeout, a gateway
        // in front of a backend that is still starting. The stored session is
        // very probably still valid, so it is left exactly as it is and the
        // check is repeated with backoff, indefinitely: the device may simply
        // be waiting out an outage, and there may be nobody there to sign in.
        setSessionRecovery("reconnecting");
        const delay = recoveryDelayMs.current;
        recoveryDelayMs.current = nextRetryDelayMs(delay);
        retryTimer = setTimeout(() => {
          retryTimer = null;
          setRecoveryAttempt((current) => current + 1);
        }, delay);
      } finally {
        if (!cancelled) setIsRestoring(false);
      }
    })();

    return () => {
      cancelled = true;
      controller.abort();
      if (retryTimer !== null) clearTimeout(retryTimer);
    };
  }, [client, recoveryAttempt, runtimeReady, session]);

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

  const authenticateWithoutPersisting = React.useCallback(
    async (username: string, password: string): Promise<AuthenticationCandidate> => {
      const response = await client.login(username, password);
      const tokens: StoredTokens = {
        accessToken: response.accessToken,
        refreshToken: response.refreshToken,
        expiresAt: response.expiresAt,
      };
      let transientTokens: StoredTokens | null = tokens;
      const candidateClient = createApiClient({
        baseUrlProvider,
        httpTransport,
        tokenStorage: {
          getTokens: async () => transientTokens,
          setTokens: async (next) => {
            transientTokens = next;
          },
          clearTokens: async () => {
            transientTokens = null;
          },
        },
      });
      await candidateClient.initialize();
      const [account, current] = await Promise.all([
        candidateClient.getAccount(),
        candidateClient.getSession(),
      ]);
      return {
        account,
        tokens,
        user: {
          id: current.userId,
          username: current.username,
          permissions: current.permissions,
        },
      };
    },
    [baseUrlProvider, client, httpTransport],
  );

  const activateAuthentication = React.useCallback(
    async (candidate: AuthenticationCandidate) => {
      await client.tokenStore.setTokens(candidate.tokens);
      setUser(candidate.user);
    },
    [client],
  );

  const discardAuthentication = React.useCallback(
    async (candidate: AuthenticationCandidate) => {
      await client.logout(candidate.tokens.refreshToken).catch(() => undefined);
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
      sessionRecovery,
      serverBaseUrl: bootstrapBaseUrl,
      signIn,
      authenticateWithoutPersisting,
      activateAuthentication,
      discardAuthentication,
      signOut,
      refresh,
    }),
    [
      session,
      user,
      isRestoring,
      sessionRecovery,
      bootstrapBaseUrl,
      signIn,
      authenticateWithoutPersisting,
      activateAuthentication,
      discardAuthentication,
      signOut,
      refresh,
    ],
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
        retrying={bootstrap.retrying}
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
