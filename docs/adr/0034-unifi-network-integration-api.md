# 0034 — UniFi Network Integration API mapping

- Status: accepted
- Date: 2026-09-04

## Context

The UniFi Network connector uses the official local Integration API rather
than undocumented controller endpoints. Its setup and capability checks need a
stable maintenance reference, especially because local consoles commonly use
self-signed certificates and the available operations include disruptive
network writes.

## Verified API facts

These facts were initially checked against the official Network 9.4.17 OpenAPI
document and refreshed against the Network 10.4.57 document at
`https://developer.ui.com/network/v10.4.57/openapi.json`:

- The local API base is
  `https://<console>/proxy/network/integration/v1`. UniFi introduced this API
  and its API-key flow in Network 9.1.105.
- API keys are generated at **Network > Settings > Control Plane >
  Integrations** and sent as `X-API-KEY`. The Integration API documents no
  independent scoped-permission model for one key.
- `GET /v1/sites` is the authentication and configured-site discovery proof.
  Device and client reads are separate paginated calls under the resolved site.
- Sites, devices, and clients use `offset`/`limit` with a default of 25 and a
  maximum of 200. Vouchers use the same mechanism with a default of 100 and a
  maximum of 1000. Every page envelope carries `offset`, `limit`, `count`,
  `totalCount`, and `data`; no cursor or next-page token is involved. Loom
  advances by the number of rows actually present in `data`, making pagination
  robust when a console's `count` disagrees with its payload.
- Some real Network releases omit an adopted online AP from the paginated
  devices collection even though active client rows still carry that device's
  UUID in `uplinkDeviceId`. The official per-device detail route accepts that
  UUID, so Loom best-effort reconciles missing uplinks through
  `GET /sites/{siteId}/devices/{deviceId}`. It does not call undocumented legacy
  APIs and cannot recover an omitted device that currently has no client
  reference.
- A device overview explicitly identifies capabilities through `features`:
  `accessPoint`, `switching`, and `gateway`. Classification therefore must not
  guess from the model string. Device detail exposes radio standard,
  frequency, channel, and channel width. Although the 9.4.17 schema describes
  `frequencyGHz` as a string, real consoles also return it as a JSON number
  (for example `2.4`), so the client accepts both representations. Latest
  statistics optionally expose CPU/memory utilization, uplink RX/TX rates,
  one/five/fifteen-minute load averages, and the last-heartbeat timestamp. AP
  radio statistics expose a retry percentage per radio; Loom publishes the
  highest valid value as the device's worst-band `radioTxRetryPercent`. The
  schema has no AP client
  count and no per-port throughput counters. AP client count can be derived
  honestly from the clients collection's `uplinkDeviceId`; switch port
  throughput cannot be derived and is not published.
- Device restart, port PoE cycling, guest authorization/unauthorization, voucher
  mutations, and pending-device adoption are official write operations. Guest
  unauthorization uses `UNAUTHORIZE_GUEST_ACCESS`. Voucher creation in 10.4.57
  requires only `name` and `timeLimitMinutes`; `authorizedGuestLimit` is
  optional. A connectivity test must describe write availability after
  successful API-key authentication without invoking the operations.
- `GET /sites/{siteId}/wans` is paginated and its 10.4.57 rows expose only `id`
  and `name`. `GET /pending-devices` returns pending hardware independently of
  a site. Adoption uses `POST /sites/{siteId}/devices` with `macAddress` and
  required `ignoreDeviceLimit`; Loom deliberately sends `false` so adoption
  does not bypass the configured limit.
- The 10.4.57 browse-first configuration collections are ACL rules, DNS
  policies, firewall zones/policies, networks, and WLAN broadcasts. Firewall
  policy PATCH accepts only optional `loggingEnabled`. Network deletion has an
  optional `force` flag, which Loom pins to `false`.
- WLAN overviews do not expose `hideName`; that value exists only on the detail
  response. Toggling therefore reads the full detail, removes response-only
  `id` and `metadata`, flips `enabled`, and PUTs the remaining complete mutable
  configuration. The client-wide ten-call semaphore also bounds this detail
  fan-out.
- DNS creation stays limited to the three compact official variants: A records
  (`ipv4Address`, `ttlSeconds`), CNAME records (`targetDomain`, `ttlSeconds`),
  and forward domains (`ipAddress`). AAAA, MX, SRV, and TXT remain visible and
  deletable but are not offered misleadingly through a conditional mega-form.
- Local consoles may present self-signed certificates. Loom's
  `allowInsecureCert` option relaxes certificate verification only for that
  configured connector; the origin remains HTTPS and the API key is never sent
  over plaintext transport.

## Decision

Publish one template-less **Connect via API key** setup-guide variant with no
toggles or capability requirements. Build setup-test connectors without making
a request, then perform site discovery inside `test_connection()` so transport
and authentication failures are returned as `reachable: false` instead of
escaping during construction.

After the site is resolved, probe device and client listing independently.
Report the seven write capabilities as available after authentication with a
note that the connection test did not execute them. This is an all-or-nothing
API-key model, not evidence that any disruptive operation was tested live.

Use one client-owned pagination implementation for every list endpoint. Device
presentation uses generic Lucide category icons only; no product photography is
vendored because no stable licensing-clear source was established. An
unrecognised or absent feature classification deliberately falls back to a
generic network-device icon.

Keep broad network configuration browse-first. Expose only deletion, the
firewall logging PATCH, WLAN enabled-state read-modify-write, firewall-zone
creation, and the three simple DNS creation variants. The shared params form
does not support arrays, so `createZone` accepts comma-separated network IDs at
the connector boundary and converts them to the API's required array.

## Consequences

- The existing generic setup-guide UI renders UniFi instructions and live
  capability results without connector-specific frontend code.
- Read failures remain visible per capability after authentication succeeds.
- Test Connection never restarts or adopts hardware, cycles PoE, changes guest
  access, or creates/revokes a voucher.
- A future UniFi release introducing key scopes must supersede this decision
  and derive write availability from those scopes rather than assuming the
  current all-or-nothing model.
