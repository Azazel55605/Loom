# Loom API contract

This describes the API surface that exists **today**: real multi-user
authentication backed by SQLite, and a single mock-backed connector.
Authentication is real; **authorization is not yet enforced** — see
[Known temporary behavior](#known-temporary-behavior) before assuming this
instance is access-controlled.

Connector shapes will still change as the connector contract settles
([ADR 0002](./adr/0002-connector-contract-tbd.md),
[ADR 0007](./adr/0007-in-process-connector-trait.md)). The *shape* — field
names, JSON types, nesting — is meant to survive that. Treat a change to a field
name or type here as a breaking change to three clients, and make it
deliberately.

The auth design and its trade-offs are recorded in
[ADR 0008](./adr/0008-auth-model.md).

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

CORS permits local web development plus Tauri's known webview origins
(`tauri://localhost`, `https://tauri.localhost`, and
`http://tauri.localhost`); additional browser origins may be appended through
`LOOM_CORS_ALLOWED_ORIGINS`. The explicit list is sufficient because auth uses
Bearer headers rather than ambient cookies. See
[ADR 0010](./adr/0010-desktop-secure-storage-and-network-config.md).

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

| Status | Meaning |
| --- | --- |
| 200 | Credentials accepted. A session now exists. |
| 401 | Credentials rejected, for any reason. |
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
other sessions alone. There is no "sign out everywhere" yet.

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

Registered keys today: `connectors.view`, `connectors.control`, `users.manage`,
`groups.manage`, `system.settings`. The set is defined by the `permissions`
table and extended only by migration.

**Clients may use this to hide controls the user cannot operate. That is a
convenience, never a control.** The server decides what is permitted; a client
that ignores this array learns nothing it could not have learned by trying. See
[Permission enforcement](#permission-enforcement) for what is actually checked,
and where.

### Permission enforcement

Every route except `/health`, `/setup/status`, `/setup`, `/auth/login`,
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
| `GET /connectors` | `connectors.view` | global |
| `POST /connectors/{id}/actions/{actionId}` | `connectors.control` | `connector` / `{id}` |
| `GET /users`, `POST /users`, `PATCH /users/{id}`, `DELETE /users/{id}` | `users.manage` | global |
| `GET /groups`, `POST /groups`, `PATCH /groups/{id}`, `DELETE /groups/{id}` | `groups.manage` | global |
| `GET /permissions` | `groups.manage` | global |
| `GET /account`, `PATCH /account`, `POST /account/password`, `POST /account/avatar`, `DELETE /account/avatar` | none — token only | n/a, the subject is the token's `sub` |

`GET /connectors` requires a **global** `connectors.view`, so a user holding
only a connector-scoped view grant is refused rather than shown a filtered list.
Filtering the response to what the caller may see would be friendlier and is the
natural next step — it is not built because nothing issues scoped view grants
yet, and a filter with no way to create the case it filters cannot be tested
against reality.

A 403 on the action route is returned **before** the connector id is looked up,
so an unauthorized caller gets the same response whether or not the id exists.
Otherwise the endpoint would report 404 for unknown ids and 403 for real ones,
which is a way to enumerate what is configured.

### Safeguards

Two rules protect an instance from being administered into a state nobody can
administer it out of. Both return **409 Conflict** and change nothing.

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

Neither user safeguard is a security control. An administrator can still grant
someone else the Administrators group and have them do it. They guard against
mistakes, which is the failure mode that actually happens.

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

### `GET /connectors`

**No authorization is checked** — see
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

**No authorization is checked.** Executes one action on one connector.

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
  "createdAt": "2026-08-19T17:04:11.882401Z",
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
*refresh* token's seven days, not the access token's fifteen minutes. This is an
accepted trade-off rather than a claim that the window is small: the common case
is a user rotating their own password on their own machine, which gains nothing
from being signed out, while the mechanism that would genuinely help is
on-demand revocation of a user's refresh tokens — which is also what "sign out
my other devices" and "this account is compromised" need. It belongs with that
feature rather than bolted onto this route. **Revisit when session revocation
lands.**

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
    "createdAt": "2026-08-19T17:04:11.882401Z",
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

### `DELETE /users/{id}`

Requires `users.manage`. **Response 204**, no body.

A hard delete: the row goes, and `ON DELETE CASCADE` takes the user's group
memberships and refresh tokens with it — which also ends their sessions, since a
refresh token that no longer exists cannot be redeemed.

Hard rather than soft because nothing yet references a user row historically —
no audit log, no "created by", no ownership. **That should be revisited the
moment one exists**, at which point deleting a user either orphans history or
silently rewrites it, and deactivation becomes the right default. Deactivation
is already available through `PATCH`.

| Status | Meaning |
| --- | --- |
| 204 | Deleted. |
| 404 | No such user. |
| 409 | [A safeguard refused it](#safeguards). |

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

Authentication is real. Several things around it are not finished, and the first
one is the important one.

- **The Administrators group is seeded, not maintained.** It receives a global
  grant of every permission registered at migration time. A future migration
  adding a permission key must also decide whether Administrators gets it —
  adding a row to `permissions` alone does not extend the group.
- **`system.settings` is registered and granted but enforced nowhere**, because
  no settings endpoint exists yet.
- **`GET /connectors` requires a global view grant** rather than filtering the
  list per grant. See [Permission enforcement](#permission-enforcement).
- **Permission changes take up to 15 minutes to take effect** for a signed-in
  user, since checks read the access token's claims. Revoking a grant does not
  end a session already holding it; deactivating or deleting the account does,
  at their next refresh.
- **Logout is per-token.** Revoking one refresh token leaves other devices
  signed in, and cannot recall an access token already issued. There is no "sign
  out everywhere".
- **Expired refresh-token rows are never cleaned up.** They stay in the table
  after expiry. Harmless, but the table only grows.
- **One connector is registered, and it is `MockConnector`.** It contacts
  nothing; `restart` and `ping` simulate their effects and echo their
  parameters. It is a permanent test fixture, not scaffolding — see the module
  docs in `crates/core/src/connector/mock.rs`.
- **`config_schema()` is not exposed by any endpoint.** See
  [above](#config_schema--present-on-the-trait-exposed-by-nothing).

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

The Cargo package is **`loom-web-backend`**, not `web-backend` — a crate named
`core` would collide with Rust's built-in `core`, so both crates carry the
`loom-` prefix. See [`BUILD.md`](./BUILD.md).
