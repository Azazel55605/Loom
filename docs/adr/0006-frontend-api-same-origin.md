# 0006 — The web frontend reaches the API same-origin

- Status: accepted
- Date: 2026-08-18

## Context

The frontend originally read its API base from `VITE_API_URL`, compiled into the
bundle at build time. That string is executed by the *browser*, so its default of
`http://localhost:8080` meant "the machine running the browser".

This works only when the browser and the backend are the same machine. It fails
the moment the stack runs on a server: the browser resolves `localhost` to the
user's own laptop, the request never reaches the server at all, and the failure
surfaces as an opaque `NetworkError` that looks like a backend or CORS fault.

The deeper problem is distribution. CI has to bake *some* value into the
published image, and no fixed value can be right for arbitrary users — so
`docker-compose.yml`, the published-image path, could never work for anyone.
Requiring each deployment to set the URL and rebuild also conflicts with
[0004](./0004-zero-config-startup.md).

## Decision

The frontend calls **`/api`, relative to its own origin**. The nginx that serves
the SPA proxies `/api/*` to the backend, stripping the prefix.

The proxy target is a runtime setting, `LOOM_BACKEND_ORIGIN`, defaulting to the
Compose service name `http://web-backend:8080`, so the stack needs no
configuration at all. `VITE_API_URL` survives as a build-time escape hatch for
deployments without the proxy; when set it makes requests cross-origin, subject
to the CORS policy in [0005](./0005-cors-policy.md).

The Vite dev server proxies `/api` the same way, so development exercises the
same path as production rather than a special case.

## Consequences

One published image now works for every deployment, at any hostname, with no
rebuild — which is what makes `docker-compose.yml` viable at all.

The web client no longer makes cross-origin requests, so CORS is irrelevant to
it. **0005 is still required**: the desktop and mobile clients talk to the
backend directly and are never same-origin.

The frontend container must be able to reach the backend, which it does on the
Compose network. Deployments that split the two across hosts must set
`LOOM_BACKEND_ORIGIN` accordingly.

Two sharp edges are worth recording, because both cost real debugging time:

- **`proxy_pass` with a variable does not strip the location prefix.** Using a
  variable (needed so nginx starts before the backend is up) disables the usual
  trailing-slash rewrite, so `/api/health` arrives at the backend unchanged. The
  config strips the prefix with an explicit `rewrite`.
- **An unset Docker `ARG` becomes an empty string, not undefined.** `??` does
  not treat `""` as absent, so a declared-but-empty `VITE_API_URL` silently won
  and produced an empty API base. The app treats blank as unset.
