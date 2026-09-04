# Loom API contract

This describes the API surface that exists **today**: real multi-user
authentication backed by SQLite, and a registry-driven connector system with
per-instance CRUD.

Connector shapes will still change as the connector contract settles
([ADR 0002](./adr/0002-connector-contract-tbd.md),
[ADR 0007](./adr/0007-in-process-connector-trait.md),
[ADR 0011](./adr/0011-connector-instance-registry.md)). The *shape* — field
names, JSON types, nesting — is meant to survive that. Treat a change to a field
name or type here as a breaking change to three clients, and make it
deliberately.

The auth design and its trade-offs are recorded in
[ADR 0008](./adr/0008-auth-model.md). Session visibility and login throttling
are recorded in [ADR 0030](./adr/0030-session-visibility-and-login-rate-limiting.md).

## Auth model

Two tokens, with different jobs:

| | Access token | Refresh token |
| --- | --- | --- |
| Form | HS256 JWT, claims readable by the client | Opaque 256-bit random value |
| Lifetime | **15 minutes** | **7 days** |
| Verified by | Signature only, no database read | Hash lookup in `refresh_tokens` |
| Revocable | **No** — valid until it expires | **Yes**, and rotated on every use |
| Sent as | `Authorization: Bearer <token>` | Request body, to `/auth/refresh` and `/auth/logout` |

The access token is what authenticates ordinary requests, and checking it costs
a signature verification rather than a query. It cannot be withdrawn, which is
why it is short.

The refresh token is what keeps a session alive across days. It is **rotated**:
every successful refresh issues a new one and revokes the one presented, so a
stolen refresh token is usable at most once, and its use is detectable — the
legitimate holder's next refresh fails against a token already spent.

**What a client must do:** store both tokens; send the access token on every
request; call `/auth/refresh` before `expiresAt` or after a 401; if the refresh
fails, discard both and return to login. A client that ignores refresh will
appear to sign the user out every fifteen minutes.

Permissions are granted to **groups** and reach a user through membership. Each
grant carries an optional resource scope, so "may control every connector" and
"may control only this one connector" are the same permission key with different
scope. See [Permission grants](#permission-grants).

## Paths carry no `/api` prefix

Backend paths are written exactly as the backend serves them: `/health`,
`/auth/login`, `/connector-instances`. There is no `/api` prefix in the backend's own URL
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
| Timestamps | RFC 3339, UTC. **Two spellings, both valid RFC 3339, and a client must parse either.** Values serialized by chrono's serde — notably `ConnectorStatus.lastChecked` — are `Z`-suffixed to whole seconds (`"2026-08-19T12:00:00Z"`). Values stored as text by a handler — every `createdAt`, and `expiresAt` — are written with `to_rfc3339()`, which emits a numeric offset and sub-second digits (`"2026-08-21T15:59:53.562608497+00:00"`). This was found by reading real responses, not assumed; unifying them would change the `createdAt` on users, groups, and connector instances at once, so it is recorded here rather than quietly changed. `new Date(...)` in JavaScript accepts both. |
| Absent values | An optional field is either serialized as `null` or omitted entirely. Which one it is is part of the contract and is stated per field below. |
| Path parameters | Instance and action ids are path segments: `/connector-instances/{id}/actions/{actionId}`. |

CORS permits local web development plus Tauri's known webview origins:
`tauri://localhost` for the custom protocol, `http://tauri.localhost` for
Android's default mapped origin (and the equivalent Windows default), and
`https://tauri.localhost` when the mapped HTTPS scheme is enabled. Additional
browser origins may be appended through `LOOM_CORS_ALLOWED_ORIGINS`. The
explicit list is sufficient because auth uses Bearer headers rather than
ambient cookies. See
[ADR 0010](./adr/0010-desktop-secure-storage-and-network-config.md).

## Error body

Every error response produced by Loom's own handlers has the shape:

```json
{ "error": "no such connector instance: nope" }
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

### `GET /setup/status`

**Unauthenticated by necessity**, not by oversight: a client must be able to ask
this before anyone can hold a credential.

**Request:** no body, no headers.

**Response 200:**

```json
{ "setupComplete": false }
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `setupComplete` | boolean | `false` when the instance still has no administrator. | Always present. |

Derived from whether any user row exists, not from a stored flag. A flag is a
second source of truth that can disagree with reality: if setup were interrupted
after writing the flag but before committing the user, a flag-based check would
report a configured instance nobody can log into. Counting users cannot drift,
because the thing it counts is the thing that makes setup meaningful.

One field on purpose. It answers to anyone who can reach the port, so it must
never grow a field describing a configured instance.

| Status | Meaning |
| --- | --- |
| 200 | The status was read. This is the only outcome. |

### `POST /setup`

Creates the first administrator and assigns them to the seeded **Administrators**
group. Unauthenticated for the same reason as the status route — there is nobody
to authenticate as until this succeeds — which is exactly why it must be
impossible to run twice.

**Request:**

```json
{
  "instanceName": "Example Homelab",
  "adminUsername": "admin",
  "adminPassword": "a-good-password"
}
```

| Field | JSON type | Rules |
| --- | --- | --- |
| `instanceName` | string | Non-empty after trimming. Stored in `server_config`. |
| `adminUsername` | string | Non-empty after trimming. Must be unique. |
| `adminPassword` | string | **At least 8 characters.** Hashed with argon2id; never stored or logged in the clear. |

The password rule is a length floor with no composition requirements: length is
what costs an attacker, while character-class rules mostly produce predictable
substitutions. It is a starting point, not a finished policy — see
[ADR 0008](./adr/0008-auth-model.md).

The new user is placed in the Administrators group, which holds one **global**
grant of every registered permission. So the first administrator's access token
carries all five permission keys with `resourceType` and `resourceId` both null.

**Response 200:**

```json
{ "setupComplete": true }
```

**Response 409** — already set up:

```json
{ "error": "setup has already been completed for this instance" }
```

The check and the writes run in **one transaction**, so two concurrent setup
requests cannot both create an administrator. That property is the point of this
endpoint: without it, a second caller could seize an instance that is already
configured. A client that gets a 409 should treat the instance as set up and
continue to login rather than showing an error — the end state is the one it
wanted. This happens legitimately when setup completes in another tab.

An interrupted or rejected setup leaves **no** user, so the wizard simply runs
again.

| Status | Meaning |
| --- | --- |
| 200 | Setup completed. The instance is now configured. |
| 400 | Empty `instanceName` or `adminUsername`, or a password under 8 characters. |
| 409 | Setup was already complete. Nothing changed. |
| 415 / 422 | Wrong content type, or a body that is not a setup request — rejected by the extractor before the handler runs, as plain text. See [Error body](#error-body). |
| 500 | The write failed. Nothing was committed. |

### `POST /auth/login`

**Request:**

```json
{ "username": "admin", "password": "a-good-password" }
```

**Response 200:**

```json
{
  "accessToken": "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...",
  "refreshToken": "3f1a...c9",
  "expiresAt": "2026-08-19T17:15:00Z"
}
```

| Field | JSON type | Meaning |
| --- | --- | --- |
| `accessToken` | string | HS256 JWT. Send as `Authorization: Bearer <accessToken>`. |
| `refreshToken` | string | Opaque hex string. Send in the body of `/auth/refresh` and `/auth/logout`. |
| `expiresAt` | string | When the **access** token expires — the value to schedule a refresh against. The refresh token's own expiry is not sent, because a client cannot act on it except by discovering its refresh failed. |

Each issued refresh-token session records the direct peer IP address and a
bounded copy of the request's `User-Agent`. The server never trusts forwarded
IP headers: until trusted-proxy configuration exists, a deployment behind a
reverse proxy will therefore report and rate-limit the proxy's address.

**Response 401:**

```json
{ "error": "invalid credentials" }
```

**One message for every failure.** A wrong password, an unknown username, and a
deactivated account are byte-for-byte identical responses. Distinguishing them
turns login into a username oracle — valuable to an attacker enumerating
accounts, useless to a legitimate user, whose next step is the same in all three
cases. The handler also hashes a dummy password when the username does not
exist, so the *timing* does not leak what the body refuses to.

Failed logins are limited per direct peer IP in a rolling 15-minute window.
The first 10 failures receive the same generic 401 above; subsequent attempts
receive 429 until the oldest failure leaves the window. A successful login
clears that peer's failure history. This state is deliberately in memory: it
needs no runtime configuration and resets when the backend restarts. The key is
an IP rather than a username so an anonymous caller cannot lock out a chosen
account by deliberately failing against it.

**Response 429:**

```json
{ "error": "too many login attempts; try again later" }
```

The response includes `Retry-After` as an integer number of seconds.

| Status | Meaning |
| --- | --- |
| 200 | Credentials accepted. A session now exists. |
| 401 | Credentials rejected, for any reason. |
| 429 | This direct peer has exceeded 10 failed attempts in the rolling 15-minute window. |
| 415 / 422 | Wrong content type or a body that is not a login request. |
| 500 | The lookup failed. |

### `POST /auth/refresh`

Exchanges a refresh token for a new pair, **rotating** it: the presented token is
revoked in the same operation.

**Request:**

```json
{ "refreshToken": "3f1a...c9" }
```

**Response 200:** identical in shape to login — a new `accessToken`, a **new**
`refreshToken`, and a new `expiresAt`. The old refresh token is dead from this
point; a client must store the new one.

Permissions are **recomputed from the database** on every refresh rather than
copied from the old token. This is the moment changed group membership takes
effect, and the moment a deactivated account stops working — the refresh is
rejected and the token burned.

**Response 401:**

```json
{ "error": "invalid or expired refresh token" }
```

Returned for a token that is unknown, revoked, expired, or belongs to an
inactive user. Collapsed into one message for the same reason as login: the
client's response to all four is to sign in again.

| Status | Meaning |
| --- | --- |
| 200 | A new token pair was issued. The old refresh token is now revoked. |
| 401 | The refresh token is not usable. Discard both tokens and return to login. |
| 500 | The rotation failed. |

### `POST /auth/logout`

**Request:**

```json
{ "refreshToken": "3f1a...c9" }
```

**Response 204**, with no body — **always**, whether or not the token was live.
Reporting "no such token" would let an unauthenticated caller test tokens, and
the caller's session is over either way.

Only the presented refresh token is revoked, so signing out on one device leaves
other sessions alone. Use `DELETE /users/{id}/sessions` for the explicit
sign-out-everywhere operation.

**An access token already issued stays valid until it expires.** Logout cannot
recall it. That is the cost of not checking the database on every request, and
the reason access tokens live only 15 minutes — a client should discard its
access token locally on logout rather than assume the server stopped honouring
it.

| Status | Meaning |
| --- | --- |
| 204 | Processed. No body. |
| 415 / 422 | Wrong content type or a body without `refreshToken`. |
| 500 | The revocation failed. |

### `GET /auth/session`

Who the bearer of an access token is, and what they were granted.

**Request:** no body. Header `Authorization: Bearer <accessToken>`.

**Response 200:**

```json
{
  "authenticated": true,
  "userId": "9d1f8c2e-4b7a-4c3d-9e21-0a5b6c7d8e9f",
  "username": "admin",
  "permissions": [
    { "key": "connectors.control", "resourceType": null, "resourceId": null },
    { "key": "connectors.view",    "resourceType": null, "resourceId": null },
    { "key": "groups.manage",      "resourceType": null, "resourceId": null },
    { "key": "system.settings",    "resourceType": null, "resourceId": null },
    { "key": "users.manage",       "resourceType": null, "resourceId": null }
  ]
}
```

Answered **entirely from the token's claims**, with no database read. That is
the point of a signed access token. The trade is staleness: a permission changed
a minute ago may not appear here until the token is refreshed, bounded by the
token's 15-minute life.

**Response 401** for a missing header, a non-`Bearer` scheme, a bad signature,
or an expired token:

```json
{ "error": "invalid or expired access token" }
```

| Status | Meaning |
| --- | --- |
| 200 | The token is valid. |
| 401 | No usable token. Refresh, then retry. |

### Permission grants

The `permissions` array appears in `/auth/session` and inside the access token's
claims. Each element:

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `key` | string | The permission, e.g. `connectors.control`. | Always present. |
| `resourceType` | string | The kind of resource this grant is limited to. | **`null`** for a grant covering every type. |
| `resourceId` | string | The single resource this grant is limited to. | **`null`** for a grant covering every resource of the type. |

Scope reads as:

| `resourceType` | `resourceId` | Meaning |
| --- | --- | --- |
| `null` | `null` | Global — every resource, every type |
| set | `null` | Every resource of that type |
| set | set | Exactly that one resource |

Registered keys today: `connectors.view`, `connectors.control`,
`connectors.manage`, `users.manage`,
`groups.manage`, `system.settings`. The set is defined by the `permissions`
table and extended only by migration.

**Clients may use this to hide controls the user cannot operate. That is a
convenience, never a control.** The server decides what is permitted; a client
that ignores this array learns nothing it could not have learned by trying. See
[Permission enforcement](#permission-enforcement) for what is actually checked,
and where.

### Permission enforcement

Every HTTP route except `/health`, `/setup/status`, `/setup`, `/auth/login`,
`/auth/refresh`, `/auth/logout`, and the `/avatars/*` static files requires a
valid access token.

Most also require a **grant**. The exceptions are `/auth/session` and the
[Account](#account) routes, which need a token and nothing more, because they
act only on the caller's own row and take no id to point elsewhere.

**401 and 403 mean different things and are not interchangeable.** 401 is "you
are not authenticated" — no token, a bad signature, an expired token — and a
client should refresh and retry. 403 is "you are authenticated and the answer is
still no"; refreshing will not change it. Returning 401 for a permission failure
makes a client retry the login it already completed.

The check reads the **claims in the access token**, not the database. That is
what keeps an authenticated request to one signature verification, and it means
a grant added or revoked right now reaches a user on their next refresh — at
most 15 minutes. See [ADR 0008](./adr/0008-auth-model.md).

#### Scope matching

A grant matches a check when the key matches and the grant's scope covers what
was asked for:

| Grant scope | Covers |
| --- | --- |
| `resourceType` null, `resourceId` null | every resource of every type, and any global check |
| `resourceType` set, `resourceId` null | every resource of that type |
| `resourceType` and `resourceId` set | exactly that one resource |

The asymmetry is deliberate and load-bearing:

- **A global grant satisfies a scoped check.** "May control every connector"
  plainly includes "may control this one".
- **A scoped grant does not satisfy a global check.** Holding
  `connectors.control` over one connector is not authority over connectors in
  general. Treating it as such would silently widen every narrow grant into a
  broad one, which would defeat the point of scoping.

#### What each route requires

| Route | Permission | Scope checked |
| --- | --- | --- |
| `GET /connector-types`, `POST /connector-types/{typeId}/discover`, `POST /connector-types/{typeId}/test-connection` | `connectors.manage` | global |
| `GET /connector-instances`, `GET /connector-instances/tags`, `GET /connector-instances/{id}` | `connectors.view` | global |
| `POST /connector-instances`, `PATCH /connector-instances/{id}`, `DELETE /connector-instances/{id}` | `connectors.manage` | global |
| `POST /connector-instances/{id}/actions/{actionId}` | `connectors.control` | `connector` / `{id}` |
| `GET /users`, `POST /users`, `PATCH /users/{id}`, `DELETE /users/{id}` | `users.manage` | global |
| `GET /groups`, `POST /groups`, `PATCH /groups/{id}`, `DELETE /groups/{id}` | `groups.manage` | global |
| `GET /permissions` | `groups.manage` | global |
| `GET /account`, `PATCH /account`, `POST /account/password`, `POST /account/avatar`, `DELETE /account/avatar` | none — token only | n/a, the subject is the token's `sub` |

**The three connector permissions answer three different questions**, and the
split is deliberate:

| Key | Question it answers |
| --- | --- |
| `connectors.view` | May you see this connector and its status? |
| `connectors.control` | May you press this connector's buttons? |
| `connectors.manage` | May you decide which connectors exist at all? |

`connectors.manage` is **not scopeable to a connector**, because the connector
is what is being created or destroyed; it is authority over the instance list.
Folding it into `connectors.control` would mean anyone allowed to restart one
service could also delete every connector on the instance, which is not what
granting a restart button is meant to say.

`GET /connector-instances` requires a **global** `connectors.view`, so a user
holding only an instance-scoped view grant is refused rather than shown a
filtered list. Filtering the response to what the caller may see would be
friendlier and is the natural next step — it is not built because nothing issues
scoped view grants yet, and a filter with no way to create the case it filters
cannot be tested against reality.

A 403 on the action route is returned **before** the instance id is looked up,
so an unauthorized caller gets the same response whether or not the id exists.
Otherwise the endpoint would report 404 for unknown ids and 403 for real ones,
which is a way to enumerate what is configured.

### Safeguards

Four rules protect an instance from irreversible administrative mistakes. They
return **409 Conflict** and change nothing.

**The last active administrator cannot be removed.** Deactivating, deleting, or
removing from the protected group the only remaining active member of it is
refused. Losing it means losing `users.manage` and `groups.manage` instance-wide
with no way back short of editing the database by hand. The check runs inside
the same transaction as the write it guards, against the state the commit *would*
produce, so every route to an empty administrator set is caught by one rule.

**Nobody may remove themselves.** Refused even when other administrators exist,
because an accidental self-deletion is unrecoverable by the person best placed
to notice it. Deleting someone else remains available for a genuine departure.

**The protected group cannot be deleted.** Checked on an `isProtected` column,
not on the group's name — a name is a label users change, and a check comparing
against the literal string `Administrators` would stop protecting the group the
moment someone renamed it. The flag guards deletion only: the group may still be
renamed and re-granted, which are legitimate administrative acts.

**A user who owns dashboards cannot be deleted.** Dashboard ownership is not
silently cascaded through account deletion. The owner must delete their
dashboards first, or the account can be deactivated while its content is
retained.

These safeguards are not substitutes for authorization. They guard against
irreversible mistakes after a caller has already passed the relevant permission
check.

### Access token claims

For clients that decode the JWT rather than calling `/auth/session`:

```json
{
  "sub": "9d1f8c2e-4b7a-4c3d-9e21-0a5b6c7d8e9f",
  "username": "admin",
  "permissions": [{ "key": "connectors.view", "resourceType": null, "resourceId": null }],
  "exp": 1787764500,
  "iat": 1787763600
}
```

`sub` is the user id; `exp` and `iat` are Unix timestamps in seconds. The
algorithm is HS256 and the verifier pins it — a token presenting any other
`alg`, including `none`, is rejected regardless of what its header claims.

Decoding a JWT client-side is reading an unverified assertion: a client has no
signing secret and cannot check the signature. Use it for display only.

## Connectors

Two route groups, and the split between them is the whole design:

- **`/connector-types`** is the catalog of what this *build* can create. It is
  code — each registration carries a factory function — so it is identical on
  every deployment of the same version and cannot be edited through the API.
- **`/connector-instances`** is what this *deployment* actually has. One row per
  connector a user added, stored as a type id plus an opaque JSON configuration,
  with a live connector object held in memory behind it.

Adding a connector *type* is a code change. Adding an *instance* of a registered
type is an ordinary `POST`, with the form generated from the type's published
`configSchema`. That is what keeps "add a connector" from requiring a matching
UI change in three clients. See
[ADR 0011](./adr/0011-connector-instance-registry.md).

**Status is polled and cached.** The backend performs an initial poll before it
accepts traffic, then currently polls every live connector every five seconds
(an implementation detail that may change). HTTP list and detail reads return
the latest completed snapshot immediately; they never wait for an upstream
service. A connector failure becomes that instance's `statusError` and does not
stop the poller or hide the other instances.

### `GET /ws` — connector status WebSocket

The backend path is `/ws`. In the browser's normal same-origin deployment the
proxy exposes it as `/api/ws`, matching the HTTP API prefix it strips before
forwarding.

Browsers cannot attach an `Authorization` header to a WebSocket handshake, so
the client supplies the **short-lived access token only** as a percent-encoded
query parameter:

```text
wss://loom.example.com/api/ws?token=<access-token>
```

Never send a refresh token. TLS (`wss`) protects the query in transit, but
operators should still avoid logging WebSocket query strings because request
targets can otherwise copy the access token into proxy logs. The trade-off is
recorded in [ADR 0012](./adr/0012-connector-status-push.md).

The token must be valid and carry a **global** `connectors.view` grant. A
missing, invalid, or expired token rejects the handshake with 401; a valid
caller without that grant receives 403.

After connecting, the client subscribes and unsubscribes by instance id. Both
operations are idempotent set operations:

```json
{ "type": "subscribe", "instanceIds": ["5aa2574d-9ba0-4af8-b7ae-74671fb48777"] }
```

```json
{ "type": "unsubscribe", "instanceIds": ["5aa2574d-9ba0-4af8-b7ae-74671fb48777"] }
```

When a subscribed instance's cached snapshot changes, the server pushes one of
these shapes:

```json
{
  "type": "status",
  "instanceId": "5aa2574d-9ba0-4af8-b7ae-74671fb48777",
  "status": {
    "health": "healthy",
    "details": { "load": 45.87, "enabled": true, "version": "1.2.3" },
    "lastChecked": "2026-08-21T12:00:00Z"
  }
}
```

```json
{
  "type": "status",
  "instanceId": "5aa2574d-9ba0-4af8-b7ae-74671fb48777",
  "status": null,
  "statusError": { "unreachable": { "reason": "connection timed out" } },
  "pendingOperation": null,
  "diagnosis": "Host `192.0.2.10` is unreachable on port `2375`. It may be offline, or a firewall is blocking the connection."
}
```

```json
{
  "type": "status",
  "instanceId": "5aa2574d-9ba0-4af8-b7ae-74671fb48777",
  "status": { "health": "down", "details": {}, "lastChecked": "2026-08-21T12:00:05Z" },
  "statusError": null,
  "pendingOperation": { "actionLabel": "Restart", "startedAt": "2026-08-21T12:00:03Z" },
  "diagnosis": null
}
```

`pendingOperation` and `diagnosis` are **always present**, as `null` when they
do not apply — see [Pending operations and diagnosis](#pending-operations-and-diagnosis)
for what they mean and why they are siblings of `status` rather than fields
inside it. They are pushed the moment they change, not only on the next poll:
an overlay a client learns about one poll late has been invisible for most of
the window it existed to cover.

`statusError` is omitted after a successful poll. Updates for ids not currently
subscribed on that connection are not sent. Disconnecting drops the connection's
subscription set; reconnecting clients must subscribe again. The shared client
does that automatically and reconnects with bounded exponential backoff,
including after an access-token refresh.

### `GET /connector-types`

Requires a global `connectors.manage` grant. This is the catalog behind the "add
a connector" form, and a caller who cannot add one has nothing to do with it —
the instances they may see are on `/connector-instances`, which asks only for
`connectors.view`.

**Request:** no body.

**Response 200** — a JSON **array**, one element per registered type, sorted by
`displayName` so a picker does not reshuffle between restarts:

```json
[
  {
    "typeId": "debug",
    "displayName": "Debug Connector",
    "icon": "lucide:bug",
    "configSchema": {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "title": "Debug connector configuration",
      "type": "object",
      "properties": {
        "simulatedLatencyMs": { "type": "integer", "minimum": 0, "default": 0 },
        "simulatedHealth": {
          "type": "string",
          "enum": ["healthy", "degraded", "down", "unknown"],
          "default": "healthy"
        },
        "failMode": {
          "type": "string",
          "enum": ["unreachable", "authFailed", "internal"]
        },
        "baseLoad": { "type": "number", "minimum": 0, "maximum": 100, "default": 42 },
        "label": { "type": "string", "minLength": 1, "default": "debug-fixture" },
        "enabled": { "type": "boolean", "default": true }
      },
      "additionalProperties": false
    },
    "setupGuide": {
      "variants": [
        {
          "id": "simple",
          "label": "Simple",
          "description": "Uses the live connection test for capability detail.",
          "template": "No setup needed — this is an internal test fixture.",
          "toggles": [],
          "capabilityRequirements": []
        },
        {
          "id": "configurable",
          "label": "Configurable",
          "description": "Exercises UI-only setup toggles and declarative capabilities.",
          "template": "Debug setup for {{label}}\nLOOM_DEBUG_WIDGETS={{LOOM_DEBUG_WIDGETS}}\nLOOM_DEBUG_ACTIONS={{LOOM_DEBUG_ACTIONS}}",
          "toggles": [
            {
              "key": "enableWidgets",
              "envVar": "LOOM_DEBUG_WIDGETS",
              "label": "Enable widgets",
              "description": "Includes read-only widget support in the example setup.",
              "default": true,
              "recommended": true
            },
            {
              "key": "enableActions",
              "envVar": "LOOM_DEBUG_ACTIONS",
              "label": "Enable actions",
              "description": "Includes mutating action support in the example setup.",
              "default": false,
              "recommended": false
            }
          ],
          "capabilityRequirements": [
            {
              "capabilityKey": "view-widgets",
              "label": "View widgets",
              "requiredToggleKeys": ["enableWidgets"]
            },
            {
              "capabilityKey": "perform-actions",
              "label": "Perform actions",
              "requiredToggleKeys": ["enableActions"]
            }
          ]
        }
      ]
    },
    "discoverableType": "debug",
    "discoveryTargetField": null
  },
  {
    "typeId": "docker",
    "displayName": "Docker",
    "icon": "brand:docker",
    "configSchema": {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "title": "Docker configuration",
      "type": "object",
      "properties": {
        "dockerHost": {
          "type": "string",
          "minLength": 1,
          "default": "unix:///var/run/docker.sock",
          "description": "Docker connection URI. …"
        }
      },
      "required": ["dockerHost"],
      "additionalProperties": false
    },
    "setupGuide": {
      "variants": [
        {
          "id": "socket",
          "label": "Direct socket",
          "description": "Same-host raw socket access; root-equivalent Docker authority.",
          "template": "services:\n  web-backend:\n    volumes:\n      - /var/run/docker.sock:/var/run/docker.sock\n\ndockerHost: {{dockerHost}}",
          "toggles": [],
          "capabilityRequirements": []
        },
        {
          "id": "proxy",
          "label": "Via socket proxy",
          "description": "Network-isolated LinuxServer socket-proxy setup with fine-grained read and lifecycle gates.",
          "template": "services:\n  socket-proxy:\n    image: lscr.io/linuxserver/socket-proxy:latest\n    environment:\n      PING: \"{{PING}}\"\n      VERSION: \"{{VERSION}}\"\n      CONTAINERS: \"{{CONTAINERS}}\"\n      ALLOW_LOGS: \"{{ALLOW_LOGS}}\"\n      ALLOW_START: \"{{ALLOW_START}}\"\n      ALLOW_STOP: \"{{ALLOW_STOP}}\"\n      ALLOW_RESTARTS: \"{{ALLOW_RESTARTS}}\"\n      ALLOW_PAUSE: \"{{ALLOW_PAUSE}}\"\n      ALLOW_UNPAUSE: \"{{ALLOW_UNPAUSE}}\"\n      INFO: \"{{INFO}}\"\n      SYSTEM: \"{{SYSTEM}}\"\n      IMAGES: \"{{IMAGES}}\"\n      VOLUMES: \"{{VOLUMES}}\"\n      NETWORKS: \"{{NETWORKS}}\"\n      POST: \"{{POST}}\"\n      ALLOW_ARCHIVE: \"0\"\n      ALLOW_CHANGES: \"0\"\n      ALLOW_EXPORT: \"0\"\n      ALLOW_TOP: \"0\"\n    networks: [loom-docker-api]\n\ndockerHost: tcp://socket-proxy:2375",
          "toggles": [
            { "key": "ping", "envVar": "PING", "label": "Allow ping", "description": "Reachability check.", "default": true, "recommended": true },
            { "key": "version", "envVar": "VERSION", "label": "Allow version", "description": "Version check and host-summary version.", "default": true, "recommended": true },
            { "key": "containers", "envVar": "CONTAINERS", "label": "Allow container access", "description": "Listing, inspect, and stats.", "default": true, "recommended": true },
            { "key": "allowLogs", "envVar": "ALLOW_LOGS", "label": "Allow logs", "description": "Container logs subpath only.", "default": true, "recommended": true },
            { "key": "allowStart", "envVar": "ALLOW_START", "label": "Allow start", "description": "Start action; works with POST disabled.", "default": true, "recommended": true },
            { "key": "allowStop", "envVar": "ALLOW_STOP", "label": "Allow stop", "description": "Stop action; works with POST disabled.", "default": true, "recommended": true },
            { "key": "allowRestarts", "envVar": "ALLOW_RESTARTS", "label": "Allow restarts", "description": "Restart/kill and stop actions; disruptive and opt-in.", "default": false, "recommended": false },
            { "key": "allowPause", "envVar": "ALLOW_PAUSE", "label": "Allow pause", "description": "Pause action; works with POST disabled.", "default": true, "recommended": true },
            { "key": "allowUnpause", "envVar": "ALLOW_UNPAUSE", "label": "Allow unpause", "description": "Resume action; works with POST disabled.", "default": true, "recommended": true },
            { "key": "info", "envVar": "INFO", "label": "Allow host information", "description": "Host container and image totals.", "default": false, "recommended": false },
            { "key": "system", "envVar": "SYSTEM", "label": "Allow disk-usage information", "description": "Docker disk usage from /system/df.", "default": false, "recommended": false },
            { "key": "images", "envVar": "IMAGES", "label": "Allow image access", "description": "Images table; with POST, pulling and deleting.", "default": false, "recommended": false },
            { "key": "networks", "envVar": "NETWORKS", "label": "Allow network access", "description": "Networks table; with POST, creating and deleting.", "default": false, "recommended": false },
            { "key": "volumes", "envVar": "VOLUMES", "label": "Allow volume access", "description": "Volumes table; with POST, creating and deleting.", "default": false, "recommended": false },
            { "key": "post", "envVar": "POST", "label": "Allow other write requests", "description": "Gates every non-GET method, DELETE included; lifecycle actions do not need it, image/volume/network writes do.", "default": false, "recommended": false }
          ],
          "capabilityRequirements": [
            { "capabilityKey": "list-containers", "label": "List containers", "requiredToggleKeys": ["containers"] },
            { "capabilityKey": "read-logs", "label": "Read container logs", "requiredToggleKeys": ["containers", "allowLogs"] },
            { "capabilityKey": "start-containers", "label": "Start containers", "requiredToggleKeys": ["containers", "allowStart"] },
            { "capabilityKey": "stop-containers", "label": "Stop containers", "requiredToggleKeys": ["containers", "allowStop"] },
            { "capabilityKey": "restart-containers", "label": "Restart containers", "requiredToggleKeys": ["containers", "allowRestarts"] },
            { "capabilityKey": "pause-containers", "label": "Pause containers", "requiredToggleKeys": ["containers", "allowPause"] },
            { "capabilityKey": "unpause-containers", "label": "Resume containers", "requiredToggleKeys": ["containers", "allowUnpause"] },
            { "capabilityKey": "host-summary", "label": "View host summary", "requiredToggleKeys": ["info", "system", "version"] },
            { "capabilityKey": "list-images", "label": "Browse images", "requiredToggleKeys": ["containers", "images"] },
            { "capabilityKey": "pull-image", "label": "Pull images", "requiredToggleKeys": ["containers", "images", "post"] },
            { "capabilityKey": "delete-image", "label": "Delete images", "requiredToggleKeys": ["containers", "images", "post"] },
            { "capabilityKey": "prune-images", "label": "Prune unused images", "requiredToggleKeys": ["containers", "images", "post"] },
            { "capabilityKey": "list-volumes", "label": "Browse volumes", "requiredToggleKeys": ["volumes"] },
            { "capabilityKey": "create-volume", "label": "Create volumes", "requiredToggleKeys": ["volumes", "post"] },
            { "capabilityKey": "delete-volume", "label": "Delete volumes", "requiredToggleKeys": ["volumes", "post"] },
            { "capabilityKey": "list-networks", "label": "Browse networks", "requiredToggleKeys": ["networks"] },
            { "capabilityKey": "create-network", "label": "Create networks", "requiredToggleKeys": ["networks", "post"] },
            { "capabilityKey": "delete-network", "label": "Delete networks", "requiredToggleKeys": ["networks", "post"] },
            { "capabilityKey": "list-updates", "label": "Check for container updates", "requiredToggleKeys": ["containers", "images"] },
            { "capabilityKey": "apply-update", "label": "Apply container updates", "requiredToggleKeys": ["containers", "images", "post"] }
          ]
        }
      ]
    },
    "discoverableType": null,
    "discoveryTargetField": null
  },
  {
    "typeId": "truenas",
    "displayName": "TrueNAS",
    "icon": "brand:truenas",
    "configSchema": {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "title": "TrueNAS connection",
      "type": "object",
      "properties": {
        "host": {
          "type": "string",
          "minLength": 1,
          "description": "TrueNAS hostname or IP address, without a scheme. Loom always connects with encrypted wss:// transport."
        },
        "username": {
          "type": "string",
          "minLength": 1,
          "description": "TrueNAS username that owns the API key. This is required by the current auth.login_ex API-key authentication flow."
        },
        "apiKey": {
          "type": "string",
          "minLength": 1,
          "x-loom-sensitive": true,
          "description": "API key generated from the TrueNAS top-right account/settings menu > My API Keys screen."
        },
        "allowInsecureCert": {
          "type": "boolean",
          "title": "Accept untrusted certificate",
          "default": false,
          "description": "Accept a self-signed or otherwise untrusted certificate. TLS encryption remains mandatory; this never enables an unencrypted connection."
        }
      },
      "required": ["host", "username", "apiKey"],
      "additionalProperties": false
    },
    "setupGuide": {
      "variants": [
        {
          "id": "api-key",
          "label": "Connect via API key",
          "description": "Open the top-right account/settings menu > My API Keys > Add API Key. The key inherits its associated user's RBAC privileges. TLS is mandatory; allowInsecureCert relaxes certificate validation only.",
          "template": "TrueNAS API-key checklist\n\nHost: {{host}}\nAPI-key owner: {{username}}\nTransport: encrypted WSS only\nBaseline role required for host verification: READONLY_ADMIN\n\nSelected feature roles (1 = include, 0 = omit):\nPOOL_READ — pools: {{POOL_READ}}\nDATASET_READ — datasets: {{DATASET_READ}}\nALERT_LIST_READ — alerts: {{ALERT_LIST_READ}}\nSNAPSHOT_WRITE + SNAPSHOT_DELETE — snapshot management: {{SNAPSHOT_MANAGE}}\nPOOL_WRITE — trigger pool scrubs: {{POOL_WRITE}}\nALERT_LIST_WRITE — dismiss alerts: {{ALERT_LIST_WRITE}}",
          "toggles": [
            { "key": "readPools", "envVar": "POOL_READ", "label": "Read pools", "description": "Grant POOL_READ so Loom can list pools and read capacity and health.", "default": true, "recommended": true },
            { "key": "readDatasets", "envVar": "DATASET_READ", "label": "Read datasets", "description": "Grant DATASET_READ so Loom can list datasets and their storage properties.", "default": true, "recommended": true },
            { "key": "readAlerts", "envVar": "ALERT_LIST_READ", "label": "Read alerts", "description": "Grant ALERT_LIST_READ so Loom can list active TrueNAS alerts.", "default": true, "recommended": true },
            { "key": "manageSnapshots", "envVar": "SNAPSHOT_MANAGE", "label": "Manage snapshots", "description": "Grant SNAPSHOT_WRITE and SNAPSHOT_DELETE so Loom can list, create, roll back, and delete snapshots.", "default": false, "recommended": false },
            { "key": "triggerScrub", "envVar": "POOL_WRITE", "label": "Trigger pool scrubs", "description": "Grant POOL_WRITE so Loom can start a scrub for a pool.", "default": false, "recommended": false },
            { "key": "dismissAlerts", "envVar": "ALERT_LIST_WRITE", "label": "Dismiss alerts", "description": "Grant ALERT_LIST_WRITE so Loom can dismiss an active alert.", "default": false, "recommended": false }
          ],
          "capabilityRequirements": [
            { "capabilityKey": "read-pools", "label": "Read pools", "requiredToggleKeys": ["readPools"] },
            { "capabilityKey": "read-datasets", "label": "Read datasets", "requiredToggleKeys": ["readDatasets"] },
            { "capabilityKey": "read-alerts", "label": "Read alerts", "requiredToggleKeys": ["readAlerts"] },
            { "capabilityKey": "manage-snapshot-actions", "label": "Manage snapshots", "requiredToggleKeys": ["manageSnapshots"] },
            { "capabilityKey": "trigger-scrub", "label": "Trigger pool scrubs", "requiredToggleKeys": ["triggerScrub"] },
            { "capabilityKey": "dismiss-alerts", "label": "Dismiss alerts", "requiredToggleKeys": ["dismissAlerts"] }
          ]
        }
      ]
    },
    "discoverableType": null,
    "discoveryTargetField": null
  },
  {
    "typeId": "pihole",
    "displayName": "Pi-hole",
    "icon": "brand:pihole",
    "configSchema": {
      "$schema": "https://json-schema.org/draft/2020-12/schema",
      "title": "Pi-hole connection",
      "type": "object",
      "properties": {
        "baseUrl": {
          "type": "string",
          "minLength": 1,
          "description": "Your Pi-hole's address; include the HTTP or HTTPS scheme."
        },
        "password": {
          "type": "string",
          "minLength": 1,
          "x-loom-sensitive": true,
          "description": "A Pi-hole password; an application password is recommended."
        },
        "allowInsecureCert": {
          "type": "boolean",
          "default": false,
          "description": "Accept an untrusted HTTPS certificate while retaining encrypted transport."
        }
      },
      "required": ["baseUrl", "password"],
      "additionalProperties": false
    },
    "setupGuide": {
      "variants": [
        {
          "id": "application-password",
          "label": "Connect via application password",
          "description": "Generate a dedicated application password under Settings > Web interface / API and enter it in Loom's Password field.",
          "template": "",
          "toggles": [],
          "capabilityRequirements": []
        }
      ]
    },
    "discoverableType": null,
    "discoveryTargetField": null
  }
]
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `typeId` | string | Stable machine identifier, sent back as `connectorType` when creating an instance. | Always present. |
| `displayName` | string | Human-facing name for the type picker. | Always present. |
| `icon` | string | The type's icon reference, in the same convention as [`ConnectorMetadata.icon`](#connectormetadata). Carried here so the type picker can draw an icon *before* any instance of the type exists. | **`null`** when the type declares none. |
| `configSchema` | object | JSON Schema for this type's configuration, published by the connector itself. | Always present; an object, never `null`. A type needing no configuration returns an empty schema object. |
| `setupGuide` | object | Descriptive client-rendered setup paths: `{ variants: SetupGuideVariant[] }`. See [Discovery & Setup Guides](#discovery--setup-guides). | **`null`** when the type publishes no guide. |
| `discoverableType` | string | Type id this connector can discover through a configured live instance. | **`null`** when discovery is unsupported. |
| `discoveryTargetField` | string | Candidate configuration field populated by a type-scoped discovery result's `targetFieldValue`. Clients use it to attach generic discovery assistance to the matching schema field. | **`null`** when discovery does not target one field. |

**The schema is advisory for the client and not the server's validator.** The
backend does not check a submitted configuration against it — it hands the value
to the connector's factory, which is the only thing that knows what the keys
mean. A configuration that satisfies the schema's shape can still be refused
(see [`POST /connector-instances`](#post-connector-instances)).

Today the array holds five: the debug fixture, the unified Docker connector,
the TrueNAS connector, the Pi-hole connector, and the UniFi Network connector. It was an array from day one so that registering a real connector
type is an insertion rather than a reshape of this response, which is exactly
what adding Docker turned out to be.

**This response is type-level and needs nothing running.** The Docker entry is
returned, with its full schema, on a host with no Docker daemon at all — so
someone can open the add-connector form and read what it needs before they have
a working endpoint. Whether the endpoint answers is decided when the *instance*
is created, not here.

### Connector types

| `typeId` | What one instance is | Configuration | Validated by |
| --- | --- | --- | --- |
| `debug` | A fixture that contacts nothing. Permanent — see `crates/core/src/connector/debug.rs`. | How it should pretend to behave. | Parsing alone; there is nothing to reach. |
| `docker` | One Docker daemon connection and its host-level aggregate view. Containers are addressable sub-targets of that instance. | Required `dockerHost` (`unix://` or `tcp://`) only. | A real daemon connection and ping. |
| `pihole` | One Pi-hole v6 instance with host-level statistics and DNS blocking control. | Required `baseUrl` including `http://` or `https://`, sensitive `password`, and optional `allowInsecureCert` (default `false`); an application password is recommended. | `POST /api/auth`, retaining `session.sid` for `X-FTL-SID` authentication. |
| `truenas` | One TrueNAS host with an aggregate host view plus addressable pool and dataset sub-targets. | Required bare `host`, required API-key owner `username`, sensitive `apiKey`, and optional `allowInsecureCert` (default `false`). | A mandatory-TLS WebSocket connection and `auth.login_ex` API-key authentication. |
| `unifi-network` | One local UniFi Network console site with host-level counts and addressable device sub-targets. | Required HTTPS `host` origin, sensitive `apiKey`, optional site UUID/internal reference/name (default `default`), and optional `allowInsecureCert` (default `false`). | `GET /proxy/network/integration/v1/sites` with `X-API-KEY`, followed by resolution of the configured site. |

The UniFi Network connector uses the official local Integration API. Its base is
`https://<console>/proxy/network/integration/v1`; API keys are generated in
**UniFi Network > Settings > Control Plane > Integrations**. Ubiquiti first
documented this API/key flow with Network 9.1.105, while the current published
OpenAPI is versioned with the installed Network application. A configured
`site` may be its UUID, legacy `internalReference`, or display name; Loom
resolves that to the UUID required by subsequent calls. All list calls use the
official offset envelope (`offset`, `limit`, `count`, `totalCount`, `data`) and
are consumed until `totalCount` is reached, advancing by the number of rows
actually returned so an inconsistent envelope `count` cannot skip a row. Sites,
devices, and clients default to 25 rows and allow 200; vouchers default to 100
and allow 1000. The host view
publishes `deviceCount`, `onlineDeviceCount`, and `clientCount` from the complete
device/client collections.

Each returned device is an addressable `device:{deviceId}` sub-target. A poll
also reads `/sites/{siteId}/devices/{deviceId}/statistics/latest` for every
known device, with at most ten Integration API calls in flight across the
connector. Device views publish the exact API `state`, `model`, a human-readable
`uptime` derived from `uptimeSec`, CPU/memory utilization, and uplink RX/TX
rates when those optional statistics are returned. Access points additionally
publish `connectedClientCount`, derived from the complete client collection's
`uplinkDeviceId`, and a radio summary built from the documented standard,
frequency, channel, and channel width fields. The current API publishes no
per-port throughput counters, so Loom does not invent a switch aggregate.
`frequencyGHz` accepts both the documented string form and the numeric form
returned by some real consoles. Radio summaries preserve one line per radio and
spell out frequency, channel, channel width, and the Wi-Fi generation/802.11
standard so compact tiles do not merge adjacent radios into an ambiguous value.
`ONLINE` is Healthy;
`PENDING_ADOPTION`, `UPDATING`, `GETTING_READY`, `ADOPTING`, and `DELETING` are
Degraded; `OFFLINE`, `CONNECTION_INTERRUPTED`, and `ISOLATED` are Down. Unknown
future state values remain Unknown rather than being guessed. Device type and
its generic icon come from the documented `features` values (`accessPoint` ->
`lucide:wifi`, `switching` -> `lucide:ethernet-port`, `gateway` ->
`lucide:router`); an absent/unrecognised feature set uses `lucide:network`
rather than guessing from a model name. When two devices would otherwise have
the same label, only those colliding labels receive the last two MAC octets as
a suffix.

Device targets expose a parameterless, disruptive `restart` action. It invokes
`POST /sites/{siteId}/devices/{deviceId}/actions` with
`{ "action": "RESTART" }` and warns that attached clients—and potentially the
network when restarting a gateway—will disconnect briefly.
There is no official site-summary or WAN-status endpoint in the current local
Network specification, so no WAN field is inferred from undocumented APIs.

### UniFi Network resource kinds

| Kind | Scope | Columns | Actions | Official source |
| --- | --- | --- | --- | --- |
| `ports` | Device target only | `port`, `poeEnabled`, `linkStatus` | Row: disruptive `cyclePoe` | The device detail's `interfaces.ports`; `POST /sites/{siteId}/devices/{deviceId}/interfaces/ports/{portIdx}/actions` with `POWER_CYCLE`. The 9.4.17 schema publishes neither port names nor PoE watt draw, so Loom does not fabricate either column. |
| `clients` | Host only | `name`, `mac`, `ipAddress`, `connectedTo`, `isGuest`, `authorized` | Row: `authorizeGuest` | The same paginated `/sites/{siteId}/clients` collection used by the host poll. Authorization posts `AUTHORIZE_GUEST_ACCESS`, with optional duration, data allowance, and RX/TX rate limits. VPN/Teleport rows legitimately have no MAC or uplink device. |
| `vouchers` | Host only | `code`, `expiresAt`, `usesRemaining`, `createdAt` | Row: `revokeVoucher`; kind: `createVoucher` | `GET`/`POST /sites/{siteId}/hotspot/vouchers` and `DELETE /sites/{siteId}/hotspot/vouchers/{voucherId}`. Remaining uses are the optional authorized-guest limit minus the authorized count; an unlimited voucher reports no numeric remainder. |

`createVoucher` creates one voucher and requires `name`, `timeLimitMinutes`,
and `authorizedGuestLimit`; optional parameters are `dataUsageLimitMBytes`,
`rxRateLimitKbps`, and `txRateLimitKbps`. `authorizeGuest` takes the same four
optional limit fields supported by the API. The official client action enum
contains guest authorize/unauthorize only—there is no client block/unblock
operation, so Loom exposes none.

The UniFi Network setup guide has one instruction-only **Connect via API key**
variant. It directs administrators running Network 9.1.105 or newer to
**Network > Settings > Control Plane > Integrations**, explains the explicit
`allowInsecureCert` opt-in for a locally self-signed certificate, and makes
clear that transport remains HTTPS even when certificate verification is
relaxed. Its template, toggles, and capability requirements are empty because
there is no command snippet and the official API-key model has no independent
per-capability scopes.

Test Connection first performs the same site-list request used by ordinary
construction. A transport or authentication failure returns `reachable:
false` with the connector's real error. Once authenticated, it genuinely lists
devices and clients and reports those read capabilities independently. Device
restart, PoE cycling, guest authorization, and voucher create/revoke are
reported available without being executed: the API key has no partial
permission model from which to infer a narrower result, and a connection test
must not disrupt a network or mutate its hotspot configuration.

Local consoles may use a locally issued or self-signed HTTPS certificate.
`allowInsecureCert` relaxes peer verification only for that connector instance,
remains off by default, and never permits plaintext transport.

The Pi-hole connector publishes `queriesToday`, `queriesBlockedToday`,
`blockPercentage`, `domainsOnBlocklist`, `uniqueClients`, `blockingEnabled`,
and `queriesHistory`. One poll fetches `/api/stats/summary`, `/api/history`, and
`/api/dns/blocking` concurrently; an expired SID is reauthenticated once and
the failed request retried. Its `setBlocking` action permanently enables or
disables blocking through `POST /api/dns/blocking` with `timer: null`. Pi-hole
`allowInsecureCert` relaxes certificate validation for self-signed or otherwise
untrusted homelab HTTPS endpoints while retaining encrypted transport. It is
off by default and applies only to this configured connector instance.

Its setup guide points to **Settings > Web interface / API > Configure app
password**. The application password is separately revocable, is shown only
once, and lets automation authenticate when 2FA would otherwise require a TOTP.
The live connection test authenticates first, then actually reads statistics,
the domain collection, and top clients. Those three results are reported
individually; once authentication succeeds, `setBlocking`, `addDomain`,
`removeDomain`, and `toggleDomainEnabled` are reported available because
Pi-hole application-password sessions have no per-operation permission model.

### Pi-hole resource kinds

Both kinds are host-only; Pi-hole has no Loom sub-targets.

| Kind | Columns | Actions | Source |
| --- | --- | --- | --- |
| `domains` | `domain` (text), `type` (`allow`/`deny` text), `comment` (text), `enabled` (bool) | Row: `toggleDomainEnabled`, `removeDomain`. Kind: `addDomain` with `domain`, `listType`, and optional `comment`. | `GET /api/domains`; exact entries only. Mutations use `/api/domains/{type}/exact[/{domain}]`. |
| `clients` | `client` (resolved hostname when present, otherwise IP), `queryCount` (number) | Read-only. | `GET /api/stats/top_clients`; each row maps `name`/`ip` and `count`. |

The Pi-hole API's combined domain response also contains `kind: "regex"`
entries. They are deliberately not flattened into this four-column exact-domain
kind: without exposing pattern kind, an exact and regex entry can look
identical, and an edit could silently change semantics. Regex-domain management
needs its own explicit UI contract if added later. `ResourceItem.id` is the
Pi-hole database id; row actions resolve that id against a fresh domain listing
before building the type/kind/domain item URL, so the opaque browser id never
needs to contain an unescaped domain. The verified upstream mapping and this
exact-only boundary are recorded in
[`adr/0033-pihole-v6-resource-api.md`](adr/0033-pihole-v6-resource-api.md).

The TrueNAS schema rejects schemes and paths in `host`; Loom owns the fixed
`wss://` scheme and `/api/current` JSON-RPC path. `apiKey` carries
`"x-loom-sensitive": true`, so it follows the encrypted-at-rest and
redact-on-read contract. `allowInsecureCert` opts out of certificate validation
for self-signed homelab deployments without permitting plaintext transport.
`username` identifies the account that owns the API key and is sent with it via
the current `auth.login_ex` `API_KEY_PLAIN` flow. Stored configurations created
before the username field was introduced retain the deprecated key-only fallback;
new configurations require the username.

Current TrueNAS 25.10 API keys are user-linked rather than independently
scoped: each key receives the effective RBAC roles of its associated user.
Create and assign limited access through the user's group privilege under
**Credentials > Groups > Privileges**; create the key itself through the
top-right account/settings menu at **My API Keys > Add API Key**, or through
**Credentials > Users > user > Add/View API Keys**. `auth.me` is a safe
read-only introspection call and returns the session's effective
`privilege.roles`, so Loom uses it to report write capability without
performing a scrub, snapshot mutation, or alert dismissal.

The setup connection test requires `core.ping` to return `"pong"` and
`system.info` to succeed; current TrueNAS requires `READONLY_ADMIN` for the
latter. It then genuinely probes `pool.query`, `pool.dataset.query`, and
`alert.list`. Write availability is derived from `auth.me`: snapshot
management requires both `SNAPSHOT_WRITE` and `SNAPSHOT_DELETE`, scrub
requires `POOL_WRITE`, and alert dismissal requires `ALERT_LIST_WRITE`.
`FULL_ADMIN` is recognized as granting all three. If role introspection is
missing or malformed, Loom conservatively reports writes unavailable rather
than attempting one.

TrueNAS documents API keys as password-equivalent and requires encrypted
transport; a key used for an insecure HTTP authentication attempt can be
automatically revoked. Loom therefore has no `ws://` mode.
`allowInsecureCert` only accepts a self-signed or otherwise untrusted
certificate while retaining encrypted `wss://` transport.

The host view publishes `poolCount`, `totalCapacityBytes`,
`usedCapacityBytes`, `freeCapacityBytes`, and `truenasVersion`. Each pool is an
addressable `pool:{name}` target publishing `status`, `usedBytes`, `freeBytes`,
and `capacityPercent`, with a disruptive `startScrub` action. A successful
action result means TrueNAS accepted and started its background scrub job; it
does not mean the scrub has completed. Each dataset is an addressable
`dataset:{path}` target publishing `usedBytes`, `availableBytes`,
`compressionRatio`, and `snapshotCount`. Dataset detail exposes snapshot
browsing plus create, rollback, and delete actions; the host resource browser
also lists pools, datasets, and active alerts, with alert dismissal where
permitted. The connector deliberately
does not mislabel `system.info.physmem` (installed RAM) or `loadavg` as memory
or CPU utilization.

**Creating a `docker` instance actually connects.** `POST
/connector-instances` opens and pings the endpoint before writing a row. An
unreachable daemon returns a 400 carrying an `unreachable` connector error
naming the configured endpoint.

The host view publishes
`totalContainers`, `runningContainers`, `stoppedContainers`, `totalImages`,
`diskUsageBytes`, `imageDiskUsageBytes`, and `dockerVersion`.
`imageDiskUsageBytes` is the image share of `diskUsageBytes` — reported
separately because it is the part a user can act on, since images are what the
Images table prunes and "102 GiB of Docker" does not say whether pruning would
help. Both come from one `/system/df` read, so they cannot drift apart, and the
image figure can never exceed the total it is part of. Each container returned by the live
sub-target endpoint has target-scoped `status`, `cpuPercent`, `cpuHistory`,
`memoryUsageBytes`, `memoryHistory`, `uptime`, and `logs`, plus start/stop/
restart/pause/unpause actions. Stored configuration containing a stale
`containerName` property remains loadable: the parser ignores that unused key,
but the schema no longer offers it for new instances.

#### Docker socket-proxy mapping and security status

The Docker setup guide is derived from LinuxServer's current
[`readme-vars.yml`](https://github.com/linuxserver/docker-socket-proxy/blob/main/readme-vars.yml),
generated [README](https://github.com/linuxserver/docker-socket-proxy/blob/main/README.md),
and [HAProxy template](https://github.com/linuxserver/docker-socket-proxy/blob/main/root/templates/haproxy.cfg).
The Docker-compatible API gates and their current defaults are:

- enabled (`1`): `EVENTS`, `PING`, `VERSION`;
- disabled (`0`): `ALLOW_ARCHIVE`, `ALLOW_CHANGES`, `ALLOW_EXPORT`,
  `ALLOW_LOGS`, `ALLOW_PAUSE`, `ALLOW_RESTARTS`, `ALLOW_START`, `ALLOW_STOP`,
  `ALLOW_TOP`, `ALLOW_UNPAUSE`, `AUTH`, `BUILD`, `COMMIT`, `CONFIGS`,
  `CONTAINERS`, `DISTRIBUTION`, `EXEC`, `IMAGES`, `INFO`, `NETWORKS`, `NODES`,
  `PLUGINS`, `POST`, `SECRETS`, `SERVICES`, `SESSION`, `SWARM`, `SYSTEM`,
  `TASKS`, and `VOLUMES`.

`ALLOW_START`, `ALLOW_STOP`, `ALLOW_RESTARTS`, `ALLOW_PAUSE`, and
`ALLOW_UNPAUSE` are checked before the broad `POST` rejection and therefore
work with `POST=0`. `ALLOW_RESTARTS` also admits stop and kill in the current
rules, which is why the guide labels it disruptive. Logs require both the base
`CONTAINERS` section and `ALLOW_LOGS`.

##### `POST` gates every method that is not `GET`

The HAProxy template contains exactly one method rule:

```
http-request deny unless METH_GET || { env(POST) -m bool }
```

It sits **after** the per-action container rules — which is why the `ALLOW_*`
lifecycle gates work with `POST=0`, since an earlier `http-request allow`
short-circuits — and **before** every category rule (`IMAGES`, `VOLUMES`,
`NETWORKS`, `CONTAINERS`, …). So `POST` is not a POST-verb toggle: it is an
**any-method-but-`GET`** master gate, and `DELETE` is covered by it even though
`DELETE` is never named. There is no per-category write toggle and no `DELETE`
toggle.

Verified empirically against `lscr.io/linuxserver/socket-proxy:latest`:

| Env | `GET /images/json` | `DELETE /images/…` | `POST /images/create` |
| --- | --- | --- | --- |
| `CONTAINERS=1 POST=1`, categories off | 403 | 403 | 403 |
| `IMAGES=VOLUMES=NETWORKS=1 POST=0` | 200 | 403 | 403 |
| `IMAGES=VOLUMES=NETWORKS=1 POST=1` | 200 | 404 (daemon) | 404 (daemon) |

Volumes and networks behave identically. Hence **every read capability needs
only its category toggle, and every write capability needs its category toggle
*and* `POST`.**

##### Toggles and the capabilities they unlock

| Toggle | Env var | Default | Unlocks |
| --- | --- | --- | --- |
| `ping` | `PING` | on | Reachability check |
| `version` | `VERSION` | on | `host-summary` (with `info`, `system`) |
| `containers` | `CONTAINERS` | on | `list-containers`, and a prerequisite of everything container-scoped |
| `allowLogs` | `ALLOW_LOGS` | on | `read-logs` |
| `allowStart` | `ALLOW_START` | on | `start-containers` |
| `allowStop` | `ALLOW_STOP` | on | `stop-containers` |
| `allowRestarts` | `ALLOW_RESTARTS` | off | `restart-containers` (also admits stop and kill) |
| `allowPause` | `ALLOW_PAUSE` | on | `pause-containers` |
| `allowUnpause` | `ALLOW_UNPAUSE` | on | `unpause-containers` |
| `info` | `INFO` | off | `host-summary` |
| `system` | `SYSTEM` | off | `host-summary` |
| `images` | `IMAGES` | off | `list-images`, `list-updates`; with `post`, `pull-image`, `delete-image`, `prune-images`, `apply-update` |
| `networks` | `NETWORKS` | off | `list-networks`; with `post`, `create-network`, `delete-network` |
| `volumes` | `VOLUMES` | off | `list-volumes`; with `post`, `create-volume`, `delete-volume` |
| `post` | `POST` | off | Every non-`GET` request outside the `ALLOW_*` lifecycle paths |

| Capability | Required toggles |
| --- | --- |
| `list-containers` | `containers` |
| `read-logs` | `containers`, `allowLogs` |
| `start-containers` | `containers`, `allowStart` |
| `stop-containers` | `containers`, `allowStop` |
| `restart-containers` | `containers`, `allowRestarts` |
| `pause-containers` | `containers`, `allowPause` |
| `unpause-containers` | `containers`, `allowUnpause` |
| `host-summary` | `info`, `system`, `version` |
| `list-images` | `containers`, `images` |
| `pull-image` | `containers`, `images`, `post` |
| `delete-image` | `containers`, `images`, `post` |
| `prune-images` | `containers`, `images`, `post` |
| `list-volumes` | `volumes` |
| `create-volume` | `volumes`, `post` |
| `delete-volume` | `volumes`, `post` |
| `list-networks` | `networks` |
| `create-network` | `networks`, `post` |
| `delete-network` | `networks`, `post` |
| `list-updates` | `containers`, `images` |
| `apply-update` | `containers`, `images`, `post` |

`list-images` requires `containers` as well because the Images table's
"Used by" column is a container listing — with `IMAGES` but not `CONTAINERS`
every image would claim nothing is using it. `list-updates` requires both
because an update check inspects the container and then its local image before
it asks any registry anything; `apply-update` adds `post` because it pulls
(`POST /images/create`), removes the old container (`DELETE`) and creates the
replacement (`POST`).

`GET /connector-types/{id}/connection-test` **live-probes** every read
capability — the container listing, the logs subpath, `/info`, `/system/df`,
and the image, volume and network listings — and reports each as available or,
on a `403`, unavailable with a note naming that capability's own toggles. Write
capabilities are never exercised: the only way to prove a delete is permitted is
to delete something, so they are reported unavailable with a note saying what to
confirm. A raw `unix://` socket reports every capability available without
probing, because it gates none of them.

As verified on 2026-08-28, [CVE-2026-78122](https://nvd.nist.gov/vuln/detail/CVE-2026-78122)
identifies Tecnativa `docker-socket-proxy` through 0.5.0, not LinuxServer's
image. LinuxServer's current configuration includes the granular archive,
change, export, logs, and top gates and its repository lists no published
security advisory specific to the image. That is not a reason to expose the
proxy publicly: it still fronts a root-equivalent API, so the generated Compose
keeps it on an internal network with no published host port and explicitly
leaves unused sensitive read paths disabled.

### `GET /connector-instances`

Requires a **global** `connectors.view` grant.

**Request:** no body.

**Response 200** — a JSON **array**, one element per stored instance, ordered by
`name`:

```json
[
  {
    "id": "3f1c1c5a-0f2e-4d1a-9c1e-2b6a8f0d4e77",
    "name": "Fixture",
    "connectorType": "debug",
    "createdAt": "2026-08-21T09:14:03.914238771+00:00",
    "tags": ["lab", "test"],
    "sensitiveFieldsSet": ["apiToken"],
    "metadata": {
      "id": "debug",
      "name": "Debug Connector",
      "icon": "lucide:bug",
      "version": "0.1.0",
      "minSize": [2, 2]
    },
    "iconOverride": "lucide:hard-drive",
    "status": {
      "health": "healthy",
      "details": {
        "": {
          "load": 45.87,
          "label": "debug-fixture",
          "enabled": true,
          "log": "2026-08-21T09:20:06.000Z INFO  simulated tick 41 — load 44.6%, enabled\n2026-08-21T09:20:11.000Z WARN  simulated tick 42 — load 45.9%, enabled",
          "loadHistory": [
            { "timestamp": "2026-08-21T09:20:01Z", "value": 42.1 },
            { "timestamp": "2026-08-21T09:20:06Z", "value": 44.6 },
            { "timestamp": "2026-08-21T09:20:11Z", "value": 45.87 }
          ]
        },
        "fixture-a": { "load": 45.87, "enabled": true }
      },
      "lastChecked": "2026-08-21T09:20:11Z"
    },
    "displayFields": [
      { "label": "Host", "value": "debug.invalid" },
      { "label": "Connector version", "value": "0.1.0" }
    ]
  }
]
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `id` | string | The instance's UUID. This is the id used in every other connector path and in resource-scoped grants. | Always present. |
| `name` | string | The user's name for this instance. | Always present. |
| `connectorType` | string | Which registered type it is. | Always present. |
| `createdAt` | string | RFC 3339 timestamp, in the stored spelling — numeric offset, sub-second digits. See [Conventions](#conventions). | Always present. |
| `tags` | array of strings | Free-form administrator labels assigned to this instance, sorted alphabetically. | Always present; may be empty. |
| `sensitiveFieldsSet` | array of strings | Schema-marked sensitive config keys that currently have a stored value. The values themselves are never returned. | Always present; may be empty. |
| `metadata` | object | [`ConnectorMetadata`](#connectormetadata) from the live connector. | Always present. |
| `iconOverride` | string | The user's icon for *this instance*, overriding `metadata.icon`. Same [reference convention](#icon-references). Set through `PATCH /connector-instances/{id}`. | **`null`** when no override is set — fall back to `metadata.icon`, then to the client's own default. |
| `status` | object | [`ConnectorStatus`](#connectorstatus) from the latest completed poll. | **`null`** before the first background poll or when the latest check itself failed. |
| `statusError` | object | The [`ConnectorError`](#connectorerror) that made `status` null. | **Omitted** (not null) before the first poll and on the healthy path. |
| `pendingOperation` | object | A disruptive action Loom is running against this instance right now: `{ actionLabel, startedAt }`. See [Pending operations and diagnosis](#pending-operations-and-diagnosis). | **`null`** when nothing is running. |
| `diagnosis` | string | Why this instance is Down, established by probing the network beneath it. | **`null`** unless it is Down *and* its connector names an endpoint worth probing. |
| `displayFields` | array | [`DisplayField`](#displayfield) values the connector agreed may be shown. | Always present; may be empty. |

#### Pending operations and diagnosis

Both fields sit **beside** `status`, never inside it. `status` is what the
connector reported; these two are what the *platform* knows about it, and
folding either into [`ConnectorStatus`](#connectorstatus) would change a Core
type that every connector and all three clients already depend on — to say
something no connector can know.

**`pendingOperation` exists because a restarting service is genuinely Down.**
An action whose `isDisruptive` is true takes the service away and brings it
back; a poll landing in that window reports an outage that is accurate and
unhelpful. While such an action runs, this field carries the action's own
label, and a client shows **"Performing: Restart"** in place of the health
badge. The raw health value is still there underneath for anything that wants
it.

- Set immediately before the action is dispatched, cleared when it returns —
  whichever way it returns, because a restart that failed is not still being
  performed.
- Cleared by a safety net after two minutes if the action never reports back at
  all, so one hung call cannot pin an instance to "Performing…" for the life of
  the process.
- Only actions the connector marks disruptive raise it. `stop` does not: the
  person who pressed Stop is not surprised that the service stopped.

**`diagnosis` answers "why".** When an instance goes Down and its connector
publishes a network target, the backend resolves the host and attempts a TCP
connection to the port, producing one of three sentences: DNS failed, the host
is unreachable on that port, or the host is reachable and the service is not.
The probe is debounced to at most once a minute per instance while it stays
Down, and the field is cleared as soon as it recovers.

Not every Down instance gets one, and that is deliberate rather than a gap. A
connector talking to a Unix socket, or an in-process fixture, has no host and
port whose reachability would tell anyone anything — those report `null` rather
than a reassuring sentence that means nothing.

**One failing connector does not fail the list.** A connector whose `status()`
returns an error contributes `"status": null` plus a `statusError`, and every
other instance still reports normally.

**A row with no live connector is still listed.** If an instance could not be
constructed at startup — its type is not registered in this build, or its stored
configuration is no longer valid — it appears with stand-in `metadata`, an empty
`displayFields`, and a `statusError` explaining that nothing was loaded. Hiding
it would leave a user with a connector they can neither see nor delete.

`GET /connector-instances` requires a **global** `connectors.view`, so a user
holding only an instance-scoped view grant is refused rather than shown a
filtered list. Filtering the response to what the caller may see would be
friendlier and is the natural next step — it is not built because nothing issues
scoped view grants yet, and a filter with no way to create the case it filters
cannot be tested against reality.

| Status | Meaning |
| --- | --- |
| 200 | The list was produced. This is the only success outcome, including when every connector failed. |
| 403 | The caller lacks a global `connectors.view` grant. |

### `GET /connector-instances/tags`

Requires a **global** `connectors.view` grant. Returns the distinct tags
currently assigned to at least one connector instance, sorted alphabetically.
There is no tags master resource: deleting or replacing the final assignment
removes that tag from this response automatically.

**Request:** no body.

**Response 200:**

```json
["lab", "production", "test"]
```

| Status | Meaning |
| --- | --- |
| 200 | The active tag vocabulary was produced; an empty array is valid. |
| 403 | The caller lacks a global `connectors.view` grant. |

### `GET /connector-instances/{id}`

Requires a **global** `connectors.view` grant. Everything the list entry
carries, plus what a dashboard placement UI needs.

**Response 200:**

```json
{
  "id": "3f1c1c5a-0f2e-4d1a-9c1e-2b6a8f0d4e77",
  "name": "Fixture",
  "connectorType": "debug",
  "createdAt": "2026-08-21T09:14:03.914238771+00:00",
  "tags": ["lab", "test"],
  "sensitiveFieldsSet": ["apiToken"],
  "metadata": { "id": "debug", "name": "Debug Connector", "icon": "lucide:bug", "version": "0.1.0", "minSize": [2, 2] },
  "iconOverride": null,
  "status": { "health": "healthy", "details": { "": {} }, "lastChecked": "2026-08-21T09:20:11Z" },
  "displayFields": [{ "label": "Host", "value": "debug.invalid" }],
  "config": { "baseLoad": 10 },
  "actions": [
    { "id": "ping", "targetId": null, "label": "Ping", "description": "…", "paramsSchema": {} }
  ],
  "dataPoints": [
    { "id": "load", "targetId": null, "label": "Load", "valueType": "number", "unit": "%" },
    { "id": "label", "label": "Label", "valueType": "string", "unit": null },
    { "id": "enabled", "label": "Enabled", "valueType": "bool", "unit": null },
    { "id": "loadHistory", "label": "Load history", "valueType": "timeSeries", "unit": "%" }
  ],
  "defaultLayout": {
    "bindings": [
      { "display": { "dataPointId": "load", "widgetType": "statTile", "config": {} } },
      { "display": { "dataPointId": "enabled", "widgetType": "statusDot", "config": {} } },
      { "display": { "dataPointId": "loadHistory", "widgetType": { "metricChart": { "chartType": "line" } }, "config": {} } },
      { "display": { "dataPointId": "load", "widgetType": "gauge", "config": { "min": 0, "max": 100 } } },
      { "display": { "dataPointId": "log", "widgetType": "logStream", "config": {} } },
      { "action": { "actionId": "set-enabled", "widgetType": "toggle", "config": {} } }
    ]
  },
  "discoverableType": "debug",
  "supportsSubTargets": true
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| *(list-entry fields)* | | Exactly as in `GET /connector-instances`. | |
| `config` | any | Stored non-sensitive configuration for pre-filling an edit form. Every property marked `x-loom-sensitive` in the type schema is omitted. | `null` for a row whose stored config is unreadable. |
| `actions` | array | [`ConnectorAction`](#connectoraction) — what this instance can be asked to do right now. | Always present; **may be empty** for a read-only or currently-broken connector. |
| `dataPoints` | array | [`DataPointDescriptor`](#datapointdescriptor) — what can be bound to a widget. | Always present; may be empty. |
| `defaultLayout` | object | [`WidgetLayout`](#widgetlayout) the connector ships with. | Always present; `bindings` may be empty. |
| `discoverableType` | string | Type id this live instance can discover. Clients use this to decide whether to offer discovery without guessing from `connectorType`. | **`null`** when unsupported or the stored instance is not loaded. |
| `supportsSubTargets` | boolean | Whether this live instance exposes addressable views through the sub-target endpoint. | Always present; `false` for unloaded and ordinary single-view connectors. |

Sensitive values are never returned, even to `connectors.manage`. An edit form
uses `sensitiveFieldsSet` to show that a value exists without receiving it.

| Status | Meaning |
| --- | --- |
| 200 | Found. |
| 403 | The caller lacks a global `connectors.view` grant. |
| 404 | No instance with that id. |

### `GET /connector-instances/{id}/sub-targets`

Requires a global `connectors.view` grant. Returns the live connector's cheap
addressable-view enumeration; it does not fetch status, stats, or logs.

```json
[
  { "id": "web", "label": "web (example/image:latest)", "kind": "container" },
  { "id": "database", "label": "database (example/database:latest)", "kind": "container" },
  { "id": "stack:shop", "label": "shop (stack)", "kind": "stack" }
]
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `id` | string | Stable id used by descriptors, placements, status details, and actions. | Always present. |
| `label` | string | Human-facing name shown when choosing a target. | Always present. |
| `kind` | string | What *sort* of thing this target is, in the connector's own vocabulary. | Always present; `"target"` when the connector does not distinguish. |

**`kind` is deliberately free-form, not an enum.** A closed set would have to
name every kind of thing every connector will ever address, and the first
connector wanting a "pool", a "share" or a "zone" would either wait for a Core
release or misuse the nearest existing word. It is the same choice already made
for connector type ids, action ids and data point ids: the vocabulary belongs to
the connector, and Loom carries it without interpreting it. Clients may group or
icon by it and **must** tolerate an unrecognised value by treating the target as
an ordinary one. Nothing in Loom branches on it — a connector that behaves
differently per kind does so from its own `target_id`, which is what it actually
receives.

Sub-targets are not discovery proposals and do not create connector instances.
They are views inside the already-configured connection. `discover()` retains
its separate purpose of suggesting whole new connector instances.

| Status | Meaning |
| --- | --- |
| 200 | Live sub-target list returned; it may be empty. |
| 400 | This connector does not support sub-targets, is unloaded, or enumeration failed. |
| 403 | The caller lacks a global `connectors.view` grant. |
| 404 | No instance with that id. |

## Update management

Loom can ask whether what a connector manages is out of date, and can apply the
answer. Both halves are generic: the *capability* is a connector trait method,
the *schedule* is a platform background task, and the *record* is the ordinary
[action log](#action-log). Docker is the first connector to implement it, not
the shape it was built around. See
[`adr/0023-docker-update-management.md`](adr/0023-docker-update-management.md).

### Update settings are a configuration convention

A connector that wants scheduled checking publishes these keys in **its own**
`configSchema`, with its own descriptions and defaults. The scheduler reads them
from the stored configuration by name; a connector that publishes none of them
is never checked.

| Key | Type | Default | Meaning |
| --- | --- | --- | --- |
| `checkForUpdates` | boolean | `false` | Check this instance at all. Off by default: checking contacts a third party. |
| `checkIntervalMinutes` | integer | `360` | Minutes between checks. |
| `autoApplyUpdates` | boolean | `false` | Apply a found update unattended. |
| `autoApplyAtTime` | string | *(empty)* | `HH:MM` **local** maintenance window. Empty means apply as soon as one is found. |
| `excludeFromAutoUpdate` | boolean | `false` | Keep checking and reporting, never apply. Overrides `autoApplyUpdates`. |

A key that is missing or of the wrong type falls back to its default rather than
failing: the scheduler runs across every instance, and one malformed
configuration must not stop the others being checked.

### The scheduler

A background task **separate from the status poller**, and deliberately so: the
poller asks a local daemon how something is doing every few seconds and backs
off when it fails; this asks a third-party registry what exists every few hours
and is rate-limited by somebody else. It wakes once a minute, checks whose
interval has elapsed, and for each due instance checks every target in turn with
a short pause between them — a host with thirty containers must not open thirty
registry connections in the same instant from one address.

An automatically applied update runs through the **same** action-execution path
as a manual one: same audit-log entry, same pre-action snapshot, same
pending-operation overlay, same immediate re-poll. Its log entry is attributed
to the system rather than to a person — `invokedBy.system` is `true` and
`invokedBy.id` is `null`.

### `updateStatus` on the instance detail

`GET /connector-instances/{id}` carries two new fields:

```json
{
  "supportsUpdateChecking": true,
  "updateStatus": {
    "web": {
      "available": true,
      "latestRef": "example/app@sha256:0123abcd",
      "lastChecked": "2026-08-28T03:00:12Z"
    }
  }
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `supportsUpdateChecking` | boolean | Whether this connector can answer the question at all. | Always present; `false` for an unloaded instance. |
| `updateStatus` | object | Readings keyed by target, `""` for the instance itself — the same convention `status.details` uses. | Always present; **empty until a check has run**. |
| `available` | boolean | Something newer exists. | Always present. |
| `latestRef` | string | What the newer thing is called, in the managed system's own terms — a digest, a tag, a version. Opaque to Loom. | `null` when nothing newer was found. |
| `lastChecked` | string | When this was established. | Always present. |

`updateStatus` sits **beside** `status`, not inside it. A registry reading is
hours old by design and a status reading is seconds old; one object carrying
both would invite a client to treat them as equally fresh.

### The `applyUpdate` action, and rollback

Docker's containers offer `applyUpdate`, taking `{ "targetImageRef": string }`.
It pulls that reference and recreates the container on it, preserving
environment, volumes, ports, restart policy, labels and networks. It is marked
`isDisruptive` and declares `snapshotDataPointIds: ["currentImageRef"]`, so the
action log records what the container was running immediately before.

**Rollback is that same action with the recorded reference.** There is no
`rollback` action and no stored "previous version" anywhere: the value a
rollback needs is on the log entry, put there by the generic snapshot mechanism.

The pull happens **before** anything is stopped, so a registry failure costs
nothing — the container is still running and the `ActionResult` says so. Each
later failure point reports which one it was and what state the host is in.

### Resource kinds

Two browsable tables, through the ordinary
[resource browser](#resource-browser):

| Kind | Provided by | Rows | Row action | Kind action |
| --- | --- | --- | --- | --- |
| `updates` | The connector | Every target with a waiting update, from its last check | `applyUpdate` with the row's `latestRef` | `updateAll` — applies each in turn, sequentially |
| `recentlyUpdated` | **The platform** | Successful `applyUpdate` entries from the action log | `applyUpdate` with the row's `previousRef` — this is the rollback | — |

Both are `applicableTarget: "hostOnly"`. "What on this host is behind?" and
"what did we update?" are questions about the host; a container's own view of
either would be the same table filtered to one row.

`recentlyUpdated` is the one kind Loom itself provides rather than the
connector, and it is offered for **any** instance whose connector reports
`supportsUpdateChecking`. Its rows are the action log's, and no connector can
see the action log — a connector reaching into Loom's database to fill a table
would invert the dependency the architecture rests on. Its columns are `target`,
`targetImageRef` (the previous reference, from the log entry's snapshot),
`newRef` (from its params), `appliedAt`, and `appliedBy` (a username, or
`Loom (scheduled)`).

**Two column-key conventions make a row self-describing**, and neither is
update-specific:

- **A column key that matches an action parameter name answers it.** Both tables
  name their reference column `targetImageRef` — `applyUpdate`'s own parameter —
  so a client fills that parameter from the row the button sits in. A row action
  whose every parameter is answered this way runs on click instead of opening a
  form.
- **A column keyed `targetId` names the sub-target the row's actions address.**
  A host-scoped table can list rows belonging to different sub-targets — one row
  per container — and the row's own `id` cannot stand in for that: in
  `recentlyUpdated` the id is a log entry. A client that has no browsing target
  reads `targetId` from the row.

### Private registries are not supported yet

Registry requests are made **anonymously**. A public repository on any registry
implementing the v2 API works — the `WWW-Authenticate` challenge is followed
wherever it points, so no registry is special-cased. A **private** repository
answers that challenge with a `401`, which surfaces as
`ConnectorError::AuthFailed` naming the repository and saying that Loom does not
yet support registry credentials.

That is a real limitation, stated plainly rather than presented as "no update
available". Fixing it is a decision about credential storage, not about HTTP.

## Docker host inventory

Three further browsable tables, all `applicableTarget: "hostOnly"` — images,
volumes and networks belong to the daemon, and "the images of one container" is
not a smaller version of that question but a different one with no answer.

| Kind | `groupByKey` | Columns | Row actions | Kind action |
| --- | --- | --- | --- | --- |
| `images` | `repository` | `repository`, `tag`, `imageId`, `size` (bytes), `created`, `usedBy`, `usage` (status) | `deleteImage`, `checkImageUpdate` | `pullImage` (`{ "imageRef": string }`), `pruneImages` |
| `volumes` | — | `name`, `driver`, `mountpoint`, `created`, `usedBy` | `deleteVolume` | `createVolume` (`{ "name": string, "driver"?: string }`, default `local`) |
| `networks` | — | `name`, `driver`, `scope`, `subnet`, `created`, `usedBy` | `deleteNetwork` | `createNetwork` (`{ "name": string, "driver"?: string }`, default `bridge`) |

Row ids are what the corresponding action is given as `resourceId`: an image's
`repository:tag` (or its content id, for an untagged image), a volume's name, a
network's **id** — Docker accepts either a network's name or its id, and only
the id is guaranteed unambiguous.

Image rows are one per **tag**, so an image carrying three tags is three rows —
a tag is what a person pulls, checks and deletes; the shared image behind them
is what the repeated `imageId` and `size` say. An image with no tag gets one row
keyed by its content id, and those sort **after** every named repository rather
than into their alphabetical place: on a real host they are the largest group
and the least interesting, and leading with three hundred `<none>` rows buries
every image somebody could name.

`usedBy` names the containers using that image, volume, or network, from one
container listing read per browse rather than one per row. Images are matched by
resolved image **id**, not by the reference a container was created from: a
container created from `app:latest` goes on naming `app:latest` after the tag
has moved, which would attribute it to whichever image holds that tag now rather
than to the one it is running.

The images table's `groupSummary` carries two group-level fields: `groupUsage`
(a status pill reading `In use`, `Some unused` or `Unused`) and `groupSize`.
Both are computed over the group's **distinct images**, not its rows.

**`groupSize` is labelled "Combined size" and not "total", deliberately.**
Docker's per-image `Size` counts every layer the image is built from, and images
share layers — so summing even distinct images counts shared layers once per
image. The result is an upper bound on disk, not disk: one real host reporting
102 GiB of Docker disk in total showed 297 GB across its untagged images alone.
The exact figure (`Size - SharedSize`) exists, but `SharedSize` is only computed
when a listing asks for it, and that computation is the same expensive layer
walk `/system/df` does — which this connector already treats as too costly to
call at poll cadence. So the cheap upper bound is reported under a name that
does not promise otherwise.

`pruneImages` removes every image no container is using — `dangling=false`, not
Docker's default `dangling=true`, which would keep every tagged image however
unused. That is exactly the set the table's own `Unused` pills mark: a
destructive button whose effect did not match the pills beside it would be one
you cannot predict by looking. Its `ActionResult` message names the space
reclaimed and its payload carries `{ "removed": number, "spaceReclaimedBytes":
number }` — the daemon's own figure, and the one number here that is real disk
rather than an upper bound.

`checkImageUpdate` reuses the same anonymous registry digest check the
[update management](#update-management) feature does, and returns its finding as
the action's `message` — "Update available", "Up to date", or the registry's
refusal. It downloads nothing.

**Volume size is deliberately not a column.** Docker only knows it from the
`/system/df` endpoint, which walks every volume's directory tree and can take
tens of seconds on a host with a large database volume — too expensive to pay on
every browse. If it turns out to matter, the honest way to add it is from the
connector's already-cached reading of that endpoint, with its age shown, rather
than by making this listing slow.

## Docker stacks

Containers sharing a `com.docker.compose.project` label are additionally
addressable as one **stack**. A stack is not something Loom maintains — it is
something Compose already recorded and this connector reads, so a project that
stops being deployed stops appearing, with no state to clean up.

- **Target id: `stack:{project}`.** A colon is the whole trick: Docker container
  names are restricted to `[a-zA-Z0-9][a-zA-Z0-9_.-]*`, so no container can ever
  be called `stack:anything` and **no existing target id changes meaning**. A
  saved placement pointing at `web` still points at the container `web`.
- **`kind: "stack"`**, beside the containers' unchanged `kind: "container"`.
  Stacks are *added* to the sub-target list, never substituted for their
  members: a stack is another way to look at the same containers, and someone
  who placed one container on a dashboard did not ask for that to become a
  stack tile.

### Stack data points

| Id | Type | Meaning |
| --- | --- | --- |
| `overallStatus` | string | `Running` (every member running), `Stopped` (none), `Partial` (some). |
| `memberCount` | number | Containers labelled into the project. |
| `runningCount` / `stoppedCount` | number | The split. |
| `cpuPercent` / `cpuHistory` | number / time series | Summed over members. |
| `memoryUsageBytes` / `memoryHistory` | number / bytes time series | Summed over members. |

CPU and memory reuse the **container ids on purpose**: they mean the same thing,
so a widget bound to `cpuPercent` draws a percentage whether the target is one
container or ten, and a `stackCpuPercent` would make every renderer learn a
second name for one reading.

**`overallStatus` is a data point, not health.** It is a plain string exactly
like a container's own `status`. `ConnectorStatus.health` says whether Loom can
reach the Docker daemon; a deliberately stopped stack is not a Docker host that
is down, and collapsing the two would make an ordinary maintenance window look
like an outage. See
[`adr/0027-docker-stacks.md`](adr/0027-docker-stacks.md).

**No extra Docker calls.** The aggregates are summed from readings the poll has
already taken for each container, and the members table lists those same
readings. Measured, not assumed: a poll of a host whose containers carry a
Compose label makes exactly the same number of daemon requests as a poll of the
same host without one.

### Stack actions and resource kind

`start`, `stop` (neither disruptive) and `restart` (disruptive) — the same ids a
container offers, with the same meanings, so a client needs no stack-specific
button and the action log reads the same for both. Each runs across the members
**sequentially**, for the reason `updateAll` does: starting six containers at
once on a home server is not what `docker compose up` would give you, and
stopping them at once takes down interdependent things simultaneously. A member
that fails does not stop the rest, and the result names **which** container
refused and what Docker said — "the stack action failed" is not actionable.

`pause`, `unpause` and `applyUpdate` are refused for a stack rather than applied
to an arbitrary member: they are per-container operations with no defensible
whole-stack meaning.

`stackMembers` (`applicableTarget: "targetOnly"`) is published **only** for a
stack target, and is the kind that motivated the `?targetId=` parameter above.
Columns: `targetId`, `status`, `cpuPercent` (number), `memoryUsageBytes`
(bytes). Browse-only — every member is also an ordinary sub-target with its own
detail view and controls, so a second set of buttons here would be a second
place to keep them working.

## Docker host log table

One more host-scoped kind, `logs` (`applicableTarget: "hostOnly"`, ungrouped),
answering a question no per-container view can: *what is everything on this host
saying right now?*

| Column | Type | Meaning |
| --- | --- | --- |
| `targetId` | text | Container name — the platform's convention for "which sub-target this row is about". |
| `status` | text | The daemon's own container state (`running`, `exited`, …). |
| `latestLogLine` | text | The most recent line, with Docker's timestamp prefix stripped. |
| `lastLogTimestamp` | timestamp | When that line was written — best effort, see below. |

**Browse-only: no row actions and no kind actions.** Opening one container's
full log is the per-container detail view's job, and a second route to it here
would be a second thing to keep working. The rows are read concurrently, at the
same bounded fan-out a status poll uses, and returned sorted by container name
so the table does not reshuffle between refreshes.

A container whose log cannot be read still gets a row, carrying the daemon's
explanation as its line — "this one's log driver does not support reading back"
is a useful thing for a log table to say, and dropping the row would make the
container look as though it did not exist.

**`lastLogTimestamp` is Docker's own timestamp when there is one, and the
connector's fetch time when there is not.** The request asks for
`timestamps=true`, so Docker prefixes each line with when the container emitted
it; that is used whenever it parses. When it does not — a driver that records no
times, a container that has said nothing, or a failed read whose explanation
stands in place of a line — the fallback is when the reading was taken, which is
at least true about something. The two are not distinguished in the column: a
`timestamp` cell has nowhere to put the distinction, a second "is this exact?"
column would cost more attention than it is worth, and the fallback is never
*older* than the real answer, so a stale-looking row is never a lie in the
direction that matters.

This kind needs no new capability. It is the container log endpoint read once
per container, so it is gated by `read-logs` (`containers` + `allowLogs`) like
the per-container `logs` data point.

**Delete is offered on every row, including ones Docker will refuse** — a volume
a container has mounted, and the built-in `bridge`, `host` and `none` networks.
Docker's refusal is passed through verbatim as the action's `message` with
`success: false`. This is a deliberate simplification: hiding the button would
mean re-implementing the daemon's removability rules here, from the outside,
where they would be wrong quietly and after a Docker upgrade, rather than wrong
loudly at the moment someone tried.

## Action log

Every invocation of `POST /connector-instances/{id}/actions/{actionId}` is
recorded, for every action on every connector, whatever the outcome. This is
not something a connector opts into or a client asks for: it happens in the
endpoint, so "who restarted the media server, and when?" is answerable without
anyone having planned to ask. See
[`adr/0022-action-log-and-update-checking.md`](adr/0022-action-log-and-update-checking.md).

A row is written **before** the action is dispatched, carrying the caller, the
parameters as submitted, and any pre-action snapshot. It is updated when the
action returns. Two consequences worth knowing:

- **An action that cannot be recorded is not performed.** A failure to write
  the row answers 500 and dispatches nothing. An audit trail whose gaps are
  exactly the interesting invocations is not one.
- **A row with `completedAt: null` is meaningful.** The action was authorized
  and dispatched and Loom never learned the outcome — which is what a restart
  that took the process down with it looks like from here.

### Pre-action snapshots

A [`ConnectorAction`](#connectoraction) may declare `snapshotDataPointIds`.
Before such an action runs, the backend reads each listed data point's current
value **from the poll cache** — no extra call to the service — scoped to the
action's own `targetId`, and stores the result on the log row as
`{ "<dataPointId>": <value> }`.

Ids the connector never reported are simply absent from the snapshot. A
snapshot is a record of what was known, not a guarantee that everything listed
was available: refusing to run an action because a reading was missing would be
the worse failure.

### `GET /connector-instances/{id}/action-log`

Requires a global `connectors.view` grant — reading the history is looking, not
doing, and the people most in need of "what happened to this service?" are
exactly the ones without authority to have done it.

| Query parameter | Meaning | Default |
| --- | --- | --- |
| `actionId` | Only invocations of this action. | All actions. |
| `targetId` | Only invocations against this sub-target. | All targets. |
| `success` | `true` for successful invocations or `false` for failures. Outstanding rows match neither value. | All outcomes. |
| `before` | Only rows invoked before this RFC 3339 timestamp (exclusive). Used as the newest-first “Load more” cursor. | No upper bound. |
| `after` | Only rows invoked after this RFC 3339 timestamp (exclusive). | No lower bound. |
| `limit` | Maximum rows. Clamped to 1–200 rather than refused, because an over-large value means "as much as there is". | 50 |

**Response 200** — newest first:

```json
[
  {
    "id": "0f2f1a5c-6b0e-4a1f-9a2e-2b6f4e1c33d0",
    "actionId": "recalibrate",
    "targetId": null,
    "params": {},
    "invokedBy": {
      "id": "48ae87dc-fc35-42db-a3de-467677ff8061",
      "username": "admin",
      "system": false
    },
    "invokedAt": "2026-08-28T18:00:00+00:00",
    "completedAt": "2026-08-28T18:00:02+00:00",
    "success": true,
    "resultMessage": "Simulated service recalibrated.",
    "snapshot": { "load": 41.7 }
  }
]
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `id` | string | This log entry. | Always present. |
| `actionId` | string | The action as invoked. | Always present. |
| `targetId` | string | The sub-target addressed. | `null` for an instance-level action. |
| `params` | any | The parameters as submitted, not normalized. | Always present; `null` when the caller sent no body. |
| `invokedBy` | object | `{ id, username, system }` — named, not merely identified, so the log reads without a second lookup. `system: true` marks an action Loom invoked itself, today meaning the [update scheduler](#the-scheduler). | `id` and `username` are `null` exactly when `system` is `true`; the database enforces that the two cannot both identify the actor. |
| `invokedAt` | string | When the action was dispatched. | Always present. |
| `completedAt` | string | When it returned. | **`null` while outstanding** — see above. |
| `success` | boolean | Whether the service carried the action out. A reached-and-declined action and a request that never arrived are both `false`, distinguished by `resultMessage`. | `null` while outstanding. |
| `resultMessage` | string | The `ActionResult`'s message, or the connector error's. | `null` while outstanding. |
| `snapshot` | object | Declared data points' values from just before the action ran. | `null` when the action declared none. |

| Status | Meaning |
| --- | --- |
| 200 | Entries returned; an empty array means nothing has been invoked here. |
| 400 | `before` or `after` is not an RFC 3339 timestamp. |
| 403 | The caller lacks a global `connectors.view` grant. |
| 404 | No instance with that id. |

### `GET /audit-log`

Requires a global `connectors.manage` grant. This is the cross-instance
administrative view of the same `connector_action_log` rows returned by the
per-instance endpoint; it joins connector identity and actor names rather than
maintaining a second audit store.

All supplied filters combine with AND. Results are newest first.

| Query parameter | Meaning | Default |
| --- | --- | --- |
| `instanceId` | Only invocations for this connector instance. | All instances. |
| `actionId` | Only invocations of this action. | All actions. |
| `userId` | Only invocations attributed to this user. System invocations have no user id and therefore do not match. | All actors. |
| `success` | `true` for successful invocations or `false` for failures. Outstanding rows match neither value. | All outcomes. |
| `before` | Only rows invoked before this RFC 3339 timestamp (exclusive). Used as the newest-first “Load more” cursor. | No upper bound. |
| `after` | Only rows invoked after this RFC 3339 timestamp (exclusive). | No lower bound. |
| `limit` | Maximum rows. Clamped to 1–200. | 50 |

**Response 200:** the per-instance [action-log response](#get-connector-instancesidaction-log),
with these fields added to every row:

```json
[
  {
    "instanceId": "f5318e63-4516-4302-81cb-23cb99820f52",
    "instanceName": "Workshop Docker",
    "connectorType": "docker",
    "id": "0f2f1a5c-6b0e-4a1f-9a2e-2b6f4e1c33d0",
    "actionId": "restart",
    "targetId": "container:example-app",
    "params": {},
    "invokedBy": {
      "id": "48ae87dc-fc35-42db-a3de-467677ff8061",
      "username": "admin",
      "system": false
    },
    "invokedAt": "2026-08-30T18:00:00+00:00",
    "completedAt": "2026-08-30T18:00:02+00:00",
    "success": true,
    "resultMessage": "Container restarted.",
    "snapshot": null
  }
]
```

| Status | Meaning |
| --- | --- |
| 200 | Entries returned; an empty array means no invocations match. |
| 400 | `before` or `after` is not an RFC 3339 timestamp. |
| 403 | The caller lacks a global `connectors.manage` grant. |

**Deleting a connector instance deletes its log** (`ON DELETE CASCADE`) — a
history of something that no longer exists is not evidence. **Deleting a *user*
named in the log is refused** with 409; deactivate the account instead.
Attribution a later account deletion can erase is not an audit trail.

## Resource browser

Some connectors manage *collections* — a Docker daemon's images, volumes, and
networks; a backup tool's snapshots. Those are not data points (a data point is
one reading that drives one widget) and not sub-targets (a sub-target is an
addressable view of the same service). They are tables: many rows, a few
columns, and some operations offered beside them.

A connector publishes zero or more **resource kinds**. Each kind names its
columns, its per-row actions, and its whole-kind actions. Nothing here is
Docker-specific: a client renders a table from the descriptors without knowing
what the connector is. See
[`adr/0021-connector-resource-browser.md`](adr/0021-connector-resource-browser.md)
and, for the two presentation hints added once a second connector needed them,
[`adr/0024-resource-kind-presentation-hints.md`](adr/0024-resource-kind-presentation-hints.md).

### Column value types

`valueType` tells a client how to *format* a cell. The value on the wire is
always the raw one; scaling and localizing are the client's business, so two
clients never disagree about what the number meant.

| `valueType` | Wire shape | Expected rendering |
| --- | --- | --- |
| `text` | string | As-is. |
| `number` | number | The client's ordinary numeric formatting. |
| `bool` | boolean | A yes/no affordance, not the literal `true`. |
| `timestamp` | string, ISO 8601 | Localized to the viewer's locale and timezone — a date, a time, or "3 days ago". |
| `bytes` | number, **raw byte count** | Human-readable size (`1.4 GB`), never the raw integer. |
| `status` | object `{ "label": string, "tone": "neutral" \| "positive" \| "caution" \| "negative" }` | A coloured pill. |

A `status` cell carries its **own tone** rather than the client inferring one
from the label. "Unused" is reclaimable disk for a Docker image and a failure
for a backup job; a table of known words in the frontend would be a connector's
vocabulary living in someone else's code. The tone names sentiment, not colour,
so a client is free to render it in a high-contrast or colour-blind palette.

These are deliberately not the [`DataPointValueType`](#datapointdescriptor)
values. A data point needs `timeSeries` and has no use for a byte count; a table
cell is the reverse.

### Invoking a resource action

Row and kind actions are ordinary [`ConnectorAction`](#connectoraction) values
and run through the ordinary
[`POST /connector-instances/{id}/actions/{actionId}`](#post-connector-instancesidactionsactionid)
endpoint, with the same `connectors.control` requirement, scoped to the same
`connector` / `{id}` resource. Browsing introduced **no new permission tier**: a
resource action is a connector action, not connector management.

Which row a row action acts on travels in `params` under the key
**`resourceId`**, carrying that row's `id`:

```json
{ "params": { "resourceId": "sha256:2f1c…" } }
```

A kind action addresses no row and sends no `resourceId`. A connector must
refuse a row action without one (`ConnectorError::InvalidParams` → 400) rather
than guessing, and should declare the field in the action's `paramsSchema` so a
client can see the requirement instead of reading it here.

`targetId` keeps its existing meaning — which *sub-target* is addressed — and is
orthogonal to `resourceId`.

### `GET /connector-instances/{id}/resource-kinds`

Requires a global `connectors.view` grant. Returns the live connector's
descriptors. Browsing what a service holds is looking at it, not administering
Loom, so this is the same read-only tier as sub-targets.

`?targetId=` optionally names which view is being looked at, and is passed
through to the connector unchanged; omitting it (or sending it empty, which is
what a blank form field does) means the instance as a whole. It exists so a kind
can be **absent** rather than merely empty: `applicableTarget` already says
*where* a kind belongs, and that is enough while every target of a connector is
the same sort of thing. It stops being enough when they are not — Docker's
stacks and its containers are both sub-targets, and "the containers in this
stack" is a table one of them has and the other does not, which `targetOnly`
cannot express because a container is a target too.

A listing (`/resources/{kind}`) is validated against **that target's** kinds for
the same reason: a kind only a stack publishes is as much "no such kind" for a
container as one nothing publishes at all.

```json
[
  {
    "kind": "widgets",
    "label": "Widgets",
    "columns": [
      { "key": "name", "label": "Name", "valueType": "text" },
      { "key": "size", "label": "Size", "valueType": "bytes" },
      { "key": "createdAt", "label": "Created", "valueType": "timestamp" }
    ],
    "rowActions": [
      {
        "id": "recycle",
        "targetId": null,
        "label": "Recycle",
        "description": "Pretends to recycle one simulated resource.",
        "paramsSchema": {
          "type": "object",
          "properties": { "resourceId": { "type": "string", "minLength": 1 } },
          "required": ["resourceId"],
          "additionalProperties": false
        },
        "isDisruptive": false
      }
    ],
    "kindActions": [
      {
        "id": "cleanupAll",
        "targetId": null,
        "label": "Clean up all",
        "description": "Pretends to clean up every simulated widget at once.",
        "paramsSchema": {},
        "isDisruptive": false
      }
    ],
    "groupByKey": null,
    "applicableTarget": "any",
    "groupSummary": []
  }
]
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `kind` | string | Stable machine id, unique within the connector, and the URL segment rows are fetched under. | Always present. |
| `label` | string | Human-facing table or tab name. | Always present. |
| `columns` | array | Column descriptors, in display order. | Always present; may be empty. |
| `rowActions` | array | `ConnectorAction`s taking a `resourceId`. | Always present; **may be empty**. |
| `kindActions` | array | `ConnectorAction`s addressing the kind as a whole. | Always present; **may be empty**. |
| `groupByKey` | string | A `columns[].key` whose value rows should be gathered under. | **`null`** for a flat table. |
| `applicableTarget` | string | Where this kind means anything: `hostOnly`, `targetOnly`, or `any`. | Always present; defaults to `any`. |
| `groupSummary` | array | `ColumnDescriptor`s describing each **group** as a whole, shown on the group heading and never as a row cell. | Always present; empty unless `groupByKey` is set. |

#### `groupSummary`

Each descriptor's `key` names a field that **every row of a group carries with
the same value**, so a client reads it off any row of the group. Deliberately
not client-side aggregation, which looks like the obvious generic answer and is
wrong for the first real case: Docker's image table lists one row per *tag*, so
three tags of one 2 GB image are three 2 GB rows, and a client summing them
would report 6 GB of disk that does not exist. The same applies to a verdict —
"some of these are unused" is not derivable from a column of per-row verdicts
without knowing which rows are the same underlying thing. Only the connector
knows that.

#### `groupByKey`

A **hint, not a contract**. The rows are the same rows either way, and a client
that ignores it renders a correct flat table. Rows arrive already ordered by the
grouping key, so a client builds contiguous sections without re-sorting; a row
with no value for the key belongs to a group of its own rather than being
dropped, and a key naming no column is ignored the way an unknown `fields` key
is.

#### `applicableTarget`

Whether the kind belongs in a view of the instance as a whole (`hostOnly`), of
one sub-target (`targetOnly`), or both (`any`, the default). Declared rather
than inferred: an empty listing cannot distinguish "this does not apply here"
from "there are none right now", which is the difference between a tab that will
fill tomorrow and one that never will. A client that does not recognise the
value should *show* the kind — a newer backend inventing a fourth case must not
make a table vanish from an older client with no explanation.

Both fields are additive. A descriptor written before they existed keeps exactly
the behaviour it had.

| Status | Meaning |
| --- | --- |
| 200 | Descriptors returned; an empty array means this connector browses nothing. |
| 400 | The instance is not loaded. |
| 403 | The caller lacks a global `connectors.view` grant. |
| 404 | No instance with that id. |

### `GET /connector-instances/{id}/resources/{kind}`

Requires a global `connectors.view` grant. Returns the current rows of one
kind. `?targetId=` optionally scopes the listing to a sub-target and is passed
through to the connector unchanged; omitting it means the instance as a whole.

```json
[
  {
    "id": "widget-1",
    "fields": {
      "name": "alpha-widget",
      "size": 1048576,
      "createdAt": "2026-01-04T09:15:00Z"
    }
  }
]
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `id` | string | Stable row id, passed back as `resourceId` when a row action is invoked. | Always present. |
| `fields` | object | Cell values keyed by `ColumnDescriptor.key`. | Always present; a missing key renders as an empty cell, and an unknown key is ignored. |

**An unknown `kind` is 400, not an empty list.** At the connector level the two
are the same answer, because a connector returns an empty list for a kind it
does not have. The backend validates `kind` against that instance's live
descriptors first, so a user staring at an empty table knows whether they are
looking at a service holding nothing or at a typo.

| Status | Meaning |
| --- | --- |
| 200 | Rows returned; an empty array means the kind exists and currently holds nothing. |
| 400 | This connector instance has no resource kind by that name, or is not loaded. |
| 400 | `ConnectorError::InvalidParams` / `InvalidConfig` from the listing. |
| 403 | The caller lacks a global `connectors.view` grant. |
| 404 | No instance with that id. |
| 502 | `ConnectorError::Unreachable` or `AuthFailed` — the listing could not be carried out. |
| 500 | `ConnectorError::Internal`. |

## Discovery & Setup Guides

Discovery has two complementary entry points. Instance-scoped discovery runs
through an already-configured live connector and is useful for bulk proposals.
Type-scoped discovery constructs and immediately discards a connector from a
candidate configuration, allowing setup forms to discover one field before an
instance exists. Setup guides and connection tests are also type-scoped:
guides describe independent setup paths, while a test constructs a candidate
connector and asks it for live reachability and capability detail. Neither
operation persists an instance.

### Setup guide variants, toggles, and template substitution

Each `setupGuide.variants` element is an independent way to prepare the service.
A variant contains an id, label, description, plain-text template, optional
UI-only toggles, and declarative capability requirements. Toggle values never
enter the submitted connector configuration and are never persisted. They only
affect live template rendering and declarative capability availability.

Each capability requirement lists `requiredToggleKeys`. The v1 rule is
deliberately **AND-only**: every listed toggle must be enabled for that
capability to be available. There is no OR expression model until a real
connector demonstrates the need for one.

`variant.template` may contain literal `{{fieldName}}` placeholders for the
exact camelCase property name from that type's `configSchema.properties`, or
`{{toggleKey}}` placeholders matching the selected variant's toggles. Clients
substitute current form and toggle values. Substitution is entirely client-side:
Core and the backend neither interpret templates nor persist rendered output.

For example, `{{label}}` refers to the debug schema's editable `label`
property. A client that does not recognise a placeholder must
leave it visibly unchanged rather than silently remove content.

### `POST /connector-instances/{id}/discover`

Requires a global `connectors.manage` grant, the same tier as creating an
instance. The request has no body.

**Response 200** — suggested resources, not created instances:

```json
{
  "discoveryTargetField": null,
  "resources": [
    {
      "suggestedName": "Discovered Debug Fixture 1",
      "targetConnectorType": "debug",
      "config": {
        "simulatedHealth": "healthy",
        "baseLoad": 24,
        "label": "discovered-alpha",
        "enabled": true
      },
      "targetFieldValue": null
    }
  ]
}
```

| Field | JSON type | Meaning |
| --- | --- | --- |
| `discoveryTargetField` | string or null | Candidate config field that `targetFieldValue` can fill directly. |
| `resources` | array | Discovery proposals. |
| `suggestedName` | string | Human-facing starting name for a future connector instance. |
| `targetConnectorType` | string | Type id whose normal factory validates and constructs `config`. |
| `config` | any | Suggested configuration in the target type's schema shape. |
| `targetFieldValue` | any or null | Value for `discoveryTargetField`, when discovery supports field assignment. |

Discovery does not persist anything. A client may present or edit suggestions,
then creates accepted resources through the ordinary
`POST /connector-instances` contract.

| Status | Meaning |
| --- | --- |
| 200 | Discovery completed; `resources` may be empty. |
| 400 | The instance does not support discovery or is not loaded. |
| 403 | The caller lacks a global `connectors.manage` grant. |
| 404 | No instance with that id. |

### `POST /connector-types/{typeId}/discover`

Requires a global `connectors.manage` grant. The JSON request body is a
candidate connector configuration. It only needs to satisfy the connector
factory.

The backend constructs the candidate, confirms that configuration supports
discovery, calls it once, and discards it without inserting a row or adding it
to the runtime map.

**Response 200:** the same `{ discoveryTargetField, resources }` envelope as
instance-scoped discovery. Docker does not support discovery: its containers
are sub-targets of one instance and are obtained from the read-only endpoint
above instead.

| Status | Meaning |
| --- | --- |
| 200 | Candidate discovery completed; `resources` may be empty. |
| 400 | Unknown type, factory validation failed (including an unreachable host), or discovery is unsupported for this candidate configuration. |
| 403 | The caller lacks a global `connectors.manage` grant. |

### `POST /connector-types/{typeId}/test-connection`

Requires a global `connectors.manage` grant. The JSON request body is a
candidate connector configuration, with the same shape accepted by that
connector type's factory.

The backend constructs a throwaway connector, calls `test_connection()` once,
returns its reachability and capability report, then discards it. It never
inserts a `connector_instances` row or adds the candidate to the runtime map.
The check is distinct from the recurring full `status()` poll and must not
probe destructive/write capabilities by performing them.

A type may publish a dedicated no-I/O constructor for this endpoint when its
ordinary instance factory validates a broader feature first. Docker uses that
path so a proxy that answers ping/version but denies `CONTAINERS`, `INFO`, or
`SYSTEM` still returns a useful 200 capability report instead of failing during
construction. Ordinary instance creation continues through the validating
factory unchanged.

**Request:**

```json
{
  "label": "candidate",
  "enabled": false
}
```

**Response 200:**

```json
{
  "reachable": true,
  "capabilities": [
    {
      "key": "read-status",
      "label": "Read status",
      "available": true,
      "note": null
    },
    {
      "key": "view-widgets",
      "label": "View widgets",
      "available": true,
      "note": null
    },
    {
      "key": "perform-actions",
      "label": "Perform actions",
      "available": false,
      "note": "Unavailable while the debug fixture's enabled flag is off."
    }
  ],
  "message": null
}
```

Connectors that do not override `test_connection()` receive the default check:
`status()` maps Healthy and Degraded to `reachable: true`, Down, Unknown, or an
error to `reachable: false`, and `capabilities` remains empty. A connector
override may publish finer-grained rows. Declarative toggle requirements and
live capability rows use the same stable capability keys but are distinct
mechanisms.

| Status | Meaning |
| --- | --- |
| 200 | Candidate connection test completed, whether reachable or not. |
| 400 | Unknown type or the connector factory rejected the candidate configuration. |
| 403 | The caller lacks a global `connectors.manage` grant. |

### `POST /connector-instances`

Requires a global **`connectors.manage`** grant — not `connectors.view`. Adding
and removing connectors is authority over the instance list itself, which is a
different capability from seeing or operating a connector that already exists.

**Request:**

```json
{
  "connectorType": "debug",
  "name": "Fixture",
  "config": { "baseLoad": 10 }
}
```

| Field | JSON type | Required | Meaning |
| --- | --- | --- | --- |
| `connectorType` | string | yes | A `typeId` from `GET /connector-types`. |
| `name` | string | yes | Trimmed; must not be empty. |
| `config` | any | no | Absent means "no configuration", which is what an unfilled form submits. |

**Validation is the connector's, not the schema's.** The backend builds a live
connector from `config` *before* writing anything. If the factory refuses, the
response is 400 carrying the connector's own `ConnectorError` — usually
[`invalidConfig`](#connectorerror) — and no row is created. This catches what a
shape check cannot: an unknown key, or a value that is the right type and still
out of range. A row the factory would refuse must never reach the database, or
it would be silently skipped at the next startup.

**Response 201** — the same body as `GET /connector-instances/{id}`. Building
the connector still validates and pings its configured endpoint before the row
is written, but the response does not wait for a full status inventory. It may
therefore contain `"status": null` with no `statusError`; the background poller
fills it shortly afterward.

| Status | Meaning |
| --- | --- |
| 201 | Created, persisted, and installed in the runtime. |
| 400 | `name` was empty, `connectorType` is not registered, or the connector refused `config`. |
| 403 | The caller lacks a global `connectors.manage` grant. |

An unregistered `connectorType` is **400, not 404**: the request as a whole is
malformed, and a 404 would suggest the *instance* was not found.

### `PATCH /connector-instances/{id}`

Requires a global `connectors.manage` grant.

**Request** — every field optional; an absent field is left alone:

```json
{
  "name": "Renamed",
  "config": { "label": "after-update" },
  "iconOverride": "lucide:hard-drive",
  "tags": ["lab", "test"]
}
```

| Field | JSON type | Meaning |
| --- | --- | --- |
| `name` | string | New display name. Must not be empty or whitespace. |
| `config` | object | New configuration. Non-sensitive fields replace the stored set; omitted sensitive fields keep their existing encrypted value. |
| `iconOverride` | string or `null` | This instance's icon, overriding its type's. See below. |
| `tags` | array of strings | Complete replacement tag set. Values are trimmed, duplicates are collapsed, and an empty array clears all tags. |

**`config` replaces all non-sensitive fields.** Sensitive fields are the one
intentional exception: including one replaces it, while omitting one preserves
its existing encrypted value byte-for-byte. Sending an empty string is not
"keep"; clients must omit an untouched sensitive key. Before the connector is
rebuilt, the backend decrypts preserved values into a temporary plaintext copy,
so factories never receive ciphertext.

**`iconOverride` has three request states, not two.** Omitting the key leaves
the stored override alone; sending `null` **clears** it back to the connector
type's own icon; sending a string sets it. The distinction is why the field is
nullable rather than merely optional — collapsing "leave it" and "clear it" into
one request would make a chosen icon impossible to undo. The value follows the
[icon reference convention](#icon-references) and is **not validated**: an
empty or whitespace-only string is stored as `null`, and anything else is stored
as sent, because only a client knows which icons it has.

There is no `iconOverride` on `POST /connector-instances`. A new instance has
nothing to be distinguished from yet, and one field with one place to set it is
one fewer way for the two to disagree.

The live connector is rebuilt and replaced on every successful update, whether
or not `config` changed. That is also how an instance that failed to load at
startup gets a second chance once its configuration is fixed. As with create,
the response does not wait for the replacement's full status inventory, so its
status is initially null until the scheduled background poll completes.

When `tags` is present, replacing the tag rows and updating the instance happen
in one database transaction. Omitting `tags` leaves the existing set alone;
sending an empty array removes every assignment. Empty or whitespace-only tag
values are rejected.

**Response 200** — the same body as `GET /connector-instances/{id}`.

| Status | Meaning |
| --- | --- |
| 200 | Updated, persisted, and the live connector replaced. |
| 400 | `name` was present and empty, a tag was empty, or the connector refused the new `config`. Nothing is changed. |
| 403 | The caller lacks a global `connectors.manage` grant. |
| 404 | No instance with that id. |
| 422 | `iconOverride` was present and was neither a string nor `null`. |

### `DELETE /connector-instances/{id}`

Requires a global `connectors.manage` grant. Removes the row and drops the live
connector.

**Response 204** — no body.

| Status | Meaning |
| --- | --- |
| 204 | Deleted. |
| 403 | The caller lacks a global `connectors.manage` grant. |
| 404 | No instance with that id. |

Dashboard placements reference connector instances with `ON DELETE CASCADE`, so
deleting an instance also removes every placement that embeds it. Dashboards
themselves remain; only the now-invalid placements disappear.

### `POST /connector-instances/{id}/actions/{actionId}`

Requires `connectors.control` over **this instance**, checked as
`connector` / `{id}`. A global grant covers every instance; a grant scoped to
one instance covers only that one.

**Request:** an optional JSON body. The target-aware envelope is:

```json
{ "targetId": "web", "params": { "force": true } }
```

`targetId` is optional and `null` addresses the host/aggregate view. For wire
compatibility, a body without `targetId` or `params` is treated as the params
object directly. An empty body becomes JSON `null` params.

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

After any action returns, Loom schedules an immediate background status poll.
The action response does not wait for that inventory: the action itself is the
request's result, while status updates continue through the normal polling and
WebSocket path.

**A valid `actionId` is one the connector currently advertises anywhere.** That
means its top-level `actions` list *or* the `rowActions`/`kindActions` of any of
its [resource kinds](#resource-browser) — both are dispatched identically and
carry the same permission requirement. An id advertised in neither is refused
with 404 before anything is dispatched. The one exception is a connector that
currently advertises *nothing at all*, which is what an unreachable service
looks like: the call is passed through so the connector can state its real
problem instead of being told its actions do not exist.

A 403 is returned **before** the instance id is looked up, so an unauthorized
caller gets the same response whether or not the id exists. Otherwise the
endpoint would report 404 for unknown ids and 403 for real ones, which is a way
to enumerate what is configured.

Permission scoping remains instance-wide. A `connectors.control` grant over one
Docker instance therefore permits actions on every container sub-target within
that daemon connection. This is less granular than the former per-container
instance model; per-target grants are a possible future extension, not part of
this contract.

| Status | Meaning |
| --- | --- |
| 200 | The connector was reached and produced an `ActionResult` — successful or not. |
| 400 | The request body was present but not valid JSON. Loom's error shape. |
| 400 | `ConnectorError::InvalidParams` — the action exists, the parameters do not satisfy it. |
| 403 | The caller lacks `connectors.control` over this instance. |
| 404 | No instance with that id. Loom's error shape, with **no** `connectorError` — no connector ran. |
| 404 | `ConnectorError::InvalidAction` — the connector exists, the action id does not. |
| 502 | `ConnectorError::Unreachable` or `ConnectorError::AuthFailed`. |
| 500 | `ConnectorError::Internal`. |

## Dashboards

Dashboards use a dedicated per-object ACL: **owner**, **editor**, and
**viewer**. This is intentionally separate from the group/permission RBAC
described above. A dashboard share is an end-user decision about one dashboard;
`connectors.*`, `users.*`, and `groups.*` grants are administrator-managed
system capabilities.

The two systems are orthogonal:

- Sharing a dashboard does not grant `connectors.view` or
  `connectors.control`.
- A connector grant does not reveal or make editable a dashboard that was not
  shared with the caller.
- A placement may display the cached connector summary embedded in the shared
  dashboard. Any action invoked from it still calls the existing connector
  action endpoint, which checks the viewer's own `connectors.control` grant.

Every dashboard endpoint requires a valid access token, but none requires an
RBAC permission key. A dashboard-role failure and an RBAC permission failure
both use HTTP 403 because both mean "authenticated, but not authorized"; their
authorization sources and error messages are different. A missing dashboard is
also returned as 403 when role resolution finds no access, so callers cannot
use dashboard detail paths to enumerate private dashboard ids.

Role ordering is `owner > editor > viewer`:

| Role | View | Pin for self | Add/edit/remove placements and groups | Rename/delete/share |
| --- | --- | --- | --- | --- |
| owner | yes | yes | yes | yes |
| editor | yes | yes | yes | no |
| viewer | yes | yes | no | no |

Combining placements into one tile is a layout edit and sits in the same column
as the rest of them — see [Placement groups](#placement-groups).

### `GET /dashboards`

Lists dashboards the caller owns, receives directly, or receives through any
group membership. Pinned dashboards sort first, then by name.

```json
[
  {
    "id": "be676fe1-a863-48d0-b8e9-86d83a671d6a",
    "name": "Operations",
    "role": "editor",
    "pinned": true
  }
]
```

An empty accessible set is `200 []`.

### `POST /dashboards`

Creates a dashboard owned by the caller.

```json
{ "name": "Operations" }
```

**Response 201** is the dashboard summary with `role: "owner"` and
`pinned: false`.

| Status | Meaning |
| --- | --- |
| 201 | Dashboard created. |
| 400 | `name` is empty or whitespace. |

### `GET /dashboards/{id}`

Requires Viewer or better. Returns the resolved owner, the caller's effective
role, and the dashboard's tiles **in two lists**: `placements` holds the
standalone ones and `placementGroups` holds the combined ones, each with its
ordered members.

The split is done here rather than left to the client. A single flat array with
a `groupId` on each entry would make every client re-derive the same partition
and the same member ordering, and get one of them wrong in a different way each
time. A placement that is a member of a group appears **only** inside that
group, never in `placements`.

```json
{
  "id": "be676fe1-a863-48d0-b8e9-86d83a671d6a",
  "name": "Operations",
  "owner": {
    "id": "48ae87dc-fc35-42db-a3de-467677ff8061",
    "username": "owner"
  },
  "role": "viewer",
  "createdAt": "2026-08-21T18:00:00+00:00",
  "placements": [
    {
      "id": "cab30488-a7b8-4746-95b3-4a5fbfbb0e94",
      "connector": {
        "id": "5aa2574d-9ba0-4af8-b7ae-74671fb48777",
        "name": "Media server",
        "connectorType": "debug",
        "createdAt": "2026-08-21T17:00:00+00:00",
        "metadata": {
          "id": "debug",
          "name": "Debug fixture",
          "icon": null,
          "version": "1.0.0",
          "minSize": [2, 2]
        },
        "status": {
          "health": "healthy",
          "details": { "": {} },
          "lastChecked": "2026-08-21T18:00:05Z"
        },
        "displayFields": []
      },
      "targetId": null,
      "positionX": 0,
      "positionY": 0,
      "width": 2,
      "height": 2,
      "widgetBindings": [],
      "createdAt": "2026-08-21T18:00:00+00:00",
      "groupId": null
    }
  ],
  "placementGroups": [
    {
      "id": "3d6c1f0a-2b1e-4f8c-9a77-0d2b6f4e1c33",
      "name": "Core services",
      "icon": "lucide:network",
      "positionX": 0,
      "positionY": 2,
      "width": 6,
      "height": 3,
      "createdAt": "2026-08-21T18:10:00+00:00",
      "members": [
        { "id": "…", "connector": { "…": "…" }, "groupId": "3d6c1f0a-2b1e-4f8c-9a77-0d2b6f4e1c33", "…": "…" },
        { "id": "…", "connector": { "…": "…" }, "groupId": "3d6c1f0a-2b1e-4f8c-9a77-0d2b6f4e1c33", "…": "…" }
      ]
    }
  ]
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `placements` | array | Standalone placements only. | Always present; may be empty. |
| `placementGroups` | array | Combined tiles. See [Placement groups](#placement-groups). | Always present; may be empty. |

A member is a placement object exactly as `placements` holds one, including its
own `positionX`/`positionY`/`width`/`height` — see
[Placement groups](#placement-groups) for why those are still there and what
they mean while grouped. `members` is ordered; render it in the order given.

`connector` is exactly the cached summary builder used by
`GET /connector-instances`; failed and unloaded connectors therefore keep the
same `status: null` plus `statusError` behavior.

| Status | Meaning |
| --- | --- |
| 200 | Dashboard returned. |
| 403 | Caller has no dashboard role. |

### `PATCH /dashboards/{id}`

Owner only. Editors deliberately cannot rename.

```json
{ "name": "Renamed operations" }
```

**Response 200** is the full dashboard detail.

| Status | Meaning |
| --- | --- |
| 200 | Renamed. |
| 400 | `name` is empty or whitespace. |
| 403 | Caller is not the owner. |

### `DELETE /dashboards/{id}`

Owner only. Returns 204 and cascade-deletes the dashboard's shares, pins, and
placements.

| Status | Meaning |
| --- | --- |
| 204 | Dashboard and dependent rows deleted. |
| 403 | Caller is not the owner. |

### `POST /dashboards/{id}/pin`

### `DELETE /dashboards/{id}/pin`

Viewer or better. These idempotently add or remove only the caller's pin and
return 204. Pins never affect another user's list. A caller without dashboard
access receives 403.

### `GET /dashboards/{id}/shares`

Owner only. The display name is resolved at read time from `users.username` or
`groups.name`:

```json
[
  {
    "id": "a0764c1c-72de-435b-b099-335fc7189d88",
    "targetType": "group",
    "targetId": "b4817a8c-e18c-42c4-b5e8-e9f1343514b8",
    "role": "view",
    "resolvedName": "Household",
    "createdAt": "2026-08-21T18:00:00+00:00"
  }
]
```

### `POST /dashboards/{id}/shares`

Owner only.

```json
{
  "targetType": "user",
  "targetId": "48ae87dc-fc35-42db-a3de-467677ff8061",
  "role": "edit"
}
```

`targetType` is `user` or `group`; `role` is `view` or `edit`. The target must
exist in the corresponding table before the share is inserted.

| Status | Meaning |
| --- | --- |
| 201 | Share created; response is one share object. |
| 400 | Invalid type/role or target does not exist. |
| 403 | Caller is not the dashboard owner. |
| 409 | That dashboard already has a share for this target. |

When direct and group shares overlap, the highest applicable role wins.
Ownership always wins over every share.

### `DELETE /dashboards/{id}/shares/{shareId}`

Owner only. Returns 204 after revocation, which takes effect immediately rather
than waiting for access-token refresh. Returns 404 when that share id does not
belong to the dashboard, and 403 when the caller is not the owner.

### `POST /dashboards/{id}/placements`

Editor or Owner.

```json
{
  "connectorInstanceId": "5aa2574d-9ba0-4af8-b7ae-74671fb48777",
  "targetId": "web",
  "positionX": 0,
  "positionY": 0,
  "width": 3,
  "height": 2,
  "widgetBindings": [
    { "display": { "dataPointId": "load", "widgetType": "gauge", "config": {} } },
    { "action": { "actionId": "restart", "widgetType": "button", "config": {} } }
  ]
}
```

`targetId` is optional/null for the host view. When present, the connector must
support sub-targets and the id must appear in a live enumeration.
`widgetBindings` may be omitted, in which case the connector's target-specific
default layout is stored. Width and height must each meet the live connector's
`metadata.minSize`.

Each binding is validated against the namespace its own tag names — see
[`WidgetLayout`](#widgetlayout):

- a `display` binding's `dataPointId` and descriptor `targetId` must match the
  placement's `targetId`;
- an `action` binding's `actionId` and descriptor `targetId` must match the
  placement's `targetId`.

A 400 lists every invalid id, and says which kind each one is, so the two are
never confused: `widget bindings reference unknown data points: nope; unknown
actions: also-nope`.

The connector row must exist and its live connector must be available so the
metadata contract can be validated. No connector ownership or
`connectors.control` grant is inferred or created.

| Status | Meaning |
| --- | --- |
| 201 | Placement created; response is the placement shape from dashboard detail. |
| 400 | Connector missing/unavailable, unknown/unsupported target, size below minimum, or invalid binding. |
| 403 | Caller is not an Editor or Owner. |

### `PATCH /dashboards/{id}/placements/{placementId}`

Editor or Owner. Any omitted field remains unchanged:

```json
{ "positionX": 2, "positionY": 1, "width": 4, "height": 3 }
```

`positionX`, `positionY`, `width`, `height`, `targetId`, and `widgetBindings` are mutable;
the connector instance is fixed. Size and binding validation is identical to
create. Returns the updated placement on 200, 403 for insufficient role, 404
when the placement does not belong to this dashboard, and 400 for validation
failure.

This endpoint works on a **grouped** placement too, and what it edits then is
the standalone geometry the placement will return to when it is ungrouped — not
where it sits inside its group. See [Placement groups](#placement-groups).

### `DELETE /dashboards/{id}/placements/{placementId}`

Editor or Owner. Returns 204, 403 for insufficient role, or 404 when the
placement does not belong to this dashboard.

**Deleting a group member can dissolve its group.** If the deletion leaves a
placement group with fewer than two members, that group is removed and its
remaining member returns to standalone. See
[auto-dissolve](#a-group-below-two-members-dissolves).

## Placement groups

Several placements combined into one wider tile. Grouping is **retroactive**
(nothing at placement-creation time decides it), **connector-agnostic** (members
need not share a type, or anything else), and holds **any number of members from
two upward** — nothing in the model is pairwise.

Every endpoint below requires **Editor or Owner** on the dashboard, exactly like
the placement endpoints. There is no permission key: grouping is a layout edit,
and layout edits are an ACL question — see
[`0013`](./adr/0013-dashboard-sharing-model.md) and
[`0015`](./adr/0015-dashboard-tile-grouping.md).

Each group also has a user-facing `name` and optional generic `icon` reference.
`icon: null` means the client uses its group fallback. Both are properties of
the combined tile, not of any member connector.

### A member keeps its own position and size

A group has its own `positionX`/`positionY`/`width`/`height`, and **that box is
what the grid lays out**. A member's own four geometry fields are still returned
and are still writable, but they are **ignored for grid rendering** while it is
grouped.

They are not cleared, and that is deliberate: ungrouping is a write of `null` to
two columns, after which every placement renders standalone again exactly where
and at what size it was before it was grouped. Grouping is therefore an
experiment a user can undo, not a decision that costs them their layout.

The consequence worth knowing: for a grouped placement those fields are
stale-by-design. Do not use them to position a member inside its group.

### A group below two members dissolves

**If a group's membership drops below two, the group is deleted and any
remaining member returns to standalone.** A group of one is the placement it
contains with an extra layer of indirection, so it is not allowed to exist.

This means **removing one member of a pair un-groups both placements and
destroys the tile** — including the placement that was not named in the request.

Membership can fall below two through three routes, and the rule holds on all
of them:

| How | Endpoint |
| --- | --- |
| A member is removed from the group | `DELETE …/placement-groups/{groupId}/members/{placementId}` |
| A member placement is deleted outright | `DELETE …/placements/{placementId}` |
| The connector instance behind a member is deleted, cascading its placements away | `DELETE /connector-instances/{id}` |

Because a dissolve can change tiles the caller did not ask about, the two
deleting endpoints return **204 with no body**. Re-read `GET /dashboards/{id}`
rather than trying to patch local state from the response.

### Member ordering

`members` is ordered, and `POST` sets the initial order from the order of
`placementIds`. Ordering is a sort key, not an index: removing a member leaves a
gap, and adding one appends past the current last member rather than filling
that gap. Only the relative order is ever meaningful.

### `POST /dashboards/{id}/placement-groups`

Editor or Owner. Combines existing placements into a new tile.

```json
{
  "placementIds": [
    "cab30488-a7b8-4746-95b3-4a5fbfbb0e94",
    "7f2e9d10-3c4b-4a58-8e6f-1b0d2c3a4e5f"
  ],
  "name": "Core services",
  "icon": "lucide:network",
  "positionX": 0,
  "positionY": 2,
  "width": 6,
  "height": 3
}
```

| Field | JSON type | Meaning |
| --- | --- | --- |
| `placementIds` | array | **At least two**, each once, each a placement on this dashboard, none already in a group. Their order becomes the initial member order. |
| `name` | string | Optional display name. Trimmed and non-empty when supplied; omitted generates `Group of N`. |
| `icon` | string or `null` | Optional group icon reference. Omitted or `null` uses the generic group fallback. |
| `positionX`, `positionY` | integer | The tile's grid position. |
| `width`, `height` | integer | The tile's grid size. Both at least 1. |

**Response 201** — the created group, in the same shape `placementGroups`
carries in dashboard detail, members resolved.

Every rejection below is a 400, because every one of them is a failure of the
request body rather than a missing resource at the URL. Each names the offending
ids, since "some placements cannot be grouped" is not actionable on its own.

| Status | Meaning |
| --- | --- |
| 201 | Group created. |
| 400 | Fewer than two ids, a repeated id, an id that is not a placement on this dashboard, an id already in a group, or a `width`/`height` below 1. |
| 403 | Caller is not an Editor or Owner. |

### `PATCH /dashboards/{id}/placement-groups/{groupId}`

Editor or Owner. Renames or re-icons the tile, moves or resizes it, reorders its members, or combines those changes. Every
field is optional; an omitted one is left alone, and an empty body is a no-op.

```json
{
  "name": "Storage cluster",
  "icon": "lucide:hard-drive",
  "positionX": 2,
  "height": 4,
  "memberOrder": [
    "7f2e9d10-3c4b-4a58-8e6f-1b0d2c3a4e5f",
    "cab30488-a7b8-4746-95b3-4a5fbfbb0e94"
  ]
}
```

`memberOrder` must name **exactly** the current membership — the same ids, each
once, nothing added and nothing missing. Reordering is not a back door for
joining or leaving a group: those have their own endpoints, with their own
validation and, in the leaving case, a cascade this endpoint must not silently
trigger.

Omitting `name` or `icon` leaves it alone. `name` is trimmed and cannot be
empty; `icon: null` explicitly clears the assignment back to the group default.

**Response 200** — the updated group with its members in the new order.

| Status | Meaning |
| --- | --- |
| 200 | Updated. |
| 400 | `name` is empty, `memberOrder` does not match current membership, or `width`/`height` below 1. |
| 403 | Caller is not an Editor or Owner. |
| 404 | No such group on this dashboard. |

### `POST /dashboards/{id}/placement-groups/{groupId}/members`

Editor or Owner. Appends one standalone placement after the current last member.

```json
{ "placementId": "9c8b7a65-4321-4f0e-9d8c-7b6a5f4e3d2c" }
```

A placement already in **another** group is refused rather than moved: leaving a
group can dissolve it, and a request that says "add" must not be the thing that
destroys a different tile. Ungroup it first.

**Response 200** — the updated group, members resolved.

| Status | Meaning |
| --- | --- |
| 200 | Added. |
| 400 | `placementId` is not a placement on this dashboard, or is already in a group. |
| 403 | Caller is not an Editor or Owner. |
| 404 | No such group on this dashboard. |

### `DELETE /dashboards/{id}/placement-groups/{groupId}/members/{placementId}`

Editor or Owner. Removes one member, returning it to standalone at its preserved
position and size.

**This can delete the group.** If the removal leaves fewer than two members, the
group is dissolved: any remaining member is also returned to standalone and the
group row is deleted. See [auto-dissolve](#a-group-below-two-members-dissolves).

**Response 204** — no body, because a dissolve may have changed placements the
caller did not name. Re-read the dashboard.

| Status | Meaning |
| --- | --- |
| 204 | Removed; the group may no longer exist. |
| 403 | Caller is not an Editor or Owner. |
| 404 | No such group on this dashboard, or that placement is not one of its members. |

### `DELETE /dashboards/{id}/placement-groups/{groupId}`

Editor or Owner. Splits the tile apart in one action: **every** member returns to
standalone at its preserved position and size, and the group row is deleted.
**No placement is deleted.**

Distinct from removing members one at a time, which for a group of three would
take two requests and dissolve the group on the second anyway.

**Response 204** — no body.

| Status | Meaning |
| --- | --- |
| 204 | Group deleted; every member is standalone again. |
| 403 | Caller is not an Editor or Owner. |
| 404 | No such group on this dashboard. |

## Account

Self-service routes: a signed-in user managing their **own** account. Every one
of them requires a valid access token and **no permission grant at all**.

That is not an oversight. The structural reason is that none of these paths
takes a user id — the subject is read from the token's `sub` claim, so there is
no value a caller could supply to reach somebody else's account. Requiring
`users.manage` here would produce an instance where an ordinary user cannot
change their own password. Acting on *another* user stays where it belongs, in
[Administration](#administration), which does take an id and does require a
grant.

All of them return **401** without a valid token; that case is not repeated in
the tables below.

### `GET /account`

**Response 200:**

```json
{
  "id": "9d1f8c2e-4b7a-4c3d-9e21-0a5b6c7d8e9f",
  "username": "admin",
  "displayName": "The Admin",
  "avatarUrl": "/avatars/2f1c8b90-5d3e-4a71-9c02-6b8d4e1f7a35.png",
  "createdAt": "2026-08-19T17:04:11.882401553+00:00",
  "groups": [
    { "id": "00000000-0000-4000-8000-000000000001", "name": "Administrators" }
  ]
}
```

`displayName` and `avatarUrl` are `null` when unset. As everywhere else in this
API, **there is no password field**.

`groups` is included for context and is **read-only here**. Membership is an
administrative decision made through [`PATCH /users/{id}`](#patch-usersid);
offering it on a self-service route would be offering privilege escalation with
a friendly label.

#### `avatarUrl` is relative

`avatarUrl` is a path, not an absolute URL — `/avatars/{uuid}.{ext}`. **Resolve
it against the same base URL as the API itself.**

It has to be relative, because the backend does not know the origin it is being
reached through. The web frontend reaches it through an `/api` proxy on its own
origin; desktop and mobile reach a user-supplied server URL directly; a reverse
proxy may put it somewhere else again. Any absolute URL the backend invented
would be wrong for at least one of them.

Note the one asymmetry this creates for the web frontend: `/api` is stripped
before the backend sees a request, but the avatar path is served by the backend
at `/avatars/…`, so the frontend requests it at `/api/avatars/…` — the same
transformation it applies to every other path in this document.

Avatar files are served **unauthenticated**, by a read-only static file service
that answers GET and HEAD only. Browsers do not attach an `Authorization` header
to `<img>` loads, so authenticating them would require cookies or signed URLs.
What is exposed is a profile picture, to someone who can already reach the
server *and* guess a random UUIDv4 filename.

| Status | Meaning |
| --- | --- |
| 200 | The profile. |
| 404 | The account behind this token no longer exists — deleted mid-session. |

### `PATCH /account`

Both fields optional; an absent field is left alone.

```json
{ "username": "renamed", "displayName": "The Admin" }
```

`displayName` distinguishes **absent** from **present and null**: omitting it
keeps the current value, sending `null` clears it. An all-whitespace string also
clears it, since `"   "` is not a name and would render as a blank where a name
belongs. Usernames are trimmed.

Uniqueness is checked **excluding the caller's own row**, so submitting a form
that echoes back the current username is not a conflict with itself.

**Response 200** — the updated profile, in the `GET /account` shape.

| Status | Meaning |
| --- | --- |
| 200 | Applied. |
| 400 | Empty username. |
| 409 | That username is taken by another account. |

#### Renaming yourself and the token in your hand

The access token embeds `username` at issuance, so a token minted before the
rename keeps reporting the old name until it expires. Nothing invalidates it and
nothing needs to: access tokens live 15 minutes, and the next refresh mints one
carrying the new name. This is the same staleness window that already applies to
[permission changes](#permission-enforcement).

It is display-only staleness. Every handler identifies the caller by `sub`, the
user id, which never changes — so a stale `username` claim can show an
out-of-date name for a few minutes, but cannot address the wrong account.

### `POST /account/password`

```json
{ "currentPassword": "…", "newPassword": "…" }
```

`currentPassword` is verified against the stored argon2 hash before anything is
written. The password floor is **8 characters**, the same constant `POST /setup`
and `POST /users` use.

**Response 204**, no body.

| Status | Meaning |
| --- | --- |
| 204 | Changed. |
| 400 | New password under 8 characters. |
| 401 | `currentPassword` is wrong. |
| 404 | The account no longer exists. |

The 401 here carries a **distinct message** — `current password is incorrect` —
and is deliberately not the login route's uniform rejection. Login gives one
identical answer for every failure because distinguishing them tells an
anonymous caller which usernames exist. Here the caller is already authenticated
as this exact account, so there is nothing left to disclose, and naming the
wrong field is the only way a client can say *which* of the two inputs to fix.

**Existing sessions survive a password change.** Nothing is revoked, so a
session established beforehand keeps working — and the bound on that is the
*refresh* token's seven days, not the access token's fifteen minutes. A user may
end those sessions explicitly from Account's Active Sessions section.

### `POST /account/avatar`

`multipart/form-data` with a single file field. Fields without a filename are
skipped, so a client that also sends text parts is not rejected for it.

**Response 200:**

```json
{ "avatarUrl": "/avatars/2f1c8b90-5d3e-4a71-9c02-6b8d4e1f7a35.png" }
```

Accepted formats are **PNG, JPEG, and WebP**, at most **2 MB**.

| Status | Meaning |
| --- | --- |
| 200 | Stored. |
| 400 | No file field, or the bytes are not a decodable image in an accepted format. |
| 413 | The file is larger than 2 MB. |
| 404 | The account no longer exists. |

#### What is validated, and why the header is not

The declared `Content-Type` is **never consulted**. It is a string the caller
chose, so a shell script announced as `image/png` passes any header check. The
format is taken from the content's own magic bytes, and the image is then
**decoded in full** rather than having its header parsed — a truncated or
corrupt file has a perfectly valid header and would otherwise be stored as a
picture that no client can render.

Decoding is bounded by an allocation ceiling separate from the 2 MB byte limit,
because compressed size says nothing about decoded size: a small, valid PNG can
declare enormous dimensions and expand into gigabytes. A file that trips the
ceiling is a 400, like any other unusable upload.

The stored filename is `{uuid_v4}.{ext}` and is **never derived from the
uploaded filename**, which is caller-controlled. Uploading a replacement deletes
the previous file, so avatars do not accumulate on disk; because each upload
gets a fresh name, a cached URL never shows a stale picture.

### `DELETE /account/avatar`

Deletes the stored file, if any, and clears the field.

**Response 200** — the updated profile, in the `GET /account` shape, with
`avatarUrl` now `null`. The whole profile rather than an acknowledgement, so a
client re-renders from one authoritative answer.

Deleting when there is no avatar is **not** an error: the end state is the one
the caller asked for.

| Status | Meaning |
| --- | --- |
| 200 | Cleared. |
| 404 | The account no longer exists. |

## Administration

All of these require a **global** grant, and all return **403** without one and
**401** without a valid token. Those two cases are not repeated in the tables
below.

### `GET /users`

Requires `users.manage`.

**Response 200:**

```json
[
  {
    "id": "9d1f8c2e-4b7a-4c3d-9e21-0a5b6c7d8e9f",
    "username": "admin",
    "isActive": true,
    "createdAt": "2026-08-19T17:04:11.882401553+00:00",
    "groupIds": ["00000000-0000-4000-8000-000000000001"]
  }
]
```

**There is no password field, and there must never be one.** A password hash is
not secret in the way a password is, but publishing it hands an attacker an
offline target they can work on at their own pace.

### `POST /users`

Requires `users.manage`.

```json
{
  "username": "housemate",
  "password": "a-good-password",
  "groupIds": ["<group id>"]
}
```

`groupIds` may be omitted or empty — an account with no groups can sign in and
do nothing, which is a valid state. The password floor is **8 characters**, the
same constant `POST /setup` uses, so the rule cannot drift between the two ways
an account is created.

**Response 201** — the created user, in the `GET /users` shape.

| Status | Meaning |
| --- | --- |
| 201 | Created. |
| 400 | Empty username, password under 8 characters, or an unknown group id. |
| 409 | That username is taken. |

### `PATCH /users/{id}`

Requires `users.manage`. Both fields are optional; an absent field is left
alone.

```json
{ "isActive": false, "groupIds": ["<group id>"] }
```

`groupIds` **replaces** membership wholesale rather than applying a delta: the
caller states the membership it wants and gets exactly that, with no dependence
on what it believed the previous state to be.

**Response 200** — the updated user.

| Status | Meaning |
| --- | --- |
| 200 | Applied. |
| 400 | An unknown group id. |
| 404 | No such user. |
| 409 | [A safeguard refused it](#safeguards): this is you, or it would leave no active administrator. |

### `GET /users/{id}/sessions`

Lists the user's active refresh-token sessions, newest first. A caller may
always name their own user id; naming anybody else requires global
`users.manage`. Revoked and expired rows are retained for token-history
semantics but omitted from this view.

**Response 200:**

```json
[
  {
    "id": "32a847f9-448b-4d33-a9e5-b2950299c435",
    "createdAt": "2026-08-30T12:00:00Z",
    "expiresAt": "2026-09-06T12:00:00Z",
    "userAgent": "Mozilla/5.0 (...) Chrome/140.0.0.0 Safari/537.36",
    "ipAddress": "203.0.113.10",
    "isCurrent": true
  }
]
```

`userAgent` and `ipAddress` are nullable for sessions created before context
capture existed or by a caller that supplied no user agent. `isCurrent` is true
only when the listed row is the refresh-token session named by the requester's
access token; it is never inferred from an IP or browser string.

| Status | Meaning |
| --- | --- |
| 200 | Active sessions returned. |
| 403 | The target is another user and the caller lacks global `users.manage`. |
| 404 | No such user. |

### `DELETE /users/{id}/sessions/{sessionId}`

Revokes one refresh-token session. The same self-or-`users.manage` access rule
applies. **Response 204**, including when that session is already revoked or
expired; repeating a revocation is harmless. The session id must belong to the
user id in the path.

| Status | Meaning |
| --- | --- |
| 204 | Processed, including when the session is unknown or already ended. |
| 403 | The target is another user and the caller lacks global `users.manage`. |
| 404 | No such user. |

### `DELETE /users/{id}/sessions`

Revokes every active refresh-token session for that user. The same
self-or-`users.manage` access rule applies. For self-service this deliberately
includes the caller's current session: clients clear their locally stored token
pair after the 204 and return to login. **Response 204**, including when there
are no active sessions.

Revocation prevents any affected refresh token from minting another access
token. An access token already issued remains valid until its 15-minute expiry,
because ordinary authenticated requests verify its signature without a
database lookup.

| Status | Meaning |
| --- | --- |
| 204 | Processed. |
| 403 | The target is another user and the caller lacks global `users.manage`. |
| 404 | No such user. |

### `DELETE /users/{id}`

Requires `users.manage`. **Response 204**, no body.

A hard delete when the user owns no dashboards: `ON DELETE CASCADE` takes group
memberships and refresh tokens with it, ending their sessions. Dashboard
ownership is a restricting foreign key. An owner must delete their dashboards
first; Loom does not silently erase user-authored dashboards as a side effect of
account administration. Deactivation remains available when content must be
retained.

| Status | Meaning |
| --- | --- |
| 204 | Deleted. |
| 404 | No such user. |
| 409 | [A safeguard refused it](#safeguards): this is the caller, it would remove the last administrator, or the user still owns dashboards. |

### `GET /groups`

Requires `groups.manage`.

**Response 200:**

```json
[
  {
    "id": "00000000-0000-4000-8000-000000000001",
    "name": "Administrators",
    "description": "Full access to every permission across every resource.",
    "createdAt": "2026-08-19T00:00:00Z",
    "isProtected": true,
    "memberCount": 1,
    "permissions": [
      { "key": "connectors.control", "resourceType": null, "resourceId": null }
    ]
  }
]
```

`isProtected` marks a group that cannot be deleted. Clients should hide or
disable the delete control for it rather than let the user find out through a
409.

### `POST /groups`

Requires `groups.manage`.

```json
{
  "name": "Viewers",
  "description": "Read-only access.",
  "permissions": [
    { "key": "connectors.view", "resourceType": null, "resourceId": null }
  ]
}
```

**Response 201** — the created group. New groups are never protected.

| Status | Meaning |
| --- | --- |
| 201 | Created. |
| 400 | Empty name, or a permission key not in the catalog. |
| 409 | That name is taken. |

An unregistered permission key is a **400**, not a silently stored grant. The
foreign key onto `permissions` is what enforces it, so a typo like
`connectors.contorl` fails loudly instead of becoming a grant that authorizes
nothing and looks correct in a UI.

### `PATCH /groups/{id}`

Requires `groups.manage`. All fields optional; absent means unchanged.

```json
{ "name": "Operators", "description": null, "permissions": [] }
```

`permissions` replaces the group's grants wholesale, on the same reasoning as
user membership.

A protected group **may** be renamed and re-granted — only deletion is refused.
Blocking edits would push operators into working around the safeguard rather
than with it.

| Status | Meaning |
| --- | --- |
| 200 | Applied. |
| 400 | Empty name, or an unregistered permission key. |
| 404 | No such group. |

### `DELETE /groups/{id}`

Requires `groups.manage`. **Response 204**, no body.

Memberships and grants go with it via cascade. Users are not touched: losing a
group removes what it granted, not the accounts that held it.

| Status | Meaning |
| --- | --- |
| 204 | Deleted. |
| 404 | No such group. |
| 409 | The group is protected. See [Safeguards](#safeguards). |

### `GET /permissions`

Requires `groups.manage` — assigning grants is the only thing this is for.

**Response 200:**

```json
[
  { "key": "connectors.control", "description": "Execute actions on connectors." },
  { "key": "connectors.view",    "description": "View connectors and their status." },
  { "key": "groups.manage",      "description": "Create and modify groups and their permission grants." },
  { "key": "system.settings",    "description": "Change instance-wide settings." },
  { "key": "users.manage",       "description": "Create, modify, and deactivate user accounts." }
]
```

The catalog exists so a client can build a grant-assignment form without
hardcoding a list that would silently fall out of date the next time a migration
registers a key.

## `ConnectorError` to HTTP status

The dividing line: `InvalidAction` and `InvalidParams` are the caller's mistake;
everything else is Loom failing, or being failed by the upstream service.

| Variant | Status | Reasoning |
| --- | --- | --- |
| `InvalidAction` | 404 Not Found | Consistent with an unknown instance id — the path `/connector-instances/{id}/actions/{actionId}` names something that is not there. |
| `InvalidParams` | 400 Bad Request | The request reached a real action and was malformed. |
| `InvalidConfig` | 400 Bad Request | The submitted connector configuration was refused by the connector itself. Returned by `POST` and `PATCH /connector-instances`. |
| `AuthFailed` | **502 Bad Gateway** | Deliberately *not* 401. It means the *upstream service* rejected *Loom's* stored credentials. The caller is not the party that failed to authenticate and holds no credentials that would fix it; a 401 would tell a client to re-prompt its user, which cannot repair a bad token in Loom's connector configuration. It is a gateway failure, like `Unreachable`. |
| `Unreachable` | 502 Bad Gateway | Loom could not reach the upstream at all. |
| `Internal` | 500 Internal Server Error | The failure is inside Loom. |

Plus, outside the enum: an **unknown instance id is 404**, and an unregistered
`connectorType` is **400**, both with `error` set and `connectorError` omitted,
since no connector was invoked.

## Core wire types

These live in `crates/core/src/connector/mod.rs` and are handed to the HTTP
layer unchanged. The examples below are the real serialized output — they can be
regenerated with `cargo test --package loom-core -- --nocapture
print_wire_shapes`.

### `ConnectorStatus`

```json
{
  "health": "degraded",
  "targetHealth": {
    "": "degraded",
    "fixture-a": "healthy"
  },
  "details": {
    "": {
      "load": 62.5,
      "label": "debug-1",
      "enabled": true,
      "loadHistory": [
        { "timestamp": "2026-08-19T11:59:55Z", "value": 61.2 },
        { "timestamp": "2026-08-19T12:00:00Z", "value": 62.5 }
      ],
      "version": "1.2.3"
    },
    "fixture-a": { "load": 51.2, "enabled": true }
  },
  "lastChecked": "2026-08-19T12:00:00Z"
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `health` | string | One of `"healthy"`, `"degraded"`, `"down"`, `"unknown"`. | Always present. |
| `targetHealth` | object | Health keyed exactly like `details`: `""` for the host/aggregate view and sub-target ids for addressable targets. Clients fall back to aggregate `health` when a key is absent. | Always present in new responses; `{}` is valid and older serialized values without the field deserialize as empty. |
| `details` | object | Readings nested first by target and then by `DataPointDescriptor.id`. The empty-string key `""` is the host/aggregate sentinel; every other key is a sub-target id. | Always present; `{}` only for a connector with no data points and nothing else to report. |
| `lastChecked` | string | RFC 3339 UTC, `Z`-suffixed. When the reading was actually taken. | Always present. |

`health` is a closed set of four rather than a free-form string because clients
sort, colour, and alert on it. `"unknown"` is distinct from `"down"` so a
dashboard never reports an outage it has not observed — never-polled is not the
same as broken.

**`details` has the formal shape `details[targetKey][dataPointId]`.** It is
typed loosely in Rust for serialization flexibility, but the nesting is a
contract. `targetKey` is `""` when a descriptor's `targetId` is null, otherwise
it is that exact target id. This permits the same data-point id on many targets
without collisions.

| `valueType` | JSON shape of `details[targetKey][id]` |
| --- | --- |
| `"number"` | a JSON number |
| `"string"` | a JSON string |
| `"bool"` | a JSON boolean |
| `"timeSeries"` | a JSON array of `{ "timestamp": <RFC 3339>, "value": <number> }` objects, **oldest first** |
| `"categoryBreakdown"` | a JSON array of `{ "label": <string>, "value": <number> }` objects, for one bar or pie slice per named category |

A descriptor whose `unit` is `"bytes"` keeps a raw byte number on the wire.
Every compatible numeric widget scales that value for presentation (for
example `8954870480896` as `8.15 TiB`); connectors must not pre-scale it.

A connector may include extra keys that are not data points — a version string,
a queue depth — and a client that does not recognise one ignores it. What it may
not do is key a data point's value under anything else, because a saved
`WidgetBinding.display` stores an id and resolves it here on every poll. That is
precisely why there is no separate "values" endpoint: the status payload pushed
over [`/ws/connectors`](#get-wsconnectors) is already the render input.

`lastChecked` is part of the **value**, not the response envelope. That is what
lets a polled or cached reading stay honest about its own age: if the backend
starts serving a status from a poll loop, the timestamp travelling with the
reading means the client can show "checked 4 minutes ago" instead of implying
the number is live.

### `ConnectorAction`

```json
{
  "id": "restart",
  "targetId": "web",
  "label": "Restart",
  "description": "Restarts the service.",
  "paramsSchema": {},
  "isDisruptive": true,
  "snapshotDataPointIds": ["load"]
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `id` | string | Stable machine identifier, passed back in the action URL. | Always present. |
| `targetId` | string | Addressed sub-target for this descriptor. | `null` for a host/aggregate action. |
| `label` | string | Short human-facing name for a button or menu entry. | Always present. |
| `description` | string | Longer explanation for tooltips and confirmation prompts — the place to warn that an action is disruptive. | Serialized as **`null`** when absent, never omitted. |
| `paramsSchema` | object | JSON Schema for this action's parameters, driving client-side form generation and server-side validation. | Always present; `{}` for a parameterless action, never `null`. |
| `isDisruptive` | boolean | Whether running this makes the service stop answering for a while. Raises a [`pendingOperation`](#pending-operations-and-diagnosis) while it runs. | Always present; `false` unless the connector opts in. |
| `snapshotDataPointIds` | array of string | Data points whose current values are recorded on the [action log](#action-log) entry before this action runs. | Always present; `[]` unless the connector opts in. |

**`isDisruptive` is not "is this dangerous".** The test is whether a user would
be *surprised* by the gap. `stop` takes a service down, but the person who
pressed Stop expects that and does not need it explained; `restart` is the case
where a running service vanishes and is expected back, and the user has no idea
how long the gap should be. Marking an action disruptive when it is not
suppresses a genuine outage behind "Performing…", which is why the default is
`false` and opting in is the deliberate half.

`id` is separate from `label` so renaming a button does not invalidate stored
automations or URLs. `paramsSchema` is `{}` rather than `null` for parameterless
actions so a consumer can always treat it as a schema and never has to
special-case the absence of one.

Delivered in the `actions` array of `GET /connector-instances/{id}`, so a
client never has to know an action id in advance. The list is not fixed for a
connector type: it may vary with the connector's configuration or the remote
service's state, so treat it as data to render, not as a schema to compile
against.

### `SubTarget`

```json
{
  "id": "device:11111111-1111-1111-1111-111111111111",
  "label": "U7 Lite (a4:f6)",
  "kind": "device",
  "icon": "lucide:wifi"
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `id` | string | Stable address used as `targetId` on placements, descriptors, and action requests. | Always present. |
| `label` | string | Current human-facing label for pickers. It may include supplemental information and is not an identifier. | Always present. |
| `kind` | string | Connector-owned category used for badges and other generic presentation. Unknown values remain valid. | Always present; defaults to `target` when an older connector omits it. |
| `icon` | string | Optional curated `lucide:<name>` presentation hint for this target. Unknown references fall back to the connector icon. | Omitted when not supplied. |

Sub-target enumeration is intentionally cheap metadata. Values and action
descriptors remain in the instance detail/status contracts.

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
  "id": "debug",
  "name": "Debug Connector",
  "icon": "lucide:bug",
  "version": "1.0.0",
  "minSize": [2, 2]
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `id` | string | The connector **type**'s identifier, not the instance's. Lowercase kebab-case by convention (`"debug"`, `"reverse-proxy"`). The instance's own id is the sibling `id` on the response envelope. | Always present. |
| `name` | string | Display name shown in the UI. | Always present. |
| `icon` | string | Icon *reference*, not image data — a prefixed name each client resolves against its own icon set. See the two forms below. | Serialized as **`null`** when absent, meaning "the client picks its own fallback". |
| `version` | string | Version of the connector implementation, independent of the Loom release. | Always present. |
| `minSize` | array | `[width, height]` in dashboard grid units: the smallest footprint at which this connector is still readable. A floor the placement UI enforces, not a preferred size. | Always present; a two-element array of integers. |

`icon` carries a name rather than a URL or bytes so that Core ships no assets
and assumes no renderer — the web, desktop, and mobile clients each map the name
onto their own icon set. `version` is the connector's own, so a connector can be
revised without a platform bump.

#### Icon references

An icon string, wherever one appears in this document, takes exactly one of two
prefixed forms:

| Form | Resolves to | Example |
| --- | --- | --- |
| `brand:<key>` | An SVG vendored by the client, `<key>` matching the vendored file's name without its extension. See [`THIRD_PARTY_ICONS.md`](./THIRD_PARTY_ICONS.md) for what the web client has vendored and under which license. | `"brand:docker"` |
| `lucide:<name>` | One member of the client's curated generic icon set, `<name>` in **kebab-case**. | `"lucide:hard-drive"` |

Kebab-case because that is what lucide's own catalog uses; PascalCase is a
detail of one client library's component exports, and a wire format does not get
to depend on it.

**The backend never validates an icon reference.** It stores and returns the
string. Resolution *and fallback* are entirely client-side, because only a
client knows which icons it actually has: a reference naming something a given
client lacks falls through to the next candidate rather than failing the
request or rendering nothing. Nothing about a connector is broken because its
icon is missing.

### `DisplayField`

```json
{ "label": "Host", "value": "debug.invalid" }
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `label` | string | Short caption. | Always present. |
| `value` | string | The value as it should appear, already rendered to text. | Always present. |

**Never derived from `configSchema`.** A connector author writes these out one
by one, in code, and a field that is not written out is not shown. The obvious
automatic alternative would put whatever is in the stored configuration on the
shell, and stored configuration is exactly where credentials live.

### `DataPointDescriptor`

```json
{ "id": "load", "targetId": "web", "label": "Load", "valueType": "number", "unit": "%" }
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `id` | string | Stable machine identifier, and the second-level key under its target in `status.details`. Stored in saved layouts, so it must not change when the label does. | Always present. |
| `targetId` | string | Addressed sub-target for this descriptor. | `null` for a host/aggregate data point. |
| `label` | string | Human-facing name for a caption or legend entry. | Always present. |
| `valueType` | string | One of `"number"`, `"string"`, `"bool"`, `"timeSeries"`, `"categoryBreakdown"`. Constrains which widgets may render it. | Always present. |
| `unit` | string | Display unit (`"%"`, `"bytes"`, `"ms"`). A display concern only — the wire value is never scaled. `"bytes"` is rendered in an appropriate binary unit by every numeric widget. | Serialized as **`null`** for a dimensionless value. |

**Descriptors, not readings.** The current values arrive separately, in
`status.details`, keyed by target and then `id`. That split is what lets a dashboard be laid out
once and re-rendered on every poll without re-reading the schema.

### `WidgetLayout`

```json
{
  "bindings": [
    {
      "display": { "dataPointId": "load", "widgetType": "statTile", "config": {} }
    },
    {
      "display": {
        "dataPointId": "load",
        "widgetType": "gauge",
        "config": { "min": 0, "max": 100 }
      }
    },
    {
      "display": {
        "dataPointId": "loadHistory",
        "widgetType": { "metricChart": { "chartType": "line" } },
        "config": {}
      }
    },
    {
      "action": { "actionId": "set-enabled", "widgetType": "toggle", "config": {} }
    },
    {
      "action": {
        "actionId": "set-load",
        "widgetType": "slider",
        "config": { "min": 0, "max": 100, "step": 1 }
      }
    }
  ]
}
```

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `bindings` | array | The widgets, in the connector author's suggested reading order. | Always present; may be empty. |

**Each binding is externally tagged** — a single-key object whose key is either
`"display"` or `"action"`, the same shape as `ConnectorError`. Narrow on that
key, not on the widget type: the two arms carry different id fields because they
resolve against different things.

`{ "display": … }` — a read-only widget showing one data point:

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `dataPointId` | string | Which `DataPointDescriptor.id` this widget shows. Its current value is `status.details[dataPointId]`. | Always present. |
| `widgetType` | string **or** object | One of `"statTile"`, `"progressBar"`, `{"metricChart": {"chartType": "pie" \| "bar" \| "line"}}`, `"gauge"`, `"statusDot"`, `"logStream"`. | Always present. |
| `config` | object | Widget-specific extras: `min`/`max` for a gauge or progress bar. Free-form. | Always present; an empty object, never `null`. |

`{ "action": … }` — a control that invokes one action:

| Field | JSON type | Meaning | Nullability |
| --- | --- | --- | --- |
| `actionId` | string | Which `ConnectorAction.id` this widget invokes, as passed to [`POST /connectors/{id}/actions/{actionId}`](#post-connectorsidactionsactionid). | Always present. |
| `widgetType` | string | One of `"button"`, `"toggle"`, `"slider"`, `"textField"`, `"selector"`. | Always present. |
| `config` | object | Widget-specific extras: `min`/`max`/`step` for a slider, `options` for a selector. Free-form. | Always present; an empty object, never `null`. |

**A display `widgetType` is not always a string.** Unit variants serialize as a
bare string; the one variant carrying data serializes as a single-key object:

```json
"statTile"
{ "metricChart": { "chartType": "line" } }
```

The same warning as `ConnectorError` applies — a client that assumes a string
will throw on the chart case. Action widget types are always bare strings.

The layout is a **default, not a mandate**: it is what a user gets when they
place the connector without configuring anything, and it is theirs to edit
afterwards. There are no coordinates here — where a widget sits on a grid is the
dashboard's business, not the connector's.

### `ConnectorError`

Externally tagged, so **each variant is a single-key object** whose key is the
camelCase variant name:

```json
{ "unreachable": { "reason": "connection refused" } }
{ "authFailed": { "reason": "token rejected" } }
{ "invalidAction": { "actionId": "nope" } }
{ "invalidParams": { "actionId": "restart", "reason": "missing `force`" } }
{ "invalidConfig": { "reason": "unknown field `wat`" } }
{ "internal": "unexpected response shape" }
```

| Variant key | Payload | Fields | Meaning |
| --- | --- | --- | --- |
| `unreachable` | object | `reason` (string) | The service could not be contacted: refused, timed out, DNS failure. The fix is at the infrastructure level, not in Loom. |
| `authFailed` | object | `reason` (string) | The service was reached but rejected Loom's credentials. The stored connector configuration needs attention. Separate from `unreachable` because the remedy is completely different. |
| `invalidAction` | object | `actionId` (string) | The requested action id is not one this connector exposes — usually a stale client or an automation naming a removed action. |
| `invalidParams` | object | `actionId` (string), `reason` (string) | The action exists but the parameters do not satisfy its schema. `reason` names the failed constraint so a client can point at the field. |
| `invalidConfig` | object | `reason` (string) | The stored or submitted *connector configuration* is not something this connector can be built from. About the connector itself, not about one action's arguments. |
| `internal` | **string** | — | Anything else broke inside the connector: a bug, an unexpected response shape, a failed parse. Also used for the synthetic `statusError` on an instance that failed to load. |

**`internal` is the one asymmetry.** It is a newtype variant in Rust
(`Internal(String)`), so its value is a **bare string**, not an object with a
field. A client discriminating on the single key must handle that case
separately from the five struct variants. It is called out here because it is
exactly the kind of detail that produces a runtime type error in a client that
assumed uniformity.

This enum covers failures of the *interaction*, never of the managed service. A
service reporting its own bad state is a successful `ConnectorStatus` with
`health: "down"`; a service refusing an action is an `ActionResult` with
`success: false`. Keeping those out of this enum is what lets a client tell
"Loom is misconfigured" from "your server is unhappy".

### `config_schema()` — exposed by `GET /connector-types`

`Connector::config_schema()` returns the JSON Schema for the configuration a
connector needs, and it is served, per registered type, by
[`GET /connector-types`](#get-connector-types). This is what keeps "add a
connector" from requiring a matching UI change in three applications — the form
is derived from the schema, not written per connector.

The registry calls it through a schema function rather than on a live instance,
because the form is needed *before* there is any configuration to build an
instance from.

The schema is for the **client**. The server does not validate against it; it
hands the submitted value to the connector's factory, which is the only thing
that knows what the keys mean and is therefore the only thing that can catch a
value that is the right type and still wrong.

A connector needing no configuration returns an empty schema object rather than
`null`, matching the `paramsSchema` convention.

Connector configuration schemas may mark a top-level string property with the
Loom extension `"x-loom-sensitive": true`. The backend validates the submitted
plaintext first, then stores that field as an authenticated AES-256-GCM blob.
Clients render it as a password input and use `sensitiveFieldsSet` on edit;
schema-marked values never appear in instance responses. See
[ADR 0028](./adr/0028-connector-secrets-at-rest.md).

## Known temporary behavior

Authentication is real. Several things around it are not finished, and the first
one is the important one.

- **The Administrators group is seeded, not maintained.** It receives a global
  grant of every permission registered at migration time. A future migration
  adding a permission key must also decide whether Administrators gets it —
  adding a row to `permissions` alone does not extend the group.
- **`system.settings` is registered and granted but enforced nowhere**, because
  no settings endpoint exists yet.
- **`GET /connector-instances` requires a global view grant** rather than
  filtering the list per grant. See
  [Permission enforcement](#permission-enforcement).
- **Permission changes take up to 15 minutes to take effect** for a signed-in
  user, since checks read the access token's claims. Revoking a grant does not
  end a session already holding it; deactivating or deleting the account does,
  at their next refresh.
- **Logout is per-token.** Revoking one refresh token leaves other devices
  signed in, and cannot recall an access token already issued. There is no "sign
  out everywhere".
- **Expired refresh-token rows are never cleaned up.** They stay in the table
  after expiry. Harmless, but the table only grows.
- **One connector type is registered, and it is the debug fixture.** It contacts
  nothing; its actions simulate their effects and echo their parameters, and its
  data points move on a deterministic oscillation so charts have something to
  draw. It is a permanent development and testing fixture, not scaffolding —
  see the module docs in `crates/core/src/connector/debug.rs`.
- **A connector instance that fails to load is skipped with a warning, not a
  fatal error.** The row survives, is listed with a `statusError`, and can be
  fixed with `PATCH` or removed with `DELETE`.

### Superseded: the `dev-stub-auth` feature

Earlier revisions of this document described a `dev-stub-auth` Cargo feature
that compiled in a login accepting any credentials, a fixed public token
`dev-stub-token`, a single identity `dev-stub-user`, and in-memory setup state
that reset on restart.

**All of it is gone** — the module, the Cargo feature, the CI comment, and the
rule about it in `AGENT_INSTRUCTIONS.md`. `loom-web-backend` now has no Cargo
features at all, and there is no build of it that accepts arbitrary credentials.
The endpoint *shapes* the stub established survive, which was its purpose; the
one breaking change is that login returns `accessToken` + `refreshToken` rather
than a single `token`.

If you find a reference to `dev-stub-auth` anywhere, it is stale and should be
removed.

## Storage

SQLite, at `$LOOM_DATA_DIR/loom.db`, falling back to `./data/loom.db` when the
variable is unset — the directory is created if missing. Uploaded avatars sit
beside it in `$LOOM_DATA_DIR/avatars/`, created at startup for the same reason
the data directory is: a first run should not fail on a directory it could have
made itself.

`users.avatar_path` stores a path **relative to the data directory**
(`avatars/{uuid}.{ext}`), never an absolute one. An absolute path would bake the
deployment's layout into the database, so relocating the data directory — or
mounting the same volume at a different point in a container — would break every
avatar reference at once. Consistent with
[ADR 0004](./adr/0004-zero-config-startup.md): no required environment variable,
and a first run works with no configuration at all.

Migrations live in `crates/web-backend/migrations/` and are embedded into the
binary at compile time, so a released binary carries its own schema history.
They run automatically at startup.

The JWT signing secret is generated from the OS CSPRNG on first boot and stored
in `server_config`. It is never supplied by environment variable, and it is
persisted rather than regenerated because a secret that changed on restart would
invalidate every outstanding access token on every deploy.

The independent connector-config encryption key follows the same persistence
pattern under its own `server_config` key. It is generated from the OS CSPRNG on
first boot and never derived from or shared with the JWT signing secret.

The Cargo package is **`loom-web-backend`**, not `web-backend` — a crate named
`core` would collide with Rust's built-in `core`, so both crates carry the
`loom-` prefix. See [`BUILD.md`](./BUILD.md).
