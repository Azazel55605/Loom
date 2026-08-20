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

import { TokenStore, type StoredTokens, type TokenStorageAdapter } from "@loom/ui-kit/lib/token-store";

/** Platform-owned resolution of the backend URL or proxy prefix. */
export interface BaseUrlProvider {
  getBaseUrl(): Promise<string>;
}

type ApiRuntime = {
  readonly baseUrlProvider: BaseUrlProvider;
  readonly tokenStore: TokenStore;
  baseUrl: string | null;
  initialization: Promise<void> | null;
  inFlightRefresh: Promise<StoredTokens> | null;
};

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
 *
 * Note the deliberate asymmetry when reading it: the server treats a *scoped*
 * grant as not satisfying a *global* check, so holding `connectors.control`
 * over one connector is not authority over connectors in general.
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
   * True when the caller is authenticated and still not allowed.
   *
   * Distinct from `isUnauthorized` in the one way that matters to a client:
   * refreshing the token cannot fix it. The transport must never retry a 403
   * through a refresh, or a missing grant turns into a refresh loop.
   */
  get isForbidden(): boolean {
    return this.status === 403;
  }

  /**
   * True when the request was well-formed and refused because of the state it
   * would produce — a taken username, or one of the administration safeguards.
   *
   * The message is written for a person and should be shown as-is rather than
   * replaced with a generic failure: "that would leave no active administrator"
   * tells the user what to do next, and "something went wrong" does not.
   */
  get isConflict(): boolean {
    return this.status === 409;
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
  /**
   * The request body.
   *
   * A `FormData` is passed through untouched; anything else is serialized as
   * JSON. The distinction matters because a multipart body's `Content-Type`
   * carries a boundary that only the browser can generate — setting the header
   * ourselves would produce one without a boundary, and the server would fail
   * to parse a body that is perfectly well-formed.
   */
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
async function initializeRuntime(runtime: ApiRuntime): Promise<void> {
  if (runtime.initialization !== null) return runtime.initialization;
  runtime.initialization = Promise.all([
    runtime.baseUrlProvider.getBaseUrl(),
    runtime.tokenStore.initialize(),
  ]).then(([baseUrl]) => {
    runtime.baseUrl = baseUrl.replace(/\/$/, "");
  });
  return runtime.initialization;
}

async function request<T>(
  runtime: ApiRuntime,
  path: string,
  options: RequestOptions = {},
): Promise<T> {
  await initializeRuntime(runtime);
  const { method = "GET", token, body, signal } = options;

  const isFormData = body instanceof FormData;

  const headers: Record<string, string> = {};
  // Deliberately not set for FormData — see `RequestOptions.body`.
  if (body !== undefined && !isFormData) headers["Content-Type"] = "application/json";
  if (token) headers["Authorization"] = `Bearer ${token}`;

  const response = await fetch(`${runtime.baseUrl ?? ""}${path}`, {
    method,
    headers,
    body: body === undefined ? undefined : isFormData ? body : JSON.stringify(body),
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
function refreshSession(runtime: ApiRuntime): Promise<StoredTokens> {
  if (runtime.inFlightRefresh !== null) return runtime.inFlightRefresh;

  const stored = runtime.tokenStore.getSnapshot();
  if (stored === null) return Promise.reject(new SessionExpiredError());

  runtime.inFlightRefresh = (async () => {
    try {
      const response = await request<TokenResponse>(runtime, "/auth/refresh", {
        method: "POST",
        body: { refreshToken: stored.refreshToken },
      });

      const session: StoredTokens = {
        accessToken: response.accessToken,
        // The rotated token. Persisting the old one here would sign the user
        // out on their next refresh.
        refreshToken: response.refreshToken,
        expiresAt: response.expiresAt,
      };
      await runtime.tokenStore.setTokens(session);
      return session;
    } catch (error) {
      // A 401 means the refresh token is spent, revoked, or expired: the
      // session is genuinely over. Any other failure — backend down, network
      // out — says nothing about the token, so the session is left alone and
      // the caller sees the real error.
      if (error instanceof ApiError && error.isUnauthorized) {
        await runtime.tokenStore.clear();
        throw new SessionExpiredError();
      }
      throw error;
    } finally {
      runtime.inFlightRefresh = null;
    }
  })();

  return runtime.inFlightRefresh;
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
  runtime: ApiRuntime,
  path: string,
  options: Omit<RequestOptions, "token"> & { retryOnUnauthorized?: boolean } = {},
): Promise<T> {
  const { retryOnUnauthorized = true, ...requestOptions } = options;

  await initializeRuntime(runtime);
  if (runtime.tokenStore.getSnapshot() === null) throw new SessionExpiredError();

  if (runtime.tokenStore.expiresWithin(REFRESH_BUFFER_MS)) {
    // Let a proactive refresh failure fall through to the request below: if the
    // backend is merely unreachable the access token may still be good, and
    // failing here would report the wrong problem. A genuinely dead session
    // surfaces as the 401 handled next.
    await refreshSession(runtime).catch(() => undefined);
  }

  try {
    return await request<T>(runtime, path, {
      ...requestOptions,
      token: runtime.tokenStore.getAccessToken(),
    });
  } catch (error) {
    if (!(error instanceof ApiError) || !error.isUnauthorized) throw error;

    // Not every 401 is about the token. `POST /account/password` returns one
    // for a wrong *current password*, and treating that as an expired session
    // would be actively harmful: the client would burn a refresh, retry, get
    // the same 401, and surface it as "your session expired" — signing the user
    // out because they mistyped a password. Callers whose 401 means something
    // else opt out here.
    if (!retryOnUnauthorized) throw error;

    // The token was rejected despite looking current — expired early, or signed
    // by a backend that has since been rebuilt with a new secret.
    const session = await refreshSession(runtime);
    return await request<T>(runtime, path, {
      ...requestOptions,
      token: session.accessToken,
    });
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
function getHealth(runtime: ApiRuntime, signal?: AbortSignal): Promise<Health> {
  return request<Health>(runtime, "/health", { signal });
}

/**
 * `GET /setup/status` — whether this instance still needs first-run setup.
 *
 * Unauthenticated, necessarily: it is asked before anyone can hold a token.
 */
function getSetupStatus(runtime: ApiRuntime, signal?: AbortSignal): Promise<SetupStatus> {
  return request<SetupStatus>(runtime, "/setup/status", { signal });
}

/**
 * `POST /setup` — completes first-run setup.
 *
 * Throws an `ApiError` with status 409 when setup was already completed, which
 * a caller should treat as success: the instance is configured, which is the
 * outcome it wanted. See `ApiError.isAlreadyComplete`.
 */
function completeSetup(runtime: ApiRuntime, 
  data: SetupRequest,
  signal?: AbortSignal,
): Promise<SetupStatus> {
  return request<SetupStatus>(runtime, "/setup", { method: "POST", body: data, signal });
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
function login(runtime: ApiRuntime, 
  username: string,
  password: string,
  signal?: AbortSignal,
): Promise<TokenResponse> {
  return request<TokenResponse>(runtime, "/auth/login", {
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
function refreshTokens(runtime: ApiRuntime, 
  refreshToken: string,
  signal?: AbortSignal,
): Promise<TokenResponse> {
  return request<TokenResponse>(runtime, "/auth/refresh", {
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
function logout(runtime: ApiRuntime, refreshToken: string, signal?: AbortSignal): Promise<void> {
  return request<void>(runtime, "/auth/logout", {
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
function getSession(runtime: ApiRuntime, signal?: AbortSignal): Promise<SessionResponse> {
  return authorizedRequest<SessionResponse>(runtime, "/auth/session", { signal });
}

/** `GET /connectors` — every registered connector with its current status. */
function getConnectors(runtime: ApiRuntime, signal?: AbortSignal): Promise<ConnectorSummary[]> {
  return authorizedRequest<ConnectorSummary[]>(runtime, "/connectors", { signal });
}

/**
 * `POST /connectors/{id}/actions/{actionId}`.
 *
 * `params` is forwarded as the request body. Omitting it sends no body at all,
 * which the backend reads as JSON `null` — deliberately distinct from an empty
 * object, so "sent nothing" and "sent {}" stay distinguishable.
 */
function executeAction(runtime: ApiRuntime, 
  connectorId: string,
  actionId: string,
  params?: unknown,
  signal?: AbortSignal,
): Promise<ActionResult> {
  return authorizedRequest<ActionResult>(runtime, 
    `/connectors/${encodeURIComponent(connectorId)}/actions/${encodeURIComponent(actionId)}`,
    { method: "POST", body: params, signal },
  );
}

/* -------------------------------------------------------------------------- */
/* Administration                                                              */
/* -------------------------------------------------------------------------- */

/**
 * A user account, as returned by every `/users` route.
 *
 * There is **no password field, and there must never be one** — see the note in
 * docs/API_CONTRACT.md. If one ever appears in a response, that is a backend
 * bug to fix rather than a field to mirror here.
 */
export type User = {
  id: string;
  username: string;
  isActive: boolean;
  /** RFC 3339. */
  createdAt: string;
  /** The groups this user belongs to. Membership is stated wholesale, never as
   *  a delta. */
  groupIds: string[];
};

/** `POST /users` body. `groupIds` may be omitted — an account with no groups
 *  can sign in and do nothing, which is a valid state. */
export type CreateUserRequest = {
  username: string;
  password: string;
  groupIds?: string[];
};

/**
 * `PATCH /users/{id}` body. Every field is optional; an absent field is left
 * alone, and `groupIds` **replaces** membership rather than adding to it.
 *
 * Note the difference between absent and empty: omitting `groupIds` keeps the
 * current groups, sending `[]` removes them all.
 */
export type UpdateUserRequest = {
  isActive?: boolean;
  groupIds?: string[];
};

/** A group with its grants, as returned by every `/groups` route. */
export type Group = {
  id: string;
  name: string;
  description: string | null;
  /** RFC 3339. */
  createdAt: string;
  /** True for a group that cannot be deleted. Hide or disable the delete
   *  control rather than letting the user discover it through a 409. */
  isProtected: boolean;
  memberCount: number;
  permissions: PermissionGrant[];
};

/** `POST /groups` body. New groups are never protected. */
export type CreateGroupRequest = {
  name: string;
  description: string | null;
  permissions: PermissionGrant[];
};

/** `PATCH /groups/{id}` body. All fields optional; `permissions` replaces the
 *  group's grants wholesale, on the same reasoning as user membership. */
export type UpdateGroupRequest = {
  name?: string;
  description?: string | null;
  permissions?: PermissionGrant[];
};

/**
 * One entry of the permission catalog from `GET /permissions`.
 *
 * The catalog exists so a client can build a grant-assignment form without
 * hardcoding a list that falls out of date the next time a migration registers
 * a key. Treat it as the authoritative set, not `PERMISSION_KEYS`.
 */
export type PermissionCatalogEntry = {
  key: string;
  description: string;
};

/** `GET /users` — requires a global `users.manage` grant. */
function getUsers(runtime: ApiRuntime, signal?: AbortSignal): Promise<User[]> {
  return authorizedRequest<User[]>(runtime, "/users", { signal });
}

/**
 * `POST /users` — creates an account.
 *
 * 400 for an empty username, a password under 8 characters, or an unknown group
 * id; 409 when the username is taken.
 */
function createUser(runtime: ApiRuntime, 
  data: CreateUserRequest,
  signal?: AbortSignal,
): Promise<User> {
  return authorizedRequest<User>(runtime, "/users", { method: "POST", body: data, signal });
}

/**
 * `PATCH /users/{id}`.
 *
 * A 409 here is a safeguard (see docs/API_CONTRACT.md): the change
 * would leave no active administrator, or the caller is trying to modify their
 * own account. Show the backend's message — it says which.
 */
function updateUser(runtime: ApiRuntime, 
  id: string,
  data: UpdateUserRequest,
  signal?: AbortSignal,
): Promise<User> {
  return authorizedRequest<User>(runtime, `/users/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: data,
    signal,
  });
}

/**
 * `DELETE /users/{id}` — a hard delete, taking the user's group memberships and
 * refresh tokens with it, which also ends their sessions.
 *
 * Subject to the same safeguards as `updateUser`, with the same 409.
 */
function deleteUser(runtime: ApiRuntime, id: string, signal?: AbortSignal): Promise<void> {
  return authorizedRequest<void>(runtime, `/users/${encodeURIComponent(id)}`, {
    method: "DELETE",
    signal,
  });
}

/** `GET /groups` — requires a global `groups.manage` grant. */
function getGroups(runtime: ApiRuntime, signal?: AbortSignal): Promise<Group[]> {
  return authorizedRequest<Group[]>(runtime, "/groups", { signal });
}

/** `POST /groups`. 400 for an empty name or an unregistered permission key;
 *  409 when the name is taken. */
function createGroup(runtime: ApiRuntime, 
  data: CreateGroupRequest,
  signal?: AbortSignal,
): Promise<Group> {
  return authorizedRequest<Group>(runtime, "/groups", { method: "POST", body: data, signal });
}

/** `PATCH /groups/{id}`. A protected group may be renamed and re-granted —
 *  only deletion is refused. */
function updateGroup(runtime: ApiRuntime, 
  id: string,
  data: UpdateGroupRequest,
  signal?: AbortSignal,
): Promise<Group> {
  return authorizedRequest<Group>(runtime, `/groups/${encodeURIComponent(id)}`, {
    method: "PATCH",
    body: data,
    signal,
  });
}

/** `DELETE /groups/{id}`. 409 when the group is protected — which a client
 *  should have prevented by reading `isProtected`. */
function deleteGroup(runtime: ApiRuntime, id: string, signal?: AbortSignal): Promise<void> {
  return authorizedRequest<void>(runtime, `/groups/${encodeURIComponent(id)}`, {
    method: "DELETE",
    signal,
  });
}

/** `GET /permissions` — the catalog of registered keys. Requires
 *  `groups.manage`, since assigning grants is the only thing it is for. */
function getPermissions(runtime: ApiRuntime, 
  signal?: AbortSignal,
): Promise<PermissionCatalogEntry[]> {
  return authorizedRequest<PermissionCatalogEntry[]>(runtime, "/permissions", { signal });
}

/* -------------------------------------------------------------------------- */
/* Account (self-service)                                                      */
/* -------------------------------------------------------------------------- */

/**
 * A group the signed-in user belongs to, as reported by `GET /account`.
 *
 * Read-only context. Membership is changed through the admin `/users/{id}`
 * route, never from here — see the Account section of docs/API_CONTRACT.md.
 */
export type AccountGroup = {
  id: string;
  name: string;
};

/** The signed-in user's own profile. As everywhere, no password field. */
export type Account = {
  id: string;
  username: string;
  displayName: string | null;
  /**
   * A **relative** path like `/avatars/{uuid}.png`, or null when unset.
   *
   * Resolve it against the same base as the API: the backend cannot know the
   * origin it is reached through, so it never sends an absolute URL. Use
   * [`avatarSrc`] rather than reading this straight into an `<img src>`, or the
   * web frontend will request it on its own origin instead of through the proxy.
   */
  avatarUrl: string | null;
  createdAt: string;
  groups: AccountGroup[];
};

/**
 * `PATCH /account` body. Both optional; an absent field is left alone.
 *
 * `displayName: null` clears it — note that this is distinct from omitting the
 * field, which keeps the current value.
 */
export type UpdateAccountRequest = {
  username?: string;
  displayName?: string | null;
};

/** `POST /account/avatar` response. */
export type AvatarUploadResponse = {
  avatarUrl: string;
};

/**
 * Turns an `avatarUrl` from the API into something an `<img>` can load.
 *
 * The backend serves avatars at `/avatars/…`, and every other path in this
 * client is reached through [`API_URL`] — `/api` for the browser, an absolute
 * server URL for the desktop and mobile clients. The avatar is no different, so
 * it gets the same prefix. Reading `avatarUrl` directly would work only in the
 * one deployment where the frontend and backend share an origin *and* no proxy
 * prefix is in play.
 */
function avatarSrc(runtime: ApiRuntime, avatarUrl: string): string {
  return `${runtime.baseUrl ?? ""}${avatarUrl}`;
}

/** `GET /account` — the caller's own profile. Needs a token, no permission. */
function getAccount(runtime: ApiRuntime, signal?: AbortSignal): Promise<Account> {
  return authorizedRequest<Account>(runtime, "/account", { signal });
}

/**
 * `PATCH /account` — change your own username and/or display name.
 *
 * Throws an `ApiError` with status 409 when the username is taken by another
 * account; the check excludes your own row, so resubmitting your current
 * username is not a conflict.
 */
function updateAccount(runtime: ApiRuntime, 
  data: UpdateAccountRequest,
  signal?: AbortSignal,
): Promise<Account> {
  return authorizedRequest<Account>(runtime, "/account", {
    method: "PATCH",
    body: data,
    signal,
  });
}

/**
 * `POST /account/password`.
 *
 * A 401 here means `currentPassword` was wrong — **not** that the session
 * expired. That distinction matters to the transport as much as to the UI: see
 * the note in `authorizedRequest` about why this call opts out of the automatic
 * refresh-and-retry.
 */
function changePassword(runtime: ApiRuntime, 
  currentPassword: string,
  newPassword: string,
  signal?: AbortSignal,
): Promise<void> {
  return authorizedRequest<void>(runtime, "/account/password", {
    method: "POST",
    body: { currentPassword, newPassword },
    signal,
    retryOnUnauthorized: false,
  });
}

/**
 * `POST /account/avatar` — multipart upload of a single image.
 *
 * The backend decides what is acceptable by decoding the bytes, not by reading
 * the declared type, so a rejection here carries a real explanation (too large,
 * not a decodable image, wrong format). Show its message rather than a generic
 * failure.
 */
function uploadAvatar(runtime: ApiRuntime, 
  file: File,
  signal?: AbortSignal,
): Promise<AvatarUploadResponse> {
  const body = new FormData();
  body.append("file", file);

  return authorizedRequest<AvatarUploadResponse>(runtime, "/account/avatar", {
    method: "POST",
    body,
    signal,
  });
}

/**
 * `DELETE /account/avatar` — removes the stored file and clears the field.
 *
 * Returns the updated profile rather than an acknowledgement, and deleting when
 * there is no avatar is not an error.
 */
function deleteAvatar(runtime: ApiRuntime, signal?: AbortSignal): Promise<Account> {
  return authorizedRequest<Account>(runtime, "/account/avatar", {
    method: "DELETE",
    signal,
  });
}

/** A platform-configured instance of the complete Loom API surface. */
export type ApiClient = ReturnType<typeof createApiClient>;

/**
 * Constructs an isolated API client from platform adapters.
 *
 * No browser globals are read here. Web, desktop, and mobile decide how tokens
 * are persisted and how the backend base URL is resolved.
 */
export function createApiClient(options: {
  baseUrlProvider: BaseUrlProvider;
  tokenStorage: TokenStorageAdapter;
}) {
  const runtime: ApiRuntime = {
    baseUrlProvider: options.baseUrlProvider,
    tokenStore: new TokenStore(options.tokenStorage),
    baseUrl: null,
    initialization: null,
    inFlightRefresh: null,
  };

  return {
    tokenStore: runtime.tokenStore,
    initialize: () => initializeRuntime(runtime),
    refreshSession: () => refreshSession(runtime),
    getHealth: (signal?: AbortSignal) => getHealth(runtime, signal),
    getSetupStatus: (signal?: AbortSignal) => getSetupStatus(runtime, signal),
    completeSetup: (data: SetupRequest, signal?: AbortSignal) =>
      completeSetup(runtime, data, signal),
    login: (username: string, password: string, signal?: AbortSignal) =>
      login(runtime, username, password, signal),
    refreshTokens: (refreshToken: string, signal?: AbortSignal) =>
      refreshTokens(runtime, refreshToken, signal),
    logout: (refreshToken: string, signal?: AbortSignal) =>
      logout(runtime, refreshToken, signal),
    getSession: (signal?: AbortSignal) => getSession(runtime, signal),
    getConnectors: (signal?: AbortSignal) => getConnectors(runtime, signal),
    executeAction: (
      connectorId: string,
      actionId: string,
      params?: unknown,
      signal?: AbortSignal,
    ) => executeAction(runtime, connectorId, actionId, params, signal),
    getUsers: (signal?: AbortSignal) => getUsers(runtime, signal),
    createUser: (data: CreateUserRequest, signal?: AbortSignal) =>
      createUser(runtime, data, signal),
    updateUser: (id: string, data: UpdateUserRequest, signal?: AbortSignal) =>
      updateUser(runtime, id, data, signal),
    deleteUser: (id: string, signal?: AbortSignal) => deleteUser(runtime, id, signal),
    getGroups: (signal?: AbortSignal) => getGroups(runtime, signal),
    createGroup: (data: CreateGroupRequest, signal?: AbortSignal) =>
      createGroup(runtime, data, signal),
    updateGroup: (id: string, data: UpdateGroupRequest, signal?: AbortSignal) =>
      updateGroup(runtime, id, data, signal),
    deleteGroup: (id: string, signal?: AbortSignal) => deleteGroup(runtime, id, signal),
    getPermissions: (signal?: AbortSignal) => getPermissions(runtime, signal),
    avatarSrc: (avatarUrl: string) => avatarSrc(runtime, avatarUrl),
    getAccount: (signal?: AbortSignal) => getAccount(runtime, signal),
    updateAccount: (data: UpdateAccountRequest, signal?: AbortSignal) =>
      updateAccount(runtime, data, signal),
    changePassword: (
      currentPassword: string,
      newPassword: string,
      signal?: AbortSignal,
    ) => changePassword(runtime, currentPassword, newPassword, signal),
    uploadAvatar: (file: File, signal?: AbortSignal) => uploadAvatar(runtime, file, signal),
    deleteAvatar: (signal?: AbortSignal) => deleteAvatar(runtime, signal),
  };
}
