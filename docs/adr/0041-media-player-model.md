# ADR 0041: Shared media source and playback-target model

- Status: accepted
- Date: 2026-09-16

## Context

Loom needs a media-player foundation that can eventually cover audio today and
video later without building separate transport, queue, now-playing, and source
browsing systems for each render surface. The first expected real integration,
Music Assistant, is unusual in a useful way: it can supply media and orchestrate
playback targets. Future integrations need not have both halves. A bespoke
speaker may only accept playback commands, while a file-manager-style connector
may only expose media to play elsewhere.

Rust trait objects add one architectural constraint. The existing
`dyn Connector` registry cannot be downcast to arbitrary optional traits, and a
base trait cannot name types from a crate that already depends on it without
forming a Cargo dependency cycle. The media boundary therefore needs both an
explicit discovery mechanism and a deliberate dependency direction.

This work also exposes two gaps in Loom's current generic UI primitives. There
is no shared image-rendering primitive for artwork, and no browsable list/grid
primitive suitable for a media library. Building media-specific substitutes
would undermine the generic widget system, but solving those primitives is a
separate phase.

## Decision

### One unified media model

`crates/media` defines one `MediaKind`: `Audio` and the reserved
`Video { live: bool }`. Audio and video share source resolution, transport,
queue, volume, repeat, and now-playing state; only their eventual render surface
needs to differ. Separate audio and video widget systems would duplicate the
important plumbing and make mixed queues needlessly awkward.

The video `live` bit is present before any video connector exists because a
UniFi Protect camera feed and a video file have fundamentally different
semantics. A live feed has no duration, seek position, or meaningful queue
position; a file-like video does. `MediaItem::duration` remains the immediate
source of truth for whether an individual item has a finite duration. It is
`None` for live video and also for live audio such as radio, so callers do not
need video-only special cases.

Artwork is an `artwork_ref`, not embedded bytes. It may be a URL or a
source-specific token and is resolved by the eventual display layer. The
contract does not fetch images.

### Source and target are independent capabilities

`MediaSourceCapable` owns `browse`, `search`, and `resolve`.
`MediaTargetCapable` owns playback state, transport, seek, volume, skip, and
queue operations. A connector may implement either trait, both, or neither:

- Music Assistant implements both as a source hub and target orchestrator;
- a future bespoke speaker implements only `MediaTargetCapable`;
- a future file-manager-style connector implements only
  `MediaSourceCapable`.

`resolve()` is async and fallible because resolution may require real work, not
just a map lookup. A file-manager source may need to serve a file through its
own lightweight HTTP endpoint before it can return a playable URL.

Targets return `MediaError::Unsupported` for operations that do not apply,
especially seeking or queue operations on a live-only target. This is distinct
from a playback failure: the request is structurally inapplicable, not a failed
attempt at a supported operation.

### Core discovers optional facets explicitly

`Connector` gains `as_media_source()` and `as_media_target()`, both defaulting
to `None`. Implementations opt in by returning the appropriate trait object.
This avoids trait-object downcasting and follows the existing Connector pattern
where optional sub-target and resource-kind capabilities have harmless empty
defaults. Existing connectors change neither behavior nor implementation.

The crate dependency points from `loom-core` to `loom-media`. `loom-media` is a
transport-neutral leaf containing the traits, shared values, and a structured,
serializable `MediaError` that follows `ConnectorError` conventions. It does
**not** depend on Core. The initially considered `media -> core` direction is
impossible together with discovery methods on `core::Connector`, because Core
would then also need `core -> media`. Re-exporting `loom-media` as
`loom_core::media` preserves one convenient public path for connector authors
without reversing the actual dependency graph.

This differs from service connector crates, which depend on Core: `loom-media`
is a contract used by Core itself, not an integration that talks to a service.
It contains no network client, listener, runtime owner, permission decision, or
service implementation.

### Scope of this phase

This phase is traits and types only. It intentionally adds no media widget, no
image primitive, no browsable list/grid primitive, no DebugConnector fixture,
and no real media connector. The two missing generic primitives are explicitly
deferred to the next phase; fixtures and integrations follow after the shared
rendering surface exists.

## Consequences

- Every current connector remains source-compatible because both discovery
  methods default to `None`.
- Future source-only, target-only, and combined connectors compose through the
  same model without pretending all services have both roles.
- Audio and future video share transport and queue plumbing while retaining the
  live/file distinction required for honest controls.
- Media values and errors can cross future backend API boundaries without
  coupling the contract crate to Web/backend or to a particular player.
- The frontend still cannot render this model. Image rendering and browsable
  list/grid primitives must be designed before a media widget or fixture is
  added.
