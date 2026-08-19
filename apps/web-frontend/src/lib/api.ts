/**
 * Typed client for the web-backend API.
 *
 * Every shape here mirrors `docs/API_CONTRACT.md`, which is itself derived from
 * the serde output of the Rust structs in `crates/core/src/connector/`. Field
 * names are `camelCase` throughout because every wire type carries
 * `#[serde(rename_all = "camelCase")]` — the single exception is `/health`,
 * which predates that convention and still emits `core_version`. That
 * inconsistency is documented in the contract; it is mirrored faithfully here
 * rather than papered over, so this file keeps telling the truth about what the
 * backend actually sends.
 *
 * These types are **hand-mirrored** from the Rust structs. That is fine while
 * the surface is this small, and it is checked by review against the contract
 * doc. If it starts drifting in practice — a renamed field that typechecks
 * cleanly here and fails at runtime — the answer is to generate them from Core
 * instead, via `ts-rs` or `specta`, rather than to mirror harder. Not
 * implemented now; flagged so the decision is made deliberately when the pain
 * shows up.
 *
 * Authentication is real: see `docs/adr/0008-auth-model.md`. Every
 * authenticated call goes through one wrapper, which attaches the access token
 * and transparently refreshes it once on a 401 — see `authorizedRequest`.
 */

import {
  expiresWithin,
  getAccessToken,
  getSession as getStoredSession,
  setSession,
  type Session,
} from "@/lib/token-store";

/**
 * Base URL of the API.
 *
 * Relative by default: the frontend's own server (nginx in production, the Vite
 * dev server locally) proxies `/api` to the backend and strips the prefix, so
 * requests stay same-origin and no backend host is compiled into the bundle.
 * See docs/adr/0006-frontend-api-same-origin.md.
 *
 * `VITE_API_URL` overrides it with an absolute URL for deployments without that
 * proxy; those requests are cross-origin and rely on the backend's CORS policy.
 */
export const API_URL = resolveApiUrl();

/**
 * `??` is deliberately not used: a declared-but-empty `VITE_API_URL` (which is
 * what an unset Docker build arg produces) arrives as `""`, and `""` is neither
 * null nor undefined, so it would silently win and every request would go to
 * the wrong path.
 */
function resolveApiUrl(): string {
  const configured = import.meta.env.VITE_API_URL?.trim();
  return configured ? configured : "/api";
}

/* -------------------------------------------------------------------------- */
/* Wire types                                                                  */
/* -------------------------------------------------------------------------- */

/** Coarse verdict on a service, as `ConnectorStatus.health`. */
export type HealthState = "healthy" | "degraded" | "down" | "unknown";

/** How a service is doing, as of `lastChecked`. */
export type ConnectorStatus = {
  health: HealthState;
  /**
   * Connector-specific extras — version strings, queue depths, disk usage.
   * Intentionally unstructured; a client that does not recognise a connector
   * ignores this rather than failing to parse it.
   */
  details: unknown;
  /** RFC 3339, UTC, `Z`-suffixed. Part of the value so a polled reading stays
   *  honest about its own age. */
  lastChecked: string;
};

/** One operation a connector is willing to perform. */
export type ConnectorAction = {
  id: string;
  label: string;
  description: string | null;
  /** JSON Schema for this action's params; `{}` when it takes none. */
  paramsSchema: unknown;
};

/**
 * The outcome of an action the backend actually managed to run.
 *
 * `success: false` on a 200 means the service was reached and declined or
 * failed the request. An HTTP error means Loom never got a verdict at all —
 * these are different things and the UI should not collapse them.
 */
export type ActionResult = {
  success: boolean;
  message: string;
  payload: unknown | null;
};

/** Identifying information for a connector. */
export type ConnectorMetadata = {
  id: string;
  name: string;
  /** Icon identifier, not a URL. `null` when the connector declares none. */
  icon: string | null;
  version: string;
};

/**
 * Externally tagged `ConnectorError`: exactly one key, naming the variant.
 *
 * `internal` is a newtype variant, so its value is a bare string rather than an
 * object — the one asymmetry in the enum.
 */
export type ConnectorError =
  | { unreachable: { reason: string } }
  | { authFailed: { reason: string } }
  | { invalidAction: { actionId: string } }
  | { invalidParams: { actionId: string; reason: string } }
  | { internal: string };

/** One element of `GET /connectors`. */
export type ConnectorSummary = {
  metadata: ConnectorMetadata;
  /** `null` when the connector's own status check failed. */
  status: ConnectorStatus | null;
  /** Present only when `status` is null; absent otherwise. */
  statusError?: ConnectorError;
  /**
   * What this connector can be asked to do right now. May be empty.
   *
   * Delivered with the list rather than fetched per connector, so rendering the
   * dashboard is one request. Treat it as data to render: it can vary with the
   * connector's configuration or the remote service's state, so never hardcode
   * an action id against it.
   */
  actions: ConnectorAction[];
};

/** `GET /setup/status` and `POST /setup` response. */
export type SetupStatus = {
  /** `false` when the instance still needs first-run setup. */
  setupComplete: boolean;
};

/**
 * `POST /setup` request.
 *
 * The stub reads and discards every value. The shape is what the real
 * implementation needs, so the wizard is built against it now.
 */
export type SetupRequest = {
  instanceName: string;
  adminUsername: string;
  adminPassword: string;
};

/**
 * One permission granted to the signed-in user.
 *
 * Scope reads as: both null means every resource of every type; a
 * `resourceType` with a null `resourceId` means every resource of that type;
 * both set means exactly that one resource.
 *
 * Useful for hiding controls the user cannot operate. That is a convenience,
 * **never** a control — the server decides what is permitted, and a client that
 * ignores this array learns nothing it could not learn by trying.
 */
export type PermissionGrant = {
  key: string;
  resourceType: string | null;
  resourceId: string | null;
};

/**
 * Response shared by `POST /auth/login` and `POST /auth/refresh`.
 *
 * The refresh token **rotates**: every successful refresh returns a new one and
 * revokes the one presented. A caller must persist what it receives here; a
 * client that keeps reusing its original refresh token is signed out on its
 * second refresh.
 */
export type TokenResponse = {
  accessToken: string;
  refreshToken: string;
  /**
   * RFC 3339. Refers to the **access** token, which lives 15 minutes — the
   * value to schedule a refresh against. The refresh token's own 7-day expiry
   * is not sent, because a client cannot act on it except by discovering its
   * refresh failed.
   */
  expiresAt: string;
};

/** `GET /auth/session` response for an accepted access token. */
export type SessionResponse = {
  authenticated: boolean;
  userId: string;
  username: string;
  permissions: PermissionGrant[];
};

/**
 * The `/health` response.
 *
 * `core_version` is snake_case, unlike every other field in the API. It
 * predates the camelCase convention and three clients already read it, so
 * renaming it is a deliberate breaking change rather than a tidy-up. Mirrored
 * as-is; see the "Known wart" note in docs/API_CONTRACT.md.
 */
export type Health = {
  status: string;
  core_version: string;
};

/* -------------------------------------------------------------------------- */
/* Errors                                                                      */
/* -------------------------------------------------------------------------- */

/**
 * A non-2xx response from the API.
 *
 * Carries the HTTP status so callers can branch on it — a 401 means the token
 * is gone and the app should return to the login screen, which is different
 * from any other failure and is the one case the UI must special-case.
 */
export class ApiError extends Error {
  readonly status: number;
  readonly connectorError?: ConnectorError;
  /**
   * Whether the response carried an error body of the backend's own shape.
   *
   * This is what separates "the handler ran and rejected you" from "there is no
   * handler". A 404 from `POST /connectors/{id}/actions/{actionId}` naming an
   * unknown action arrives with `{"error": …}`; a 404 because the whole route
   * is absent from the routing table arrives with nothing. The two need
   * different explanations, and the status code alone cannot tell them apart.
   */
  readonly hasErrorBody: boolean;

  constructor(
    status: number,
    message: string,
    options: { connectorError?: ConnectorError; hasErrorBody?: boolean } = {},
  ) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.connectorError = options.connectorError;
    this.hasErrorBody = options.hasErrorBody ?? false;
  }

  /** True when the backend rejected our token and the session is over. */
  get isUnauthorized(): boolean {
    return this.status === 401;
  }

  /**
   * True when a setup attempt lost the race — the instance was already
   * configured.
   *
   * Not an error from the caller's point of view: the end state is the one it
   * was trying to reach, so the right response is to carry on to login.
   */
  get isAlreadyComplete(): boolean {
    return this.status === 409;
  }

  /**
   * True when the route itself does not exist — a 404 with no error body of
   * ours behind it.
   *
   * In practice this means the backend on the other end does not serve this
   * API — an older build, or something else on the port.
   */
  get isMissingRoute(): boolean {
    return this.status === 404 && !this.hasErrorBody;
  }
}

/**
 * What a 404 on an auth or connector route actually means.
 *
 * A 404 with no error body is not a missing record — the route is absent from
 * the routing table, so the backend on the other end is not one that serves
 * this API. In practice: an old build, or something else answering on the port.
 *
 * Reporting the raw 404 sends people looking for a typo in a URL that is
 * correct. This says the real thing instead.
 */
export const MISSING_ROUTE_MESSAGE =
  "This backend does not serve the endpoint the app asked for. It may be an " +
  "older build, or something else may be answering on that port — see " +
  "docs/BUILD.md.";

/** The shared error body: `{ "error": string }`, plus `connectorError` when a
 *  connector produced the failure. */
type ErrorBody = {
  error?: string;
  connectorError?: ConnectorError;
};

/* -------------------------------------------------------------------------- */
/* Transport                                                                   */
/* -------------------------------------------------------------------------- */

type RequestOptions = {
  method?: string;
  /** Bearer token, attached as `Authorization` when present. */
  token?: string | null;
  /** Serialized as the JSON request body when present. */
  body?: unknown;
  signal?: AbortSignal;
};

/**
 * One HTTP round trip. No token handling, no retry.
 *
 * Used directly only by calls that must not trigger a refresh: the unauth
 * endpoints, and the refresh call itself — which would otherwise recurse into
 * itself the moment a refresh token is rejected.
 */
async function request<T>(path: string, options: RequestOptions = {}): Promise<T> {
  const { method = "GET", token, body, signal } = options;

  const headers: Record<string, string> = {};
  if (body !== undefined) headers["Content-Type"] = "application/json";
  if (token) headers["Authorization"] = `Bearer ${token}`;

  const response = await fetch(`${API_URL}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
    signal,
  });

  if (!response.ok) throw await toApiError(response);

  // 204 No Content has no body to parse, and `logout` returns one.
  if (response.status === 204) return undefined as T;

  return (await response.json()) as T;
}

/* -------------------------------------------------------------------------- */
/* Session refresh                                                             */
/* -------------------------------------------------------------------------- */

/**
 * How close to expiry counts as "refresh now".
 *
 * Covers clock skew between browser and backend plus the request's flight time:
 * a token still valid when the check runs may not be by the time it arrives.
 * Refreshing slightly early costs one extra request; refreshing slightly late
 * costs a failed request and a retry.
 */
const REFRESH_BUFFER_MS = 60_000;

/**
 * The refresh in flight, if any.
 *
 * A dashboard fires several queries at once, so several can hit a 401 together.
 * Without this they would each start their own refresh, and because the backend
 * *rotates* refresh tokens the first to land invalidates the token the others
 * are still using — turning one expired access token into a forced sign-out.
 * Everyone awaits the same promise instead.
 */
let inFlightRefresh: Promise<Session> | null = null;

/** Raised when the session is over and only signing in again will fix it. */
export class SessionExpiredError extends Error {
  constructor(message = "Your session has expired. Please sign in again.") {
    super(message);
    this.name = "SessionExpiredError";
  }
}

/**
 * Exchanges the stored refresh token for a fresh pair, once.
 *
 * Concurrent callers share one request. On failure the session is cleared —
 * a rejected refresh token cannot be retried into working, and leaving it in
 * storage would mean retrying it on every subsequent request.
 */
export function refreshSession(): Promise<Session> {
  if (inFlightRefresh !== null) return inFlightRefresh;

  const stored = getStoredSession();
  if (stored === null) return Promise.reject(new SessionExpiredError());

  inFlightRefresh = (async () => {
    try {
      const response = await request<TokenResponse>("/auth/refresh", {
        method: "POST",
        body: { refreshToken: stored.refreshToken },
      });

      const session: Session = {
        accessToken: response.accessToken,
        // The rotated token. Persisting the old one here would sign the user
        // out on their next refresh.
        refreshToken: response.refreshToken,
        expiresAt: response.expiresAt,
      };
      setSession(session);
      return session;
    } catch (error) {
      // A 401 means the refresh token is spent, revoked, or expired: the
      // session is genuinely over. Any other failure — backend down, network
      // out — says nothing about the token, so the session is left alone and
      // the caller sees the real error.
      if (error instanceof ApiError && error.isUnauthorized) {
        setSession(null);
        throw new SessionExpiredError();
      }
      throw error;
    } finally {
      inFlightRefresh = null;
    }
  })();

  return inFlightRefresh;
}

/**
 * An authenticated request, with the token handling every call needs.
 *
 * Refreshes proactively when the access token is within [`REFRESH_BUFFER_MS`]
 * of expiry, and reactively **exactly once** on a 401. One retry, not a loop:
 * if a freshly minted token is also rejected, retrying cannot help and would
 * turn a broken session into a request storm.
 *
 * Every authenticated endpoint goes through here, so the refresh logic lives in
 * one place rather than at each call site.
 */
async function authorizedRequest<T>(
  path: string,
  options: Omit<RequestOptions, "token"> = {},
): Promise<T> {
  if (getStoredSession() === null) throw new SessionExpiredError();

  if (expiresWithin(REFRESH_BUFFER_MS)) {
    // Let a proactive refresh failure fall through to the request below: if the
    // backend is merely unreachable the access token may still be good, and
    // failing here would report the wrong problem. A genuinely dead session
    // surfaces as the 401 handled next.
    await refreshSession().catch(() => undefined);
  }

  try {
    return await request<T>(path, { ...options, token: getAccessToken() });
  } catch (error) {
    if (!(error instanceof ApiError) || !error.isUnauthorized) throw error;

    // The token was rejected despite looking current — expired early, or signed
    // by a backend that has since been rebuilt with a new secret.
    const session = await refreshSession();
    return await request<T>(path, { ...options, token: session.accessToken });
  }
}

/**
 * Turns a failed response into an `ApiError`.
 *
 * Not every error body is Loom's own shape: axum's extractors reject a bad
 * content type (415) or an unparseable body (422) before any handler runs, and
 * those come back as `text/plain`. Parsing is therefore best-effort, falling
 * back to the status line rather than throwing a second error while handling
 * the first.
 */
async function toApiError(response: Response): Promise<ApiError> {
  const fallback = `${response.status} ${response.statusText}`;
  try {
    const text = await response.text();
    if (!text) return new ApiError(response.status, fallback);

    try {
      const parsed = JSON.parse(text) as ErrorBody;
      return new ApiError(response.status, parsed.error ?? fallback, {
        connectorError: parsed.connectorError,
        // Only a body carrying our own `error` field proves a handler produced
        // this. Valid JSON alone does not.
        hasErrorBody: typeof parsed.error === "string",
      });
    } catch {
      // A plain-text rejection from the extractor layer — axum's 415 and 422
      // reject before any handler runs, but they are still deliberate answers
      // about this request rather than a missing route.
      return new ApiError(response.status, text.trim() || fallback, {
        hasErrorBody: true,
      });
    }
  } catch {
    return new ApiError(response.status, fallback);
  }
}

/* -------------------------------------------------------------------------- */
/* Endpoints                                                                   */
/* -------------------------------------------------------------------------- */

/** `GET /health` — unauthenticated, and the one route that predates auth. */
export function getHealth(signal?: AbortSignal): Promise<Health> {
  return request<Health>("/health", { signal });
}

/**
 * `GET /setup/status` — whether this instance still needs first-run setup.
 *
 * Unauthenticated, necessarily: it is asked before anyone can hold a token.
 */
export function getSetupStatus(signal?: AbortSignal): Promise<SetupStatus> {
  return request<SetupStatus>("/setup/status", { signal });
}

/**
 * `POST /setup` — completes first-run setup.
 *
 * Throws an `ApiError` with status 409 when setup was already completed, which
 * a caller should treat as success: the instance is configured, which is the
 * outcome it wanted. See `ApiError.isAlreadyComplete`.
 */
export function completeSetup(
  data: SetupRequest,
  signal?: AbortSignal,
): Promise<SetupStatus> {
  return request<SetupStatus>("/setup", { method: "POST", body: data, signal });
}

/**
 * `POST /auth/login`.
 *
 * Unauthenticated by definition, so it does not go through
 * `authorizedRequest` — there is no session to refresh yet.
 *
 * A 401 means the credentials were rejected. The backend deliberately returns
 * one identical response for a wrong password, an unknown username, and a
 * deactivated account, so there is nothing here to distinguish between them.
 */
export function login(
  username: string,
  password: string,
  signal?: AbortSignal,
): Promise<TokenResponse> {
  return request<TokenResponse>("/auth/login", {
    method: "POST",
    body: { username, password },
    signal,
  });
}

/**
 * `POST /auth/refresh` — exchanges a refresh token for a rotated pair.
 *
 * Low-level and rarely what you want: it neither reads nor writes the stored
 * session. Prefer [`refreshSession`], which does both and deduplicates
 * concurrent callers. This exists for a caller holding a token from somewhere
 * other than the store.
 */
export function refreshTokens(
  refreshToken: string,
  signal?: AbortSignal,
): Promise<TokenResponse> {
  return request<TokenResponse>("/auth/refresh", {
    method: "POST",
    body: { refreshToken },
    signal,
  });
}

/**
 * `POST /auth/logout` — revokes one refresh token server-side.
 *
 * Returns 204 whether or not the token was live, so this resolves rather than
 * throwing for an already-revoked token. Only the presented token is revoked:
 * other devices stay signed in.
 *
 * An access token already issued stays valid until it expires — the backend
 * cannot recall it — so a caller must also discard its local session rather
 * than assume the server stopped honouring what it already handed out.
 */
export function logout(refreshToken: string, signal?: AbortSignal): Promise<void> {
  return request<void>("/auth/logout", {
    method: "POST",
    body: { refreshToken },
    signal,
  });
}

/**
 * `GET /auth/session` — who the current access token belongs to.
 *
 * Answered from the token's claims, so the permission list can lag a change by
 * up to the access token's 15-minute life.
 */
export function getSession(signal?: AbortSignal): Promise<SessionResponse> {
  return authorizedRequest<SessionResponse>("/auth/session", { signal });
}

/** `GET /connectors` — every registered connector with its current status. */
export function getConnectors(signal?: AbortSignal): Promise<ConnectorSummary[]> {
  return authorizedRequest<ConnectorSummary[]>("/connectors", { signal });
}

/**
 * `POST /connectors/{id}/actions/{actionId}`.
 *
 * `params` is forwarded as the request body. Omitting it sends no body at all,
 * which the backend reads as JSON `null` — deliberately distinct from an empty
 * object, so "sent nothing" and "sent {}" stay distinguishable.
 */
export function executeAction(
  connectorId: string,
  actionId: string,
  params?: unknown,
  signal?: AbortSignal,
): Promise<ActionResult> {
  return authorizedRequest<ActionResult>(
    `/connectors/${encodeURIComponent(connectorId)}/actions/${encodeURIComponent(actionId)}`,
    { method: "POST", body: params, signal },
  );
}
