# 0015 — Dashboard tile grouping is its own entity, of any size, and losslessly reversible

- Status: accepted
- Date: 2026-08-23

## Context

A dashboard built on [0013](./0013-dashboard-sharing-model.md) draws one tile
per placement, and a placement is one connector instance. That is the wrong
granularity for a real homelab: two containers on the same host, a proxy and
the service behind it, three drives in one array. These belong side by side in
one visual unit, and today they can only be dragged near each other and hoped
about.

The requirement is a tile holding several placements. Four things about it were
worth deciding once rather than discovering later:

- Placements in one tile can come from **different connector types**. Nothing
  about grouping is connector-specific, and it must be testable today, with the
  only connector that exists — several `DebugConnector` instances.
- Grouping is **retroactive**. A user arranges a dashboard, then decides two
  tiles belong together. Nothing at placement-creation time can predict it.
- Groups are **not pairs**. "Combine these two" is the obvious first feature and
  the obvious first thing to outgrow.
- Grouping is an **experiment**. Someone tries it, looks at it, and undoes it.

## Decision

### A group is a separate entity, not a parent placement

`dashboard_placement_groups` is its own table: an id, a dashboard, a bounding
box, a timestamp. Membership lives on `dashboard_placements` as a nullable
`group_id` plus a `group_order` sort key.

The rejected alternative was a self-referential `parent_placement_id` on
`dashboard_placements`, making one placement the container for others. It is
fewer tables and it is worse. Every placement row would become ambiguous — is
this a connector tile or a container? — and a container row would carry a
`connector_instance_id` and `widget_bindings` that mean nothing, or a nullable
connector id that makes those columns meaningless for *every* row. A group has
no connector and no bindings. It has a box and an ordered membership, and
modelling it as the thing it is costs one table.

Membership is a column rather than a join table for the same reason: a
placement is in at most one group, so a join table would model a many-to-many
that is forbidden, and would then need a unique constraint to forbid it again.

### Any number of members, from two

No API, column, or code path assumes two. `placementIds` is a list, membership
is a query, ordering is an integer.

Two is the floor, enforced on creation and maintained afterwards. A group of one
is the placement it contains with an extra layer of indirection: it renders the
same, it is dragged the same, and its only distinguishing property is that a
user has to know to ungroup it before they can move the placement inside it
independently.

### Membership below two auto-dissolves the group

If a group drops to fewer than two members, the group row is deleted and any
remaining member returns to standalone.

This is the least obvious behaviour here, so it is stated plainly: **removing
one member of a pair un-groups both placements and destroys the tile.** The
placement that was not named in the request is also affected. The alternative —
a surviving one-member group — is a tile a user cannot dismantle without
discovering an endpoint they have no reason to look for.

Membership can fall below two by more than the obvious route, which is why the
rule is enforced in one shared sweep rather than at the one call site that
suggests it:

| How | Where the sweep runs |
| --- | --- |
| A member is removed from the group | `DELETE …/placement-groups/{id}/members/{placementId}` |
| A member placement is deleted from the dashboard | `DELETE …/placements/{id}` |
| The connector instance behind a member is deleted, cascading its placements away | `DELETE /connector-instances/{id}` |

That third path is why `routes::connectors` calls into `routes::dashboards`.
A rule that held on two of its three paths would leave one-member groups on real
dashboards, invisible until someone wondered why a tile would not move.

### Grouping is lossless because member geometry is preserved

A member keeps its own `position_x`, `position_y`, `width` and `height`. Those
four columns are **retained and ignored** while `group_id` is set: the group's
bounding box governs grid placement, and the member's own geometry is simply not
read by the renderer.

Not clearing them is the entire mechanism. Ungrouping is a write of `NULL` to
two columns, after which every placement renders standalone again exactly where
and at what size it was before. Clearing the geometry on grouping would mean
inventing a position at ungroup time — and a dashboard that rearranges itself
because a user tried a grouping and changed their mind is a dashboard people
stop experimenting with.

The cost is that those columns are stale-by-design for a grouped placement.
`PATCH …/placements/{id}` still writes them, and what it edits is the geometry
the placement will return to. This is recorded in the migration, in the field
doc comments, and in `docs/API_CONTRACT.md`, because a column whose value is
ignored under some condition is exactly the sort of thing a future reader
"cleans up".

### The detail response separates the two kinds of tile

`GET /dashboards/{id}` returns `placements` (standalone only) and
`placementGroups` (each with ordered `members`) rather than one flat array with
a `groupId` on each entry. The server is the only party that knows the
partition and the member ordering; a flat array makes every client re-derive
both, and get the ordering wrong in a different way each.

This is a breaking change to that response. It is taken now rather than
deferred: dashboards landed in the same development cycle and the only consumer
is in this repository.

## Consequences

- One new table, two new nullable columns, five new endpoints, all under the
  existing Editor-or-better dashboard ACL. No new permission key — grouping is
  a layout edit, and 0013 already decided layout edits are an ACL question.
- A `CHECK ((group_id IS NULL) = (group_order IS NULL))` makes "grouped" one
  fact rather than two that can disagree, and a partial unique index on
  `(group_id, group_order)` keeps member ordering total. The uniqueness is
  checked per statement, so reordering writes negative interim values in a first
  pass before the final `0..n-1` — noted here because it looks gratuitous
  otherwise.
- The `group_id` foreign key has **no** `ON DELETE` action. Deleting a group
  while a member still points at it fails loudly instead of silently orphaning
  it, which forces every dissolve path to clear membership first — the invariant
  the auto-dissolve rule depends on.
- `group_order` is a sort key, not an array index. Removing a member leaves a
  gap, additions append past the current maximum, and only relative order is
  ever read. Nothing renumbers, so nothing has to be atomic that is not.
- A group's members and the group itself must belong to the same dashboard.
  That is not expressible as a foreign key without a composite one, so it is
  enforced in the handlers: every membership query is scoped by `dashboard_id`.
- Frontend rendering of grouped tiles is not part of this change. The API is
  what a renderer needs; the renderer is the next step.
