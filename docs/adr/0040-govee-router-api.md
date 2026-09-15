# ADR 0040: Govee Router API connector boundary

## Context

Govee's current cloud API calls itself the v2 Router API while its published
paths remain under `/router/api/v1`. Devices declare their supported behavior
as capability objects, so assuming controls from a model number would make the
connector brittle and could offer operations a particular device cannot run.

## Decision

Loom uses the fixed HTTPS origin `openapi.api.govee.com` and sends the API key
unchanged in `Govee-API-Key`. The key is marked `x-loom-sensitive`, so the
backend's existing field-level encryption and redact-on-read behavior apply.

The connector treats `user/devices` as the capability authority. It publishes
power, brightness, packed-RGB colour, and colour-temperature readings/actions
only when their exact capability type and instance are present, and uses the
device-declared range for bounded values. Packed RGB is converted to the shared
`#RRGGBB` UI boundary. Dynamic-scene option values remain opaque JSON and are
sent back unchanged.

Control calls receive generated UUID request ids and expose those ids in the
action result for audit correlation. Calls are locally paced within the
published budgets: 30 device-list calls per account per minute, 720 controls
per account per minute, 120 controls per device per minute, and 30 state or
scene calls per device per minute. A server HTTP/body rate-limit response is
still surfaced explicitly.

Govee says a state response associated with an offline device can be historic.
Loom therefore treats a successful response as reachable and does not use the
online flag to block a control request; a failed state request is reported as
that target being Down and degrades the account aggregate.

## Consequences

- New Govee device capabilities appear without a model allowlist once mapped
  to a shared Loom data/control primitive.
- Polling a large account is intentionally sequential and can take time because
  respecting the per-device state budget matters more than low-latency bulk
  refreshes.
- The account devices table omits live power state because `user/devices` does
  not contain it; filling that column would require an extra state call per row.
- Scenes are fetched only when browsed and resolved again when applied, avoiding
  stale identifiers while preserving the service's opaque value format.
