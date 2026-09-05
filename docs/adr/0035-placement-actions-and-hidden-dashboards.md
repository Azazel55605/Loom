# 0035 — Placement actions, static tiles, and hidden dashboards

Status: accepted

## Context

Three requests arrived together and turned out to be one shape of problem.

1. **A tile should be able to do something when you click it.** Go to another
   dashboard, or fire one pre-configured connector action, without opening a
   widget and choosing parameters.
2. **A dashboard should be able to stay out of the sidebar.** A dashboard that
   exists to be *arrived at* — the destination of a tile — clutters a list it
   was never meant to be browsed from.
3. **A resource kind should be placeable like anything else** — including as a
   single placement large enough to fill a whole dashboard, which is how the
   Docker image list and the UniFi client list actually want to be read.

The obvious design for the first is a new kind of placement — a "button
placement" beside the connector placement. The planning session that preceded
this work rejected that direction and settled on composition instead, and this
ADR records the reasoning so it is not re-litigated the next time a tile needs a
new capability.

## Decision

### Click behavior is composed onto the existing placement

A new nullable `placement_action` column on `dashboard_placements`, carrying a
JSON-encoded `PlacementAction`:

```
{ type: "navigate", targetDashboardId }
| { type: "connectorAction", connectorInstanceId, targetId, actionId, params }
```

Not a new placement type, and not a new table. A placement already carries every
rule that a clickable tile also needs — geometry, grouping, the dashboard ACL,
the retroactive group membership that makes grouping lossless — and a second
tile entity would have had to restate all of them and would then have drifted
from them one release later.

Composition also buys the case a separate type forbids outright: a tile that
**both** shows a connector's live state **and** navigates somewhere when
clicked. Under a "button placement" model that tile is inexpressible, because it
is two placements occupying one square.

`PlacementAction` lives in `crates/web-backend`, not in `crates/core`. It is a
dashboard-system concept: nothing in the `Connector` trait knows dashboards
exist, and a connector must never be able to name one. The `connectorAction`
variant carries its own `connectorInstanceId` rather than borrowing the
placement's, so a tile may display one connector and act on another.

`connectorAction` clicks dispatch through `invoke_action` — the same function
the direct action endpoint and the update scheduler already share. Same
`connectors.control` resource-scoped check, same audit row, same
pending-operation overlay, same post-action refresh. The click endpoint selects
pre-configured arguments and calls the one path; it is not a parallel
implementation, because a second dispatch path is a second place for an
invocation to go unrecorded.

### `connector_instance_id` becomes nullable

A tile whose whole purpose is to navigate has no data to show and therefore no
connector to read it from. `NOT NULL` was an accurate statement about the model
until click behavior existed and is not one now.

The alternative — requiring a connector on every placement and letting a
"button" tile point at an arbitrary one it does not display — would have stored
a lie in the column that every join and every cascade reads.

The consequence is a validation rule, not a hole: a placement with no
`connectorInstanceId` **must** carry a `placementAction`, enforced at create and
at update. Widget-binding validation is skipped entirely for such a placement
rather than run against an absent connector, because there is nothing to bind. A
tile that neither shows anything nor does anything is a blank rectangle, and the
API refuses to store one.

SQLite cannot relax a column's nullability in place, so the change is the
standard table rebuild, reproducing every other column, constraint, foreign key
and index exactly.

### Two independent permission checks for `navigate`

A `navigate` action is checked **twice, against two different people, at two
different times**, and neither check substitutes for the other.

- **At save time**, against the editor writing the tile. A sanity check: does
  this dashboard exist, and can the person configuring the tile see it? Its
  purpose is to catch a typo in the editor rather than on a stranger's click.
- **At click time**, against whoever is clicking, evaluated fresh on every
  click. This is the check that governs.

Caching the first verdict would have been simpler and would have been wrong.
Shares are revoked, group memberships change, dashboards are deleted, and the
person clicking a tile is usually not the person who placed it. The creator
still being able to follow a link a viewer can no longer follow is the correct
behavior of two independent checks, not a bug to reconcile.

The two checks also answer failure differently, on purpose:

- At **save** time, "no such dashboard" and "not one you can see" are one
  answer (403). The id is arbitrary caller input at that point, so separating
  them would make placement creation a way to enumerate other people's
  dashboard ids — the same reasoning `GET /dashboards/{id}` already follows.
- At **click** time they are two answers: 403 for "you do not have permission",
  404 for "the target no longer exists". Nothing is leaked, because the caller
  was already shown that target id inside a dashboard they can read. Conflating
  them would send a user whose share was revoked hunting for a deleted
  dashboard, and a user whose target really was deleted asking its owner for a
  share that cannot help.

### `WidgetBinding::ResourceKindDisplay`

A third binding arm in Core:

```rust
ResourceKindDisplay { resource_kind: String }
```

This is the mechanism behind "a resource kind can be a normal placement,
including one large enough to fill an entire dashboard". A resource kind becomes
a *binding*, so it inherits everything a placement already has — geometry,
resize, grouping, per-target validation — instead of needing a fourth kind of
dashboard object with its own layout rules.

It is its own arm rather than a `DisplayWidgetType`, for the same reason
`Display` and `Action` are separate arms (ADR 0014): the rows come from
`list_resource_items` and not from `status.details`, so `resource_kind`
resolves against a **third** identifier space. Folding it into `dataPointId`
would produce a binding no validator could check.

It carries **no `widget_type` and no `config`**. There is exactly one way to
render a resource kind — the table/browser presentation clients already
implement for `GET /connector-instances/{id}/resources/{kind}` — and it adapts
to whatever area the placement occupies. Offering a widget type here would
invite per-kind rendering variants no connector asked for, which the resource
browser would then have to be kept in step with.

Validation matches the rigor of the other two arms: `resource_kind` must appear
in the connector's currently declared `resource_kinds(target_id)` **for this
placement's target**, read per target because `resource_kinds` takes one
precisely so a kind can be absent at one scope and present at another (ADR
0021).

`DebugConnector` ships one such binding in its `default_layout()`. A variant no
fixture exercises is a variant every renderer, validator and contract harness is
free to quietly not handle.

### `hidden` is presentation, not access control

A `hidden BOOLEAN NOT NULL DEFAULT false` column on `dashboards`, settable by
the owner through the existing update endpoint — the same tier as renaming,
because hiding changes what everyone the dashboard is shared with sees in their
own list.

`GET /dashboards` keeps returning hidden dashboards, each carrying its `hidden`
value, and `GET /dashboards/{id}` is unaffected. Filtering them out of a sidebar
is a client decision made from the flag.

Suppressing them server-side would break the exact case the flag was added for:
a hidden dashboard is typically the destination of a `navigate` tile and must
stay fully reachable by id. It would also hide a dashboard from the only screen
that could unhide it.

## Consequences

- `WidgetBinding` gains a variant, which is a **breaking change for every
  exhaustive match** over it — in Core, in the connector contract test kit, in
  connector crates, and in placement validation. That is the intended cost of
  the closed enum: each site had to state what the new arm means to it.
- `PlacementResponse.connector` is now nullable, and clients must handle a tile
  with no connector. `placementAction` is a new nullable field on every
  placement.
- A dashboard can now be pointed at by a placement on a dashboard whose editors
  cannot see it — the click check, not the save check, is what stops that being
  a leak. Any future click behavior that reaches something permissioned must
  re-check at click time in the same way.
- Deleting a dashboard leaves `navigate` placements pointing at it. They are not
  cleaned up; they answer 404 on click, which is the honest state and costs
  nothing to store. A cascade would have to reach into JSON in a text column to
  find them.
- `hidden` is not a security boundary and must never be presented as one. A user
  who can reach the id can reach the dashboard, exactly as before.
