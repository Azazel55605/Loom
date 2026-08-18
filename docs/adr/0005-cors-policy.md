# 0005 — CORS policy: permissive by default, tightened with auth

- Status: accepted
- Date: 2026-08-18

## Context

Every Loom client is a different origin from the backend. The web frontend
deploys independently and is served on its own port and usually its own host;
the Tauri desktop and mobile clients load from `tauri://localhost` and
`http://tauri.localhost`. A browser therefore refuses to let any of them read an
API response unless the backend says so with CORS headers.

Without them the failure is opaque: the request reaches the server and the
server answers correctly, but the browser discards the response and reports only
"NetworkError when attempting to fetch resource". `curl` succeeds throughout,
because no same-origin policy applies outside a browser, which makes the symptom
look like a server or network fault when it is neither.

Restricting the allowed origins by configuration was the alternative. It fails
the zero-config requirement in [0004](./0004-zero-config-startup.md): a homelab
deployment cannot know in advance what host or port its frontend will be served
from, so a required origin list would mean nobody's first `docker compose up`
works.

## Decision

The API sends CORS headers, allowing **any origin by default**, with any method
and any header.

Operators who want to pin the list may set `LOOM_CORS_ALLOWED_ORIGINS` to a
comma-separated list of origins. It is optional and unset by default, consistent
with 0004: every runtime setting has a working default.

This is safe **today** and only today. The API is unauthenticated, carries no
cookies, and returns nothing that is not already public to anyone who can reach
the port. A permissive policy grants a hostile page no more than it could get by
requesting the port directly.

## Consequences

**This must be revisited before the auth work in
[0003](./0003-auth-model-vpn-vs-external.md) ships.** Once the API accepts
credentials, allowing any origin lets any website a logged-in user visits make
authenticated requests on their behalf — the classic CSRF shape. Browsers reject
`Allow-Origin: *` together with `Allow-Credentials: true` outright, so the
combination fails loudly rather than silently, but the policy still has to
become an explicit allowlist, with the Tauri origins named. Treat this as a
prerequisite of the auth work, not a follow-up to it.

The permissive default also means the browser cannot be relied on to keep
anything away from the API. That is already the intent: 0003 puts enforcement at
the API layer regardless of network path, and CORS is a browser convention, not
an access control mechanism.

A same-origin deployment — reverse-proxying the API under the frontend's origin,
e.g. `/api` — would sidestep CORS for the web client entirely and is worth
considering later. It does not remove the need for this policy, because the
desktop and mobile clients are never same-origin.
