import * as React from "react";

import {
  ApiError,
  getSession,
  login as loginRequest,
  logout as logoutRequest,
  refreshSession,
  SessionExpiredError,
  type PermissionGrant,
} from "@/lib/api";
import {
  expiresWithin,
  getSession as getStoredSession,
  setSession,
  subscribe,
  type Session,
} from "@/lib/token-store";

/**
 * Session state for the app.
 *
 * The tokens themselves live in `token-store`, not here, because the API client
 * has to read and rotate them from outside React — see that module for why, and
 * for the note on `localStorage` versus `httpOnly` cookies, which is a decision
 * still worth making deliberately rather than inheriting.
 *
 * What this adds is the React-facing half: subscribing so components re-render
 * when the session changes, resolving who the user is, and the proactive
 * refresh that keeps a signed-in user from being interrupted.
 */

/**
 * How close to expiry triggers a proactive refresh on mount and on navigation.
 *
 * The API client applies the same buffer per request. Doing it here as well
 * means an idle tab that comes back to the foreground renews before the user
 * touches anything, rather than making their first click pay for a round trip
 * that fails and retries.
 */
const PROACTIVE_REFRESH_BUFFER_MS = 60_000;

/** Who the signed-in user is, once their token has been read. */
export type CurrentUser = {
  id: string;
  username: string;
  /**
   * The user's grants, for hiding controls they cannot operate.
   *
   * A convenience, never a control: the server decides what is permitted, and
   * it does enforce these — see the permission enforcement section of
   * docs/API_CONTRACT.md. Use `hasPermission` in `lib/permissions` to read
   * this, and expect a 403 anyway.
   */
  permissions: PermissionGrant[];
};

type AuthContextValue = {
  /** True when a session exists. Says nothing about whether it still works. */
  isAuthenticated: boolean;
  /** The signed-in user, once resolved. Null while restoring or signed out. */
  user: CurrentUser | null;
  /**
   * True while a session restored from storage is being validated on startup.
   *
   * Routing must wait for this: redirecting on "not authenticated" before the
   * check resolves would bounce a signed-in user to the login screen on every
   * reload.
   */
  isRestoring: boolean;
  /** Exchanges credentials for a token pair. Throws `ApiError` on rejection. */
  signIn: (username: string, password: string) => Promise<void>;
  /** Revokes the refresh token server-side, then clears local state. */
  signOut: () => Promise<void>;
  /** Forces a refresh now. Rarely needed — the client refreshes on its own. */
  refresh: () => Promise<void>;
};

const AuthContext = React.createContext<AuthContextValue | null>(null);

export function AuthProvider({ children }: { children: React.ReactNode }) {
  // `useSyncExternalStore` rather than local state, because the session is
  // written from inside the fetch wrapper — on a rotation the store changes
  // without any component having called a setter.
  const session = React.useSyncExternalStore<Session | null>(
    subscribe,
    getStoredSession,
    () => null,
  );

  const [user, setUser] = React.useState<CurrentUser | null>(null);
  const [isRestoring, setIsRestoring] = React.useState(() => getStoredSession() !== null);

  // Resolve the user behind the current session, and renew it if it is close to
  // expiring. Runs on mount and whenever the access token changes — so after a
  // rotation the permissions shown are the ones in the new token, not the old.
  React.useEffect(() => {
    if (session === null) {
      setUser(null);
      setIsRestoring(false);
      return;
    }

    const controller = new AbortController();
    let cancelled = false;

    (async () => {
      try {
        // Renew before asking, so a token that is about to lapse does not make
        // the first request of a restored session fail and retry.
        if (expiresWithin(PROACTIVE_REFRESH_BUFFER_MS)) {
          await refreshSession();
          // The rotation republishes the store, which re-runs this effect with
          // the new token. Stop here rather than using the token we just
          // replaced.
          return;
        }

        const current = await getSession(controller.signal);
        if (cancelled) return;

        setUser({
          id: current.userId,
          username: current.username,
          permissions: current.permissions,
        });
      } catch (error: unknown) {
        if (cancelled) return;
        if (error instanceof DOMException && error.name === "AbortError") return;

        // The session is genuinely over — the refresh token was spent, revoked,
        // or expired. `refreshSession` has already cleared the store; clearing
        // again is harmless and makes the intent explicit.
        if (error instanceof SessionExpiredError) {
          setSession(null);
          setUser(null);
        } else if (error instanceof ApiError && error.isUnauthorized) {
          setSession(null);
          setUser(null);
        }
        // Anything else — backend down, network out — says nothing about the
        // session's validity, so it is left alone. Signing the user out for an
        // unrelated outage would be the wrong call.
      } finally {
        if (!cancelled) setIsRestoring(false);
      }
    })();

    return () => {
      cancelled = true;
      controller.abort();
    };
    // `session` is a fresh object on every store write, so this re-runs after a
    // rotation and the permissions shown come from the new token.
  }, [session]);

  const signIn = React.useCallback(async (username: string, password: string) => {
    const response = await loginRequest(username, password);
    // Publishing the session lets the effect above resolve the user, so there
    // is one code path for "who am I" whether it came from a fresh login or a
    // restored session.
    setSession({
      accessToken: response.accessToken,
      refreshToken: response.refreshToken,
      expiresAt: response.expiresAt,
    });
  }, []);

  const signOut = React.useCallback(async () => {
    const current = getStoredSession();

    // Clear locally first, and unconditionally. Signing out must work when the
    // backend is unreachable — a user who cannot reach the server still wants
    // their tokens off this machine, which is the part they can actually
    // control.
    setSession(null);
    setUser(null);

    if (current !== null) {
      // Revoke server-side so the refresh token cannot be reused. Best-effort:
      // the local session is already gone, and a failure here leaves a token
      // that expires on its own within seven days.
      await logoutRequest(current.refreshToken).catch(() => undefined);
    }
  }, []);

  const refresh = React.useCallback(async () => {
    await refreshSession();
  }, []);

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

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

/** Access the session. Throws outside an `AuthProvider`, which is a wiring bug
 *  rather than a runtime condition worth handling. */
export function useAuth(): AuthContextValue {
  const context = React.useContext(AuthContext);
  if (context === null) {
    throw new Error("useAuth must be used within an AuthProvider");
  }
  return context;
}
