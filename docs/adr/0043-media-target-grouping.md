# ADR 0043: Native media-target grouping with server-side fan-out fallback

- Status: accepted
- Date: 2026-09-16

## Context

ADR 0041 separates media sources from playback targets, but its target contract
originally described only one player at a time. Music Assistant can synchronize
compatible players natively. Other connectors may have no grouping support, and
even Music Assistant cannot group every pair of players or protocols.

A client-side fallback would make Web, Desktop, and Mobile each responsible for
membership, ordering, partial failures, and permission-sensitive dispatch. It
would also let clients disagree about which targets currently form a group.
Grouping and fallback therefore belong at the existing backend media-control
boundary.

## Decision

### The media target contract exposes an optional grouping facet

`MediaTargetCapable` adds `supports_grouping`, `join_group`, and `leave_group`.
They default to unsupported, preserving every existing target implementation.
`join_group` is called on the intended primary target and receives connector-
defined target ids for the members to add.

An unsupported or incompatible combination returns `MediaError::Unsupported`.
That result is a normal capability outcome, not a panic, silent no-op, or
generic playback failure. It gives the backend an explicit point at which to
choose its fallback.

### Music Assistant uses dynamic groups

Music Assistant implements joining through `players/cmd/set_members`, passing
the primary as `target_player`, the requested members as
`player_ids_to_add`, and null for `player_ids_to_remove`. Leaving uses
`players/cmd/ungroup`.

Current player responses advertise `set_members` in `supported_features` and
list compatible player ids in `can_group_with`. The connector checks both for a
specific early explanation, while the command response remains authoritative.
MA's unsupported-feature and player-command compatibility errors retain their
real message and become `MediaError::Unsupported` on this grouping path.

Loom does not call `players/create_group_player`. Permanent group players would
clutter MA's player list with Loom-created entities after a temporary dashboard
interaction. Dynamic grouping has the desired session-scoped semantics.

### The backend owns native-versus-fan-out state

The join endpoint first attempts native grouping. Success records a native
group. `MediaError::Unsupported` records the same primary and members as a
fan-out group and still returns success, explicitly naming the selected mode.

Fan-out play, transport, seek, and volume writes run primary-first and then
member-by-member. Dispatch is sequential so one user gesture cannot create an
unbounded burst against a connector. Every member is attempted and failures are
aggregated. Reads continue to use the primary as the displayed source of truth.
Native groups receive only one command because their service owns synchronized
dispatch.

The registry is in-memory, like other transient runtime coordination. It is
cleared when an instance is updated or deleted and is lost on backend restart.
This avoids introducing durable user configuration or a migration for state
whose authoritative native half lives in another service. A native service may
retain its dynamic group through a Loom restart; Loom deliberately does not
claim or fan out that group until another join request records it again.

All join, leave, and fanned-out controls still pass through the existing
resource-scoped `connectors.control` check and connector action log.

## Consequences

- Connectors gain grouping without changing source discovery or ordinary
  single-target playback.
- Compatible Music Assistant players stay genuinely synchronized by MA.
- Incompatible players still receive equivalent controls, but fan-out cannot
  promise sample-accurate synchronization.
- Web, Desktop, and Mobile can consume one grouping API instead of reproducing
  dispatch policy.
- Process-local membership is intentionally simpler than durable recovery; a
  future requirement for restart reconstruction would need explicit upstream
  reconciliation rather than silently changing this registry into stored
  configuration.

