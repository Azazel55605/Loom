# ADR 0042: Music Assistant WebSocket transport

## Status

Accepted.

## Context

Music Assistant is Loom's first service intended to implement both sides of the
shared media model from ADR 0041. Its transport must be proven independently
before player, library, queue, or action mapping can obscure wire-level errors.

### Verified current API facts

These facts were checked on 2026-09-16 against Music Assistant's current
[official API overview](https://www.music-assistant.io/api/), the server's
[WebSocket implementation](https://github.com/music-assistant/server/blob/dev/music_assistant/controllers/webserver/websocket_client.py), and the official
[Python client](https://github.com/music-assistant/client):

- The local web server defaults to port `8095`; its WebSocket endpoint is
  `/ws`. Local installations normally use plaintext HTTP/WebSocket. TLS is
  supported when MA is placed behind an HTTPS reverse proxy, so the transport
  maps bare/HTTP hosts to `ws://` and HTTPS/WSS hosts to `wss://`.
- This is not JSON-RPC 2.0. Commands are
  `{"message_id":"...","command":"...","args":{...}}`. Success responses
  carry the same `message_id`, `result`, and a `partial` flag; failures carry
  `message_id`, `error_code`, and `details`. Event frames use an `event` field
  and are not command responses.
- The server sends `ServerInfoMessage` immediately after WebSocket setup. It
  includes `server_id`, `server_version`, `schema_version`, and
  `min_supported_schema_version`.
- Current schema 28 and later requires an access token. Authentication is the
  first command: `auth` with `args: {"token":"..."}`. Current servers return
  `{"authenticated":true,"user":...}`; the transport also accepts the older
  bare `true` success shape. The official client permits tokenless operation only for older
  pre-schema-28 servers; trusted Home Assistant ingress is a separate transport
  context and is not anonymous public API access. Loom keeps `token` optional
  solely for that legacy compatibility and reports a clear authentication
  failure when a current server receives none.
- There is no dedicated current ping or server-info command. The unsolicited
  server-info frame is the socket identity proof. `players/all` is a confirmed,
  side-effect-free authenticated command returning an array, so the isolated
  live test uses it to prove request correlation and the authenticated command
  path.

## Decision

Create `loom-connector-music-assistant` with an independently-tested transport
layer, then map that layer onto `Connector`, `MediaSourceCapable`, and
`MediaTargetCapable` without changing the wire client.

Use `tokio-tungstenite`, matching the proven TrueNAS client architecture. A
background task owns the socket and correlates UUID message ids through a
pending-request map. It also accumulates MA's chunked `partial` array results.
Calls time out after 30 seconds; disconnects resolve every in-flight call as
`Disconnected` rather than leaving waiters hanging.

After an unexpected disconnect the task reconnects after 1, 2, 4, 8, 16, then
at most 30 seconds between attempts. It reads the new server-info frame and
re-authenticates before returning to `Connected`. Calls attempted while the
transport is reconnecting fail immediately rather than building an unbounded
offline queue.

The connector maps `players/all` to player sub-targets and the host players
resource. Library discovery uses `music/browse`, `music/search`, and
`music/item_by_uri`. Playback state comes from
`player_queues/get_active_queue` plus `player_queues/items`; queue transport,
seek, shuffle, repeat, and insertion use the corresponding `player_queues/*`
commands, while player volume uses `players/cmd/volume_set`.

Music Assistant's `player_queues/play_media` consumes MA's canonical media URI,
not a raw stream fetched by Loom. The source adapter therefore deliberately
carries that URI in `ResolvedPlayable.stream_url`; only the matching MA target
adapter interprets it. This is the source-specific token case anticipated by
ADR 0041, not a claim that an MA URI is an HTTP URL.

## Consequences

- MA protocol drift is isolated from Loom's connector and media contracts.
- Both ordinary local `ws://` deployments and TLS reverse proxies are usable.
- Current servers fail clearly when a token is missing or rejected; historical
  tokenless support is not misrepresented as a current configuration mode.
- Player targets immediately render the shared MediaPlayer widget, including
  the host library browser, without connector-specific frontend code.
- Read-only live tests are opt-in through environment variables. The audible
  play/pause/resume/seek/skip/stop sequence is additionally ignored by default
  and requires an explicit acknowledgement and selected player/item.
