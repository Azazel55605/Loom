import * as React from "react";

import { ApiError, getSession, login as loginRequest } from "@/lib/api";

/**
 * Session state for the frame.
 *
 * ## Where the token lives, and why that is a stub-era decision
 *
 * The token is held in React state and mirrored into `localStorage` so a reload
 * does not throw the user back to the login screen. For a browser SPA talking
 * to a token API that is the conventional arrangement, and it is the right
 * shape to build the frame against.
 *
 * It is **not** the right security posture for real auth, and it should be
 * revisited when ADR 0003's auth model is implemented rather than inherited by
 * default. `localStorage` is readable by any script running on the origin, so a
 * single XSS bug hands over a long-lived credential; an `httpOnly` cookie is
 * not, which is why it is the usual recommendation for real sessions. The
 * trade-off is not free — cookies are sent automatically, so they need CSRF
 * defences and a same-site policy, and they interact with the CORS policy in
 * ADR 0005 and with the Tauri clients, which are never same-origin.
 *
 * That decision needs to be made deliberately, with those constraints on the
 * table. Today none of it matters: the token is the fixed public string
 * `dev-stub-token`, which the backend hands to anyone who asks, so there is
 * nothing here worth stealing.
 */

/** Storage key for the persisted token. Namespaced to avoid colliding with
 *  anything else served from the same origin. */
const TOKEN_STORAGE_KEY = "loom.auth.token";

type AuthContextValue = {
  /** The current bearer token, or null when signed out. */
  token: string | null;
  /** The authenticated user's identifier, once a session has been validated. */
  user: string | null;
  /**
   * True while a token restored from storage is being validated on startup.
   *
   * Routing must wait for this: redirecting on `token === null` before the
   * check resolves would bounce a signed-in user to the login screen on every
   * reload.
   */
  isRestoring: boolean;
  /** Exchanges credentials for a token and stores it. Throws `ApiError`. */
  signIn: (username: string, password: string) => Promise<void>;
  /** Clears the token locally. There is no server-side revocation to call. */
  signOut: () => void;
};

const AuthContext = React.createContext<AuthContextValue | null>(null);

function readStoredToken(): string | null {
  try {
    return window.localStorage.getItem(TOKEN_STORAGE_KEY);
  } catch {
    // Private browsing modes and strict cookie settings can make localStorage
    // throw on access. That costs persistence, not function.
    return null;
  }
}

function writeStoredToken(token: string | null): void {
  try {
    if (token === null) window.localStorage.removeItem(TOKEN_STORAGE_KEY);
    else window.localStorage.setItem(TOKEN_STORAGE_KEY, token);
  } catch {
    // As above: persistence is best-effort, the session still works in memory.
  }
}

export function AuthProvider({ children }: { children: React.ReactNode }) {
  const [token, setToken] = React.useState<string | null>(() => readStoredToken());
  const [user, setUser] = React.useState<string | null>(null);
  // Only a token restored from storage needs validating; with nothing stored
  // there is nothing to wait for and the login screen can render immediately.
  const [isRestoring, setIsRestoring] = React.useState(() => readStoredToken() !== null);

  // Validate a restored token exactly once on mount. A token that outlived its
  // expiry, or one issued by a different backend, must not be trusted just
  // because it is present in storage.
  React.useEffect(() => {
    const stored = readStoredToken();
    if (stored === null) return;

    const controller = new AbortController();
    let cancelled = false;

    getSession(stored, controller.signal)
      .then((session) => {
        if (cancelled) return;
        setUser(session.user);
        setIsRestoring(false);
      })
      .catch((error: unknown) => {
        if (cancelled) return;
        if (error instanceof DOMException && error.name === "AbortError") return;

        // A 401 means the token is genuinely no longer good, so drop it. Any
        // other failure — the backend is down, the network is out — says
        // nothing about the token, and discarding it would sign the user out
        // for an unrelated outage.
        if (error instanceof ApiError && error.isUnauthorized) {
          writeStoredToken(null);
          setToken(null);
        }
        setIsRestoring(false);
      });

    return () => {
      cancelled = true;
      controller.abort();
    };
  }, []);

  const signIn = React.useCallback(async (username: string, password: string) => {
    const response = await loginRequest(username, password);
    writeStoredToken(response.token);
    setToken(response.token);
    // The login response carries no identity, and the stub has only one user.
    // Ask the session endpoint rather than inventing a name here, so this stays
    // correct once login returns a real one.
    try {
      const session = await getSession(response.token);
      setUser(session.user);
    } catch {
      // Signed in regardless; the display name is not worth failing over.
      setUser(null);
    }
  }, []);

  const signOut = React.useCallback(() => {
    writeStoredToken(null);
    setToken(null);
    setUser(null);
  }, []);

  const value = React.useMemo<AuthContextValue>(
    () => ({ token, user, isRestoring, signIn, signOut }),
    [token, user, isRestoring, signIn, signOut],
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
