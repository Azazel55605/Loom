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
 * Note that the auth and connector endpoints exist **only** in a backend built
 * with the non-default `dev-stub-auth` feature. Against a default build every
 * one of them answers 404. See `docs/BUILD.md` for how to run the pair locally.
 */

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

/** `POST /auth/login` response. */
export type LoginResponse = {
  token: string;
  /** RFC 3339, UTC, one hour ahead in the current stub. */
  expiresAt: string;
};

/** `GET /auth/session` response for an accepted token. */
export type SessionResponse = {
  authenticated: boolean;
  user: string;
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

  constructor(status: number, message: string, connectorError?: ConnectorError) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.connectorError = connectorError;
  }

  /** True when the backend rejected our token and the session is over. */
  get isUnauthorized(): boolean {
    return this.status === 401;
  }
}

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

  return (await response.json()) as T;
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
      return new ApiError(
        response.status,
        parsed.error ?? fallback,
        parsed.connectorError,
      );
    } catch {
      // A plain-text rejection from the extractor layer.
      return new ApiError(response.status, text.trim() || fallback);
    }
  } catch {
    return new ApiError(response.status, fallback);
  }
}

/* -------------------------------------------------------------------------- */
/* Endpoints                                                                   */
/* -------------------------------------------------------------------------- */

/** `GET /health` — the only route present in a default backend build. */
export function getHealth(signal?: AbortSignal): Promise<Health> {
  return request<Health>("/health", { signal });
}

/**
 * `POST /auth/login`.
 *
 * The current stub accepts any credentials; the call is shaped for real auth
 * regardless, so only the backend changes when real auth lands.
 */
export function login(
  username: string,
  password: string,
  signal?: AbortSignal,
): Promise<LoginResponse> {
  return request<LoginResponse>("/auth/login", {
    method: "POST",
    body: { username, password },
    signal,
  });
}

/** `GET /auth/session` — validates a stored token. Throws a 401 `ApiError`
 *  when the token is not accepted. */
export function getSession(
  token: string,
  signal?: AbortSignal,
): Promise<SessionResponse> {
  return request<SessionResponse>("/auth/session", { token, signal });
}

/** `GET /connectors` — every registered connector with its current status. */
export function getConnectors(
  token: string,
  signal?: AbortSignal,
): Promise<ConnectorSummary[]> {
  return request<ConnectorSummary[]>("/connectors", { token, signal });
}

/**
 * `POST /connectors/{id}/actions/{actionId}`.
 *
 * `params` is forwarded as the request body. Omitting it sends no body at all,
 * which the backend reads as JSON `null` — deliberately distinct from an empty
 * object, so "sent nothing" and "sent {}" stay distinguishable.
 */
export function executeAction(
  token: string,
  connectorId: string,
  actionId: string,
  params?: unknown,
  signal?: AbortSignal,
): Promise<ActionResult> {
  return request<ActionResult>(
    `/connectors/${encodeURIComponent(connectorId)}/actions/${encodeURIComponent(actionId)}`,
    { method: "POST", token, body: params, signal },
  );
}
