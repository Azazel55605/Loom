# 0027. Docker stacks are a second kind of sub-target, not a second connector

- Status: accepted
- Date: 2026-08-29
- Amends: [0016](./0016-connector-sub-targets.md), [0024](./0024-resource-kind-presentation-hints.md)

> **Numbering.** This was specified as `0019-docker-stacks.md`, but 0019 is
> `docker-connector-merge`, which is itself already superseded by
> [0016](./0016-connector-sub-targets.md) — the sub-target decision this builds
> directly on. It takes the next free number instead of overwriting an existing
> record.

## Context

Almost everything on a homelab Docker host arrives as a Compose project: five
containers that start together, stop together, and are read as one service by
the person running them. Loom could address each of those five and the host, and
nothing in between. "Restart the media stack" meant five clicks and remembering
which five.

Compose already records the grouping — every container it creates carries a
`com.docker.compose.project` label — so the information was there. What was
missing was a way to *address* it.

## Decision

**A stack is a sub-target.** Not a new connector type, not a new placement kind,
not a Loom-side grouping the user maintains. `list_sub_targets` returns them
alongside containers, and everything that already works for a sub-target —
placements, status details, actions, the detail modal, the action log — works
for a stack on the day it appears, with no code that knows what a stack is.

Three things this required.

### `stack:{project}` as the target id

Docker container names are restricted to `[a-zA-Z0-9][a-zA-Z0-9_.-]*`. No
container can contain a colon, so no existing target id changes meaning and no
saved placement moves. The prefix is not decoration: it is the reason this
addition is not a migration.

Stacks are **added beside** their members, never substituted for them. Someone
who placed one container on a dashboard did not ask for that tile to become a
stack.

### `SubTarget::kind`, a free-form string

A client needs to tell a stack from a container to group or icon them, so
`SubTarget` gains a `kind`. It is **not** an enum. A closed set would have to
name every kind of thing every connector will ever address, and the first
connector wanting a "pool", a "share" or a "zone" would either wait for a Core
release or misuse the nearest word. This is the same choice already made for
connector type ids, action ids and data point ids, and it comes with the same
rule: nothing in Loom branches on it, and a client must tolerate a value it does
not recognise by treating the target as ordinary. The connector distinguishes
behaviour from its own `target_id`, which is the thing it actually receives.

### `resource_kinds(target_id)` — a breaking signature change

`ApplicableTarget` ([0024](./0024-resource-kind-presentation-hints.md)) lets a
descriptor say *where* it belongs, and that was sufficient while every
sub-target of a connector was the same sort of thing. It stops being sufficient
the moment they are not: "the containers in this stack" is a table a stack has
and a container does not, and `TargetOnly` cannot express that, because a
container is a target too.

The alternative was to return `stackMembers` always and let it be *empty* for a
container. That is exactly the failure 0024 already rejected for
`applicable_target`: an empty listing cannot distinguish "this does not apply
here" from "there are none right now", which is the difference between a tab
that will fill and one that never will.

So the method takes the target. The change is small and total — four
implementations, one default body, and a query parameter on one route — and it
is the sort of change that is cheap now and expensive after a third connector
ships kinds.

### Per-target state is a data point, never health

A stack reports `overallStatus` — `Running`, `Stopped`, `Partial` — as a plain
`String` data point, exactly like a container's own `status`.

**It does not touch `ConnectorStatus::health`.** That field means "can Loom
reach this Docker daemon". A stopped stack is not an unreachable daemon; it is a
stack somebody stopped, and a deliberate maintenance window must not paint a
dashboard tile red or fire an alert. This is the same rule already applied to
individual containers when sub-targets were introduced in
[0016](./0016-connector-sub-targets.md), written down here because a stack's
"partial" state is the first reading where collapsing the two would be
*tempting* — it looks like degradation, and it is not.

## Consequences

Stacks cost the daemon nothing. Their aggregates are summed from readings the
poll already takes per container, and the members table lists those same
readings; both are non-`async` functions that cannot make a request even by
accident. A test measures this rather than asserting it in prose: a poll of a
host whose containers carry a Compose label makes exactly the same number of
requests as a poll of the same host without one.

Bulk actions run **sequentially**, like `updateAll` and for the same reason, and
report per-member outcomes. "The stack action failed" is not something anyone
can act on; "redis refused, the other four stopped" is.

`pause`, `unpause` and `applyUpdate` are refused for a stack. Each is a
per-container operation whose meaning across a set of interdependent containers
is not obviously "all of them", and a control nobody can predict is worse than
no control. Updating a whole stack is a real want — it is a later decision, and
it belongs with the update-management model in
[0023](./0023-docker-update-management.md), not smuggled in here.

The connector now reads a label written by a *different tool*. If Compose
changes that label the stacks quietly disappear, which is the correct failure:
Loom would be wrong to keep asserting a grouping the source of truth no longer
claims. Nothing persists, so there is nothing to clean up when it happens.
