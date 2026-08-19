/**
 * The session's tokens: held in memory, mirrored to `localStorage`.
 *
 * A module rather than React state, because the API client needs to read the
 * access token and write a rotated pair from inside a plain `fetch` wrapper —
 * outside any component, and without importing the auth context, which would
 * make the dependency circular. The context subscribes to this instead, so
 * React still re-renders when the session changes.
 *
 * ## Where the tokens live, and why that is worth revisiting
 *
 * `localStorage` is the conventional arrangement for a browser SPA holding
 * bearer tokens, and it is what makes a reload keep you signed in. It is not
 * the strongest option available, and this choice should be made deliberately
 * rather than inherited: `localStorage` is readable by any script running on
 * the origin, so a single XSS bug hands over a live session. An `httpOnly`
 * cookie is not script-readable, which is why it is the usual recommendation.
 *
 * The trade-off is not free. Cookies are sent automatically, so they need CSRF
 * defences and a same-site policy, and they interact with the CORS policy in
 * ADR 0005 and with the Tauri clients, which are never same-origin with the
 * backend. That decision needs those constraints on the table.
 *
 * What has changed since the stub era is that this is now worth protecting: the
 * refresh token is a real seven-day credential rather than a fixed public
 * string. Two things blunt the exposure — the access token lives 15 minutes,
 * and refresh tokens rotate on every use, so a stolen one is usable at most
 * once and its use is detectable when the legitimate holder's next refresh
 * fails. Neither is a substitute for deciding where these belong.
 */

/** One storage key holding the whole session, so a partial write cannot leave
 *  an access token without its refresh token. */
const STORAGE_KEY = "loom.auth.session";

/** The tokens and when the access token stops being valid. */
export type Session = {
  accessToken: string;
  refreshToken: string;
  /** RFC 3339, from the backend. Refers to the **access** token. */
  expiresAt: string;
};

type Listener = (session: Session | null) => void;

let current: Session | null = readStored();
const listeners = new Set<Listener>();

function readStored(): Session | null {
  try {
    const raw = window.localStorage.getItem(STORAGE_KEY);
    if (raw === null) return null;

    const parsed = JSON.parse(raw) as Partial<Session>;
    // Validate rather than trust: this is data from a previous version of the
    // app as much as from this one. A session persisted by the old
    // single-token build has no `refreshToken`, and treating it as valid would
    // leave the app unable to refresh and unable to explain why.
    if (
      typeof parsed.accessToken === "string" &&
      typeof parsed.refreshToken === "string" &&
      typeof parsed.expiresAt === "string"
    ) {
      return parsed as Session;
    }
    // Anything else is stale or corrupt. Drop it so the user gets a clean
    // login rather than a session that cannot work.
    window.localStorage.removeItem(STORAGE_KEY);
    return null;
  } catch {
    // Private browsing and strict cookie settings can make localStorage throw
    // on access. That costs persistence, not function.
    return null;
  }
}

function persist(session: Session | null): void {
  try {
    if (session === null) window.localStorage.removeItem(STORAGE_KEY);
    else window.localStorage.setItem(STORAGE_KEY, JSON.stringify(session));
  } catch {
    // As above: persistence is best-effort, the session still works in memory.
  }
}

/** The current session, or null when signed out. */
export function getSession(): Session | null {
  return current;
}

/** The access token to send, or null when signed out. */
export function getAccessToken(): string | null {
  return current?.accessToken ?? null;
}

/** Replaces the session and notifies subscribers. Pass null to sign out. */
export function setSession(session: Session | null): void {
  current = session;
  persist(session);
  for (const listener of listeners) listener(session);
}

/**
 * Subscribes to session changes. Returns an unsubscribe function.
 *
 * Shaped for React's `useSyncExternalStore`, which is what keeps the context in
 * step with writes made from inside the fetch wrapper.
 */
export function subscribe(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Whether the access token expires within `bufferMs`.
 *
 * The buffer exists because a token that is valid *now* may not be by the time
 * the request reaches the backend — clock skew between browser and server, plus
 * flight time. Refreshing slightly early costs one extra request; refreshing
 * slightly late costs a failed request and a retry.
 */
export function expiresWithin(bufferMs: number): boolean {
  if (current === null) return false;

  const expiry = Date.parse(current.expiresAt);
  // An unparseable expiry means we cannot tell — treat it as due, so the app
  // refreshes and re-establishes a timestamp it understands instead of
  // trusting a value it cannot read.
  if (Number.isNaN(expiry)) return true;

  return expiry - Date.now() <= bufferMs;
}
