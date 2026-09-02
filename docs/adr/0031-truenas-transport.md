# ADR 0031: TrueNAS JSON-RPC transport

## Status

Accepted.

## Context

Loom needs to communicate with TrueNAS, whose current supported remote API is
JSON-RPC 2.0 over WebSocket. Transport and authentication are the highest-risk
part of that integration: the API recently replaced REST, its authentication
surface is evolving, and homelab systems commonly use self-signed certificates.
Building data points, sub-targets, and actions before proving this boundary
would mix protocol uncertainty with connector behavior.

The prompt proposed ADR number 0023, but that number already belongs to Docker
update management. ADRs are append-only, so this decision uses the next free
number rather than overwriting history.

### Verified API facts

These facts were checked against the current stable TrueNAS 25.10.5
documentation on 2026-09-02:

- The JSON-RPC endpoint is `/api/current`. The official
  [TrueNAS API client](https://github.com/truenas/api_client) identifies that URI
  as the JSON-RPC transport and `/websocket` as the legacy DDP transport.
- Requests use JSON-RPC 2.0 objects with `jsonrpc`, a string or numeric `id`,
  `method`, and array `params`. Responses echo the `id` and contain either
  `result` or an `error` object. The documented error object contains numeric
  `code`, nullable `message`, and structured `data`; custom server codes include
  `-32000` and `-32001`. Batch requests are not supported. See the
  [25.10 JSON-RPC protocol reference](https://api.truenas.com/v25.10/jsonrpc.html).
- `core.ping` takes `[]`, requires no authentication, and returns the string
  `"pong"`. It is side-effect-free and therefore the connectivity proof. See
  the [25.10 core.ping reference](https://api.truenas.com/v25.10/api_methods_core.ping.html).
- The preferred API-key call is `auth.login_ex` with one object in `params`:
  `{"mechanism":"API_KEY_PLAIN","username":"...","api_key":"..."}`.
  Success is an object whose `response_type` is `SUCCESS`. See the
  [25.10 auth.login_ex reference](https://api.truenas.com/v25.10/api_methods_auth.login_ex.html).
- `auth.login_with_api_key` with `params: [api_key]` remains supported by
  stable 25.10 and returns a boolean, but is deprecated and scheduled for
  removal in version 27. The
  [method reference](https://api.truenas.com/v25.10/api_methods_auth.login_with_api_key.html)
  directs clients to `auth.login_ex`. The key-only `connect` signature required
  for this transport can only express this compatibility method; a separate
  username-aware constructor exposes the preferred stable method now.
- The official [API access guidance](https://www.truenas.com/docs/scale/api/)
  states that API keys are password-equivalent, require SSL/TLS transport, and
  are automatically revoked when submitted by insecure HTTP transport. This is
  authoritative documentation, not only a community report.

The future TrueNAS 26/27 SCRAM API-key mechanism is deliberately not guessed at
here. It requires a username and a multi-step exchange and should be added from
the stable documentation for the version Loom chooses to support.

## Decision

Create `loom-connector-truenas` as a transport-only crate. It depends on Core so
the following connector implementation can build on the same crate boundary,
but implements no `Connector` behavior in this phase.

Use `tokio-tungstenite` for asynchronous WebSocket framing and `rustls` with
webpki roots for TLS. This matches the workspace's existing Rust-native TLS
choice and avoids a system OpenSSL dependency.

The client accepts a host or `wss://` authority and always constructs
`wss://<authority>/api/current`. Inputs naming `ws://`, HTTP schemes, paths,
queries, or fragments are rejected before any connection attempt. The
`allow_insecure_cert` option bypasses certificate-chain and hostname validation
only; encryption and handshake-signature verification remain active, and no
plaintext WebSocket path exists.

Each call receives a UUID string id and a oneshot waiter in a shared pending map.
One background task owns the socket, serializes outgoing calls, reads responses,
and dispatches each result or server error to its correlated waiter. Calls time
out after 30 seconds. A lost connection immediately resolves every pending call
as `Disconnected` rather than leaving it suspended.

After an unexpected close or I/O error, state moves through `Disconnected` to
`Reconnecting`. Reconnection waits 1, 2, 4, 8, 16, then at most 30 seconds
between attempts and continues at the cap. Every new WebSocket is authenticated
before state returns to `Connected`; calls made while reconnecting fail clearly
instead of accumulating an unbounded offline queue.

## Consequences

- The transport can be tested and reviewed independently of TrueNAS domain
  modeling.
- Publicly trusted and explicitly accepted self-signed certificates work
  without ever exposing an API key over plaintext transport.
- Concurrent calls cannot consume one another's responses, and disconnects do
  not strand futures.
- The key-only constructor has a documented TrueNAS 27 horizon. The next phase
  should carry a username in connector configuration and prefer the
  username-aware constructor; SCRAM remains future work tied to stable-version
  documentation.
- Data points, sub-targets, setup guides, actions, backend registration, and all
  `Connector` trait implementation are deliberately out of scope.
