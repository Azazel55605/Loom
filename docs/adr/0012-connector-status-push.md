# 0012 — Poll connector status centrally and push cache changes over WebSocket

- Status: accepted
- Date: 2026-08-21

## Context

[0011](./0011-connector-instance-registry.md) made connector instances durable
rows backed by process-lifetime connector objects, but HTTP reads still called
`Connector::status()` directly. A slow or unreachable upstream therefore made
the connector list slow, multiple clients duplicated the same work, and the web
frontend had to refetch the whole list on a timer just to notice one status
change.

The backend already owns the live connector objects and is the only process
allowed to make authorization decisions. It is therefore also the natural place
to decide status freshness once and share the result with every client.

Browsers add one constraint: the WebSocket constructor cannot set an
`Authorization` header. Authentication must use something the handshake can
carry without introducing cookie authentication and its ambient-credential
risks.

## Decision

### Poll once, cache once

`ConnectorRuntime` owns a status cache alongside its live-instance map. The
backend performs an initial poll before serving and then polls every connector
at a named five-second interval. It stores either the successful
`ConnectorStatus` or the structured `ConnectorError`; one connector's failure
is data and cannot terminate the polling task.

List and detail handlers read this cache. They do not contact an upstream
service. Create and update seed the replacement connector's cache immediately,
and delete removes its cached entry.

When a snapshot differs from the cached value, the runtime publishes a change
through a bounded Tokio broadcast channel. A lagging consumer skips missed
messages and catches the next periodic snapshot rather than exerting backpressure
on connector polling.

### Push only explicitly subscribed instances

Axum's built-in `ws` feature serves `GET /ws`; no WebSocket crate is added as a
direct backend dependency. The normal browser proxy exposes the same route as
`/api/ws`.

Each connection owns a set of instance UUIDs, changed by `subscribe` and
`unsubscribe` messages. Runtime updates are sent only when their instance id is
in that set. Closing the connection drops the set and broadcast receiver, so
there is no connection registry to clean up separately.

The handshake requires a valid short-lived JWT access token and a global
`connectors.view` grant. Because a browser cannot set the Bearer header, the
token is percent-encoded in the `token` query parameter. Refresh tokens are
never accepted. This can expose an access token to request-target logging, so
deployments must use TLS and should suppress query strings in proxy logs. The
token's fifteen-minute lifetime bounds exposure, and the client reconnects when
refresh rotates it.

The shared UI package owns a single reconnecting client. It applies bounded
exponential backoff, re-subscribes all active instance ids after reconnect, and
replaces the connection when the token store publishes a new access token.
React Query still owns initial reads, details, and mutations; WebSocket events
only replace status fields in its existing caches.

## Consequences

- Status freshness and upstream load are independent of how many clients are
  open. HTTP connector reads are bounded by database/cache work instead of
  upstream response time.
- A client may display a snapshot up to one poll interval old. `lastChecked`
  remains part of the status so that age is visible and honest.
- WebSocket delivery is best effort. The next poll repairs a missed broadcast,
  while an initial HTTP read prevents a newly connected client from waiting for
  the next change.
- Query-token authentication is a narrow browser compatibility exception, not a
  general API authentication mechanism. Any future transport that can set
  headers should continue using `Authorization: Bearer`.
- Desktop and mobile webviews allow `ws:` and `wss:` in their CSP because they
  consume the same shared connector view. Platform-specific TLS behaviour
  remains the responsibility of their transports.
