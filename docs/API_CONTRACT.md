# Loom API contract

This describes the API surface that exists **today**: a stub auth layer and a
single mock-backed connector, serving early frame and UI development. Nothing
here talks to a real service and nothing here authenticates anybody.

Endpoints and shapes will change as the real auth model
([ADR 0003](./adr/0003-auth-model-vpn-vs-external.md)) and real connectors
([ADR 0002](./adr/0002-connector-contract-tbd.md),
[ADR 0007](./adr/0007-in-process-connector-trait.md)) land. The *shape* — field
names, JSON types, nesting — is meant to survive that: the point of building the
stub against the eventual contract is that client code written now is not
thrown away. Treat a change to a field name or type here as a breaking change
to three clients, and make it deliberately.

## The auth and connector routes only exist in a dev build

The auth and connector routes are compiled in by the **non-default
`dev-stub-auth`** Cargo feature of `loom-web-backend`. In a default build those routes do not
exist: they are absent from the routing table and answer **404**, not 401. This
is asserted by tests rather than documented and hoped for — see the
`stub_absent` module in `crates/web-backend/src/main.rs`.

`GET /health` is the only route in a default build.

## Paths carry no `/api` prefix

Backend paths are written exactly as the backend serves them: `/health`,
`/auth/login`, `/connectors`. There is no `/api` prefix in the backend's own URL
space, and that is deliberate — the prefix belongs to whoever is *routing* to
the backend, not to the backend itself.

What each client does with that differs, and both are correct:

| Client | Requests | Reaches the backend as |
| --- | --- | --- |
| web-frontend | `/api/auth/login`, on its own origin | `/auth/login` |
| desktop, mobile | `<server-url>/auth/login`, directly | `/auth/login` |

The web frontend's server — nginx in production, the Vite dev server locally —
proxies `/api/*` to the backend and strips the prefix, which is what keeps the
browser same-origin and keeps any backend host out of the published bundle. See
[ADR 0006](./adr/0006-frontend-api-same-origin.md); the prefix-stripping rewrite
lives in `apps/web-frontend/nginx.conf` and `apps/web-frontend/vite.config.ts`.

The desktop and mobile clients talk to a user-supplied server URL directly, with
no proxy in the path, so they use the unprefixed paths as written here.

## Conventions

**All JSON field names are `camelCase`.** Every wire type carries
`#[serde(rename_all = "camelCase")]`, and `ConnectorError` additionally carries
`rename_all_fields = "camelCase"` so its variant tags *and* their fields are
camelCase. This is applied without exception across the connector types and the
stub's request and response bodies, and it is the assumption the TypeScript
clients are written against — a client should never need a per-field rename map.

The one place that is *not* camelCase is `/health`; see the note under that
endpoint.

Other conventions:

| Aspect | Rule |
| --- | --- |
| Content type | Requests and responses are `application/json`. Responses always set it; see per-endpoint notes for where the *request* content type is enforced. |
| Key order | Not significant. Serialization order follows the Rust struct, but clients must not depend on it. |
| Timestamps | RFC 3339, UTC, `Z`-suffixed (`"2026-08-19T12:00:00Z"`). |
| Absent values | An optional field is either serialized as `null` or omitted entirely. Which one it is is part of the contract and is stated per field below. |
| Path parameters | Connector and action ids are path segments: `/connectors/{id}/actions/{actionId}`. |

CORS is permissive by default and configurable via
`LOOM_CORS_ALLOWED_ORIGINS`; the reasoning and the conditions under which it
must be tightened are in [ADR 0005](./adr/0005-cors-policy.md) and are not
repeated here.

## Error body

Every error response produced by Loom's own handlers has the shape:

```json
{ "error": "no such connector: nope" }
```

When the failure came from a connector, the serialized `ConnectorError` is
attached as well, so a client can branch on the variant instead of parsing
prose:

```json
{
  "error": "unknown action id: self-destruct",
  "connectorError": { "invalidAction": { "actionId": "self-destruct" } }
}
```

`connectorError` is **omitted**, not null, when no connector was involved.

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `error` | string | Human-readable summary, safe to show a user. | Always present. |
| `connectorError` | object | The originating `ConnectorError`, externally tagged. | Omitted when the failure did not come from a connector. |

Two rejections happen in Axum's extractors, *before* any Loom handler runs, and
therefore do **not** use this shape — they answer `text/plain`. Both apply only
to `POST /auth/login`, the one endpoint that uses the `Json` extractor:

- **415 Unsupported Media Type** — request had no `Content-Type:
  application/json`.
- **422 Unprocessable Entity** — the body was valid JSON but not a
  `LoginRequest`.

Clients should not assume a JSON body on a 4xx from that endpoint.

## Endpoints

### `GET /health`

Present in every build, including default. Requires no feature and no auth.

**Request:** no body.

**Response 200:**

```json
{ "status": "ok", "core_version": "0.1.0" }
```

| Field | JSON type | Meaning |
| --- | --- | --- |
| `status` | string | Always `"ok"`. The route answering at all is the signal. |
| `core_version` | string | The version of the linked `loom-core`, proving the `core -> web-backend` wiring at runtime and not only at compile time. |

**Known wart:** `core_version` is **snake_case**. The `Health` struct in
`crates/web-backend/src/main.rs` has no `rename_all` attribute; it predates the
camelCase convention and is the single exception to it in the whole API. It is
documented as-is rather than quietly corrected, because a doc that lies about
one field is worse than a doc that admits an inconsistency. Renaming it to
`coreVersion` is a breaking change to whatever polls `/health`, so it wants a
deliberate change, not a drive-by fix.

Only 200 is returned; the handler is infallible.

### `POST /auth/login`

Requires `dev-stub-auth`. **Accepts any credentials.**

**Request:**

```json
{ "username": "anyone", "password": "anything" }
```

Both fields are required for the body to deserialize, and both are then
discarded. There is no user store to check them against.

**Response 200:**

```json
{
  "token": "dev-stub-token",
  "expiresAt": "2026-08-19T09:45:27.380696835Z"
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `token` | string | Always the literal `"dev-stub-token"`. | Always present. |
| `expiresAt` | string | RFC 3339 UTC, one hour after the request. Advisory only — nothing records issue times, so nothing enforces it. | Always present. |

| Status | Meaning |
| --- | --- |
| 200 | A token was issued. This is the only outcome for a well-formed request. |
| 415 | Missing `Content-Type: application/json`. Plain-text body. |
| 422 | Body was JSON but lacked `username` or `password`. Plain-text body. |
| 404 | The feature is not compiled in. |

### `GET /auth/session`

Requires `dev-stub-auth`. Reports whether the presented bearer token is the
stub token.

**Request:** no body. Header `Authorization: Bearer dev-stub-token`.

**Response 200:**

```json
{ "authenticated": true, "user": "dev-stub-user" }
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `authenticated` | boolean | Always `true` on a 200. An unauthenticated caller gets a 401, not `authenticated: false`. | Always present. |
| `user` | string | Always `"dev-stub-user"`. There is one identity and it is not tied to the submitted username. | Always present. |

**Response 401** — for a missing header, a non-`Bearer` scheme, or any other
token:

```json
{ "error": "missing or invalid bearer token" }
```

| Status | Meaning |
| --- | --- |
| 200 | The header carried the stub token. |
| 401 | No `Authorization` header, a non-`Bearer` scheme, or a token that is not the stub token. |
| 404 | The feature is not compiled in. |

### `GET /connectors`

Requires `dev-stub-auth`. **No authentication is checked** — see
[Known temporary behavior](#known-temporary-behavior).

**Request:** no body.

**Response 200** — a JSON **array**, one element per registered connector:

```json
[
  {
    "metadata": {
      "id": "mock",
      "name": "Mock Service",
      "icon": "beaker",
      "version": "0.1.0"
    },
    "status": {
      "health": "healthy",
      "details": {},
      "lastChecked": "2026-08-19T08:45:27.380462351Z"
    },
    "actions": [
      {
        "id": "restart",
        "label": "Restart",
        "description": "Pretends to restart the simulated service.",
        "paramsSchema": {}
      },
      {
        "id": "ping",
        "label": "Ping",
        "description": "Pretends to check that the simulated service answers.",
        "paramsSchema": {}
      }
    ]
  }
]
```

Today that array always has exactly one element, the `MockConnector`. It is an
array anyway, from day one, so that registering real connectors is an insertion
and not a reshape of the response — the client's list rendering, its TypeScript
types, and its loading states are all written once and stay correct. The
backend's registry is `Vec<Arc<dyn Connector>>` for the same reason.

`metadata`, `status`, and `actions` are nested rather than flattened into the
element, because all three are Core wire types the clients deserialize elsewhere
too. Nesting lets the TypeScript types compose instead of being re-declared per
response.

`actions` is included here rather than behind a separate request because the
dashboard needs it for every connector it draws: fetching it per connector would
be an N+1 round trip to build one screen. It is also what makes the contract
mean anything — a client that had to hardcode action ids would reduce
`Connector::actions` to decoration.

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `metadata` | object | `ConnectorMetadata`. Always available; it is synchronous and never fails. | Always present. |
| `status` | object | `ConnectorStatus` from a successful `status()` call. | **`null`** when the check itself failed. |
| `statusError` | object | The `ConnectorError` that made `status` null. | **Omitted** (not null) on the healthy path. |
| `actions` | array | `ConnectorAction[]` — what this connector can be asked to do right now. | Always present; **may be empty** for a read-only connector. |

**One failing connector does not fail the list.** A connector whose `status()`
returns `Err` contributes an element with `status: null` and a `statusError`,
and every other connector still reports normally:

```json
[
  {
    "metadata": {
      "id": "mock",
      "name": "Mock Service",
      "icon": "beaker",
      "version": "0.1.0"
    },
    "status": null,
    "statusError": { "unreachable": { "reason": "connection refused" } },
    "actions": []
  }
]
```

This is the whole reason the failure is per entry rather than at the response
level: in a homelab, *something* being down is the normal state, and a dashboard
that blanks out because one service is unreachable is useless exactly when it is
needed. Note also the distinction the shape preserves — a connector reporting
`health: "down"` is a **successful** status call, so it arrives in `status`, not
in `statusError`. `statusError` means Loom could not get a reading at all.

| Status | Meaning |
| --- | --- |
| 200 | The list was produced. This is the only outcome, including when every connector failed. |
| 404 | The feature is not compiled in. |

### `POST /connectors/{id}/actions/{actionId}`

Requires `dev-stub-auth`. **No authentication is checked.** Executes one action
on one connector.

**Request:** an **optional** JSON body, forwarded verbatim as the action's
`params`. The handler reads raw bytes rather than using the `Json` extractor, so
no `Content-Type` is required and an empty body is legal.

```json
{ "force": true }
```

**An absent or empty body becomes JSON `null`, not `{}`.** That is deliberate:
`null` is what `Connector::execute_action` already treats as "no parameters",
and keeping it distinct from `{}` keeps "the client sent nothing" separable from
"the client sent an empty object". A future action with an all-optional
parameter set can then tell a caller who opted out of parameters entirely from
one who explicitly submitted an empty form.

**Response 200** — an `ActionResult`:

```json
{
  "success": true,
  "message": "Simulated service restarted.",
  "payload": { "restarted": true, "params": { "force": true } }
}
```

A 200 with `success: false` is a normal answer, not an error; see
[`ActionResult`](#actionresult) for why that is different from a 5xx.

| Status | Meaning |
| --- | --- |
| 200 | The connector was reached and produced an `ActionResult` — successful or not. |
| 400 | The request body was present but not valid JSON. Loom's error shape. |
| 400 | `ConnectorError::InvalidParams` — the action exists, the parameters do not satisfy it. |
| 404 | No connector with that `id`. Loom's error shape, with **no** `connectorError` — no connector ran. |
| 404 | `ConnectorError::InvalidAction` — the connector exists, the action id does not. |
| 502 | `ConnectorError::Unreachable` or `ConnectorError::AuthFailed`. |
| 500 | `ConnectorError::Internal`. |
| 404 | The feature is not compiled in. |

## `ConnectorError` to HTTP status

The dividing line: `InvalidAction` and `InvalidParams` are the caller's mistake;
everything else is Loom failing, or being failed by the upstream service.

| Variant | Status | Reasoning |
| --- | --- | --- |
| `InvalidAction` | 404 Not Found | Consistent with an unknown connector id — the path `/connectors/{id}/actions/{actionId}` names something that is not there. |
| `InvalidParams` | 400 Bad Request | The request reached a real action and was malformed. |
| `AuthFailed` | **502 Bad Gateway** | Deliberately *not* 401. It means the *upstream service* rejected *Loom's* stored credentials. The caller is not the party that failed to authenticate and holds no credentials that would fix it; a 401 would tell a client to re-prompt its user, which cannot repair a bad token in Loom's connector configuration. It is a gateway failure, like `Unreachable`. |
| `Unreachable` | 502 Bad Gateway | Loom could not reach the upstream at all. |
| `Internal` | 500 Internal Server Error | The failure is inside Loom. |

Plus, outside the enum: an **unknown connector id is 404**, with `error` set and
`connectorError` omitted, since no connector was invoked.

## Core wire types

These live in `crates/core/src/connector/mod.rs` and are handed to the HTTP
layer unchanged. The examples below are the real serialized output — they can be
regenerated with `cargo test --package loom-core -- --nocapture
print_wire_shapes`.

### `ConnectorStatus`

```json
{
  "health": "degraded",
  "details": { "version": "1.2.3", "queueDepth": 12 },
  "lastChecked": "2026-08-19T12:00:00Z"
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `health` | string | One of `"healthy"`, `"degraded"`, `"down"`, `"unknown"`. | Always present. |
| `details` | any (usually object) | Connector-specific extras — version strings, queue depths, disk usage. Unstructured on purpose. | Always present; `{}` when there is nothing to report. |
| `lastChecked` | string | RFC 3339 UTC, `Z`-suffixed. When the reading was actually taken. | Always present. |

`health` is a closed set of four rather than a free-form string because clients
sort, colour, and alert on it. `"unknown"` is distinct from `"down"` so a
dashboard never reports an outage it has not observed — never-polled is not the
same as broken.

`details` is deliberately not typed. Forcing every service's telemetry into one
Rust struct would either bloat it or lose information, so a client that does not
recognise a particular connector simply ignores this field.

`lastChecked` is part of the **value**, not the response envelope. That is what
lets a polled or cached reading stay honest about its own age: if the backend
starts serving a status from a poll loop, the timestamp travelling with the
reading means the client can show "checked 4 minutes ago" instead of implying
the number is live.

### `ConnectorAction`

```json
{
  "id": "restart",
  "label": "Restart",
  "description": "Restarts the service.",
  "paramsSchema": {}
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `id` | string | Stable machine identifier, passed back in the action URL. | Always present. |
| `label` | string | Short human-facing name for a button or menu entry. | Always present. |
| `description` | string | Longer explanation for tooltips and confirmation prompts — the place to warn that an action is disruptive. | Serialized as **`null`** when absent, never omitted. |
| `paramsSchema` | object | JSON Schema for this action's parameters, driving client-side form generation and server-side validation. | Always present; `{}` for a parameterless action, never `null`. |

`id` is separate from `label` so renaming a button does not invalidate stored
automations or URLs. `paramsSchema` is `{}` rather than `null` for parameterless
actions so a consumer can always treat it as a schema and never has to
special-case the absence of one.

Delivered in the `actions` array of every `GET /connectors` element, so a
client never has to know an action id in advance. The list is not fixed for a
connector type: it may vary with the connector's configuration or the remote
service's state, so treat it as data to render, not as a schema to compile
against.

### `ActionResult`

```json
{
  "success": true,
  "message": "restart requested",
  "payload": { "jobId": "abc123" }
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `success` | boolean | Whether the service carried the action out. | Always present. |
| `message` | string | Human-readable summary, shown to the user verbatim. Clients are not expected to map it. | Always present. |
| `payload` | any | Structured result — a job id, the new state, a listing. | Serialized as **`null`** when absent, never omitted. |

**`Err` versus `success: false`.** These mean genuinely different things and a
client must handle them differently:

- An **HTTP error** (400/404/500/502) means Loom never got a verdict. The
  request may or may not have reached the service; nothing can be concluded
  about whether the action happened.
- **200 with `success: false`** means the service was reached, understood the
  request, and declined or failed it. There *is* a verdict, and it is no.

Collapsing the two would make it impossible for a UI to distinguish "your
server refused to restart" from "Loom is misconfigured and never asked it". The
first is the user's service to look at; the second is Loom's setup.

### `ConnectorMetadata`

```json
{
  "id": "mock",
  "name": "Mock Service",
  "icon": "beaker",
  "version": "1.0.0"
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `id` | string | Stable machine identifier: registry key and URL segment. Lowercase kebab-case by convention (`"mock"`, `"reverse-proxy"`). | Always present. |
| `name` | string | Display name shown in the UI. | Always present. |
| `icon` | string | Icon *identifier*, not image data — a name each client resolves against its own icon set. | Serialized as **`null`** when absent, meaning "use the generic fallback". |
| `version` | string | Version of the connector implementation, independent of the Loom release. | Always present. |

`icon` carries a name rather than a URL or bytes so that Core ships no assets
and assumes no renderer — the web, desktop, and mobile clients each map the name
onto their own icon set. `version` is the connector's own, so a connector can be
revised without a platform bump.

### `ConnectorError`

Externally tagged, so **each variant is a single-key object** whose key is the
camelCase variant name:

```json
{ "unreachable": { "reason": "connection refused" } }
{ "authFailed": { "reason": "token rejected" } }
{ "invalidAction": { "actionId": "nope" } }
{ "invalidParams": { "actionId": "restart", "reason": "missing `force`" } }
{ "internal": "unexpected response shape" }
```

| Variant key | Payload | Fields | Meaning |
| --- | --- | --- | --- |
| `unreachable` | object | `reason` (string) | The service could not be contacted: refused, timed out, DNS failure. The fix is at the infrastructure level, not in Loom. |
| `authFailed` | object | `reason` (string) | The service was reached but rejected Loom's credentials. The stored connector configuration needs attention. Separate from `unreachable` because the remedy is completely different. |
| `invalidAction` | object | `actionId` (string) | The requested action id is not one this connector exposes — usually a stale client or an automation naming a removed action. |
| `invalidParams` | object | `actionId` (string), `reason` (string) | The action exists but the parameters do not satisfy its schema. `reason` names the failed constraint so a client can point at the field. |
| `internal` | **string** | — | Anything else broke inside the connector: a bug, an unexpected response shape, a failed parse. |

**`internal` is the one asymmetry.** It is a newtype variant in Rust
(`Internal(String)`), so its value is a **bare string**, not an object with a
field. A client discriminating on the single key must handle that case
separately from the four struct variants. It is called out here because it is
exactly the kind of detail that produces a runtime type error in a client that
assumed uniformity.

This enum covers failures of the *interaction*, never of the managed service. A
service reporting its own bad state is a successful `ConnectorStatus` with
`health: "down"`; a service refusing an action is an `ActionResult` with
`success: false`. Keeping those out of this enum is what lets a client tell
"Loom is misconfigured" from "your server is unhappy".

### `config_schema()` — present on the trait, exposed by nothing

`Connector::config_schema()` returns the JSON Schema for the configuration a
connector needs. **No endpoint currently serves it.** It has two intended
consumers:

- **Manifest loading**, which validates a stored configuration before a
  connector is instantiated, so a bad config fails at load with a pointed
  message rather than at first use with an `Internal` error.
- **The clients**, which generate the setup form from it. This is the part that
  keeps "add a connector" from requiring a matching UI change in three
  applications — the form is derived, not written.

A connector needing no configuration returns an empty schema object rather than
`null`, matching the `paramsSchema` convention.

## Known temporary behavior

None of the following is a security measure. Do not read the stub as one.

- **Login validates nothing.** `POST /auth/login` accepts any username and
  any password, including empty ones, and returns a token.
- **The token is a fixed, hard-coded, publicly known string**
  (`"dev-stub-token"`). It is not a credential: no signature, no expiry
  enforcement, no revocation, no binding to a user. Every login yields the same
  one.
- **There is one identity**, `"dev-stub-user"`, unrelated to the submitted
  username.
- **`expiresAt` is advisory.** Nothing records when a token was issued, so
  nothing can enforce when it stops being valid.
- **The connector routes require no authentication at all.** Anyone who can
  reach the port can list connectors and execute actions. The `Authorization`
  header is read by `GET /auth/session` and by nothing else.
- **One connector is registered, and it is `MockConnector`.** It contacts
  nothing; `restart` and `ping` simulate their effects and echo their
  parameters. It is a permanent test fixture, not scaffolding — see the module
  docs in `crates/core/src/connector/mock.rs`.
- **There is no persistence of any kind.** No database exists yet: no users, no
  password hashes, no sessions, no revocation list, no connector configuration.
  This is deferred deliberately — designing persistence and secret generation
  against a stub would mean designing it twice. See
  [ADR 0004](./adr/0004-zero-config-startup.md).
- **The whole auth and connector surface is behind a non-default feature** and
  is absent from any normal build.

## The `dev-stub-auth` feature gate

Build with it explicitly:

```sh
cargo build --package loom-web-backend --features dev-stub-auth
cargo run   --package loom-web-backend --features dev-stub-auth
cargo test  --package loom-web-backend --features dev-stub-auth
```

The Cargo package is **`loom-web-backend`**, not `web-backend` — a crate named
`core` would collide with Rust's built-in `core`, so both crates carry the
`loom-` prefix. See [`BUILD.md`](./BUILD.md).

Every dependency used only by the stub (`chrono`, `serde_json`) is optional and
pulled in by the feature, so a default build does not even compile them.

A build with the feature on logs a `WARN` at startup, before it binds:

```text
WARN loom_web_backend: dev-stub-auth is COMPILED IN: /auth/login accepts
ANY username and password, and the connector routes require no authentication
at all. This build is for local development only and must not be exposed to any
network you do not fully control. See docs/API_CONTRACT.md.
```

**The rule: `dev-stub-auth` must never be enabled by default, never in any
Docker image, and never in any release CI workflow.** It exists only so the web,
desktop, and mobile clients can be built against the API's shape before the real
auth layer of [ADR 0003](./adr/0003-auth-model-vpn-vs-external.md) exists. See
[`AGENT_INSTRUCTIONS.md`](./AGENT_INSTRUCTIONS.md) for the project rule and
[`../crates/web-backend/Cargo.toml`](../crates/web-backend/Cargo.toml) for the
gate itself.
