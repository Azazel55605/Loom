# 0021 — Connector resource browser

- Status: accepted
- Date: 2026-08-28

## Context

Docker needs Loom to show images, volumes, networks, logs, and available
updates. Every one of those is the same shape: many things, a handful of
columns per thing, an operation or two on a single row, and occasionally an
operation on the whole set ("prune", "pull all"). The same shape turns up
immediately outside Docker — a backup tool's snapshots, a package manager's
installed units, a torrent client's transfers.

Nothing already in the connector contract fits it:

- A **data point** ([ADR 0014](./0014-widget-binding-model.md)) is one reading
  that drives one widget. A list of forty images is not a reading, and binding
  it to a stat tile is meaningless.
- A **sub-target** ([ADR 0016](./0016-connector-sub-targets.md)) is an
  addressable *view* of the same service, with its own status, data points, and
  dashboard placements. A Docker image has no health and belongs on no
  dashboard; making one a sub-target would put a row in the status poll, the
  placement UI, and the widget binder for something that is a table row.
- **Discovery** proposes whole new connector instances. Browsing proposes
  nothing.

The available choice was therefore between building images, volumes, networks,
logs, and updates as five Docker features, or building the shape once.

ADR numbers 0017 through 0020 are already taken by accepted decisions, so this
one uses the next free number rather than rewriting history.

## Decision

Add a generic **resource browser** to the connector contract in Core, and prove
it against `DebugConnector` before any real connector uses it — the same order
this project has established for every previous capability (widgets, discovery,
setup guides, offline handling).

A connector publishes `resource_kinds() -> Vec<ResourceKindDescriptor>`, each
kind carrying:

- `columns`: `ColumnDescriptor { key, label, value_type }`,
- `row_actions`: operations on one row,
- `kind_actions`: operations on the collection.

Rows arrive separately from `list_resource_items(kind, target_id) ->
Vec<ResourceItem>`, where a `ResourceItem` is an `id` plus a `fields` map keyed
by column. Descriptors and readings are kept apart for the same reason as
`data_points()` and `ConnectorStatus::details`: a client renders the table
structure once and refreshes the contents underneath it.

Both trait methods have defaults (`Vec::new()` / `Ok(Vec::new())`), so this is
additive — no existing connector changed, and none has to.

### `ColumnValueType` is not `DataPointValueType`

They overlap on `text`/`number`/`bool` and diverge exactly where it matters. A
data point needs `timeSeries`, because its type decides which *widget* may bind
to it. A table cell never needs a series, and does need the two cases a raw
number renders badly as: a `bytes` count that should read `1.4 GB`, and a
`timestamp` that should read in the viewer's own locale. Merging them would give
every widget binding two variants it cannot draw and every cell one it cannot
fit.

### Row targeting travels in `params`, not in a new trait parameter

A row action names its row with `params["resourceId"]`, matching the row's
`ResourceItem::id`.

The alternative was a third argument on `execute_action(action_id, target_id,
resource_id, params)`. That is a breaking change to the one method every
connector implements, in every crate and every future third-party connector, to
express something the existing `params` value already carries perfectly well.
`execute_action` has already absorbed one signature change for sub-targets; a
second one for a value that is definitionally an action parameter would be
paying a migration cost for nothing.

The trade is honest: the convention lives in documentation and in each
connector's `params_schema` rather than in the type system, so a connector
author can forget it. That is bounded by making the failure loud — a row action
receiving no `resourceId` must return `ConnectorError::InvalidParams` rather
than guess — and by `DebugConnector` demonstrating the enforced form.

### Resource actions are actions

They dispatch through the existing
`POST /connector-instances/{id}/actions/{actionId}` endpoint, require the
existing `connectors.control` permission, and are scoped to the same
`connector` / `{id}` resource. **No new permission key and no new tier.**
Browsing a table is `connectors.view`; acting on a row is `connectors.control`;
those are the two tiers that already exist and they already mean the right
thing. A new key would have to be registered by migration
([the permissions rule](../CLAUDE.md)) and would then need a story for why
"delete this image" is a different authority from "stop this container".

The action endpoint's validity check accordingly spans both universes: an
`actionId` is valid if it appears in `actions()` **or** in any resource kind's
`row_actions`/`kind_actions`, checked against the live descriptors.

### The backend validates `kind`; Core does not

`list_resource_items` returns an empty list for a kind a connector does not
have — the trait's default behaviour, and what any connector's `match` arm falls
through to. That makes "no such kind" and "this kind is empty" identical at the
trait level, which is fine for a connector and useless for a user staring at an
empty table.

So `GET /connector-instances/{id}/resources/{kind}` checks `kind` against that
instance's live `resource_kinds()` and answers 400 for one that is not there.
The check sits in the backend because that is where a client's request is
validated, and it uses the live descriptor list rather than a cached one because
a connector's kinds may depend on its configuration and state.

## Consequences

- Docker's images, volumes, networks, logs, and updates become five
  `ResourceKindDescriptor` values and one `list_resource_items` implementation,
  not five features. Nothing in the platform, the API, or the clients has to
  learn what an image is.
- One table renderer serves every connector, present and future, because it is
  built against `ColumnValueType` rather than against Docker.
- `DebugConnector` gains two fixture kinds (`widgets`, `gadgets`) covering all
  five column types and both action scopes, so the endpoints and the eventual
  UI are testable on a laptop with no homelab — consistent with that fixture's
  standing purpose.
- A row action's `resourceId` is enforced per connector rather than by the
  compiler. Accepted deliberately; see above.
- Pagination, sorting, and filtering are **not** in this version. A homelab's
  image list is tens of rows, and adding a query protocol before a real
  connector has demonstrated the need would be guessing at a shape. When one
  does, it extends `list_resource_items` and this ADR is superseded rather than
  edited.
