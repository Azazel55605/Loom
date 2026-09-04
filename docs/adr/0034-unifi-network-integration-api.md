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

These facts were checked against the official Network 9.4.17 OpenAPI document
at `https://developer.ui.com/network/v9.4.17/openapi.json`:

- The local API base is
  `https://<console>/proxy/network/integration/v1`. UniFi introduced this API
  and its API-key flow in Network 9.1.105.
- API keys are generated at **Network > Settings > Control Plane >
  Integrations** and sent as `X-API-KEY`. The Integration API documents no
  independent scoped-permission model for one key.
- `GET /v1/sites` is the authentication and configured-site discovery proof.
  Device and client reads are separate paginated calls under the resolved site.
- Device restart, port PoE cycling, guest authorization, and voucher mutations
  are official write operations. A connectivity test must describe their
  availability after successful API-key authentication without invoking them.
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
Report the five write capabilities as available after authentication with a
note that the connection test did not execute them. This is an all-or-nothing
API-key model, not evidence that any disruptive operation was tested live.

## Consequences

- The existing generic setup-guide UI renders UniFi instructions and live
  capability results without connector-specific frontend code.
- Read failures remain visible per capability after authentication succeeds.
- Test Connection never restarts hardware, cycles PoE, changes guest access, or
  creates/revokes a voucher.
- A future UniFi release introducing key scopes must supersede this decision
  and derive write availability from those scopes rather than assuming the
  current all-or-nothing model.
