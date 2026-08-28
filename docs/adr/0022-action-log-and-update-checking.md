# 0022 — Connector action log and update checking

- Status: accepted
- Date: 2026-08-28

## Context

Two capabilities were wanted at once, and both were about to be built as
Docker features:

1. **Update management.** "This container is running an old image" needs
   somewhere to report from, and applying the update needs a record of what was
   replaced so a bad update can be walked back.
2. **An audit trail.** Loom acts on services rather than only watching them, so
   "who restarted this, and when?" is a question it must be able to answer. It
   has been an open item since the platform grew its first real action.

Building either inside Docker would produce a per-connector answer to a
platform-wide question, and the second one would remain unanswered for every
other connector.

ADR numbers 0018 through 0021 are already taken by accepted decisions, so this
one uses the next free number rather than rewriting history.

## Decision

### The action log is platform infrastructure, not a feature

Every invocation through `POST /connector-instances/{id}/actions/{actionId}` is
written to `connector_action_log`. Not opt-in, not connector-declared, not a
flag on a request: the endpoint does it, so there is no version of "the action
ran but nothing recorded it" short of a bug.

Three properties follow from writing the row **before** dispatch:

- The caller, the parameters, and the pre-action state are recorded even when
  the action never returns. A row that stays pending (`completed_at IS NULL`)
  says the action was authorized and dispatched and Loom never learned the
  outcome — which is exactly what a restart that takes Loom's own process down
  with it looks like.
- The write **fails closed**. An action whose log row cannot be written answers
  500 and is not dispatched. An audit trail with gaps is worse than none,
  because the gaps are not random: they are whatever was happening when
  something was wrong.
- The completion *update* fails open. The action has already run by then;
  turning a successful restart into a 500 over an audit update would be the
  worse trade, and the pending row is itself readable.

Reading the log requires `connectors.view`, not `connectors.control`. Reading
history is looking, not doing, and the people most in need of "what happened to
this service?" are the ones without authority to have done it. **No new
permission key** was introduced — one would have to be registered by migration
and would then need a story for why reading an action's record is a different
authority from reading the instance it happened to.

### Snapshots are expressed as `snapshot_data_point_ids`

An action that wants its "before" recorded lists the data point ids worth
recording. The platform reads their current values from the **poll cache**,
scoped to the action's own `target_id`, and stores them on the log row.

The alternative was a bespoke mechanism per action type — a `Rollback` trait, a
`before_action` hook, a per-connector "capture state" method. Every version of
that requires the connector to be *called* before every action, doubling the
round trips and putting a network failure in the path of every button press;
and every version invents a second vocabulary for values that already have
one. Data points are already the platform's name for "a value this connector
reports", already declared, already polled, already keyed the same way in
`ConnectorStatus::details`. A snapshot is therefore three lines of connector
code — a list of ids — and no new concept at all.

The cost is that a snapshot is only ever as fresh as the last poll and only
ever as complete as what that poll reported. Both are accepted deliberately: a
reading taken between the decision and the dispatch is not meaningfully more
true, and refusing to run an action because a value was missing would be a far
worse failure than recording an incomplete snapshot. Ids the connector did not
report are simply absent.

This is what a rollback will be built on. It is not a rollback: nothing here
re-applies anything, because deciding that a previous value can be safely
restored is per-connector judgement and belongs in an action of its own.

### Attribution outlives the account, and the log dies with the instance

`invoked_by_user_id` references `users(id)` with **no** `ON DELETE` action, so
deleting a user who has invoked actions is refused. Attribution that a later
account deletion can quietly rewrite is not an audit trail. The delete-user
route pre-checks this and answers 409 with an explanation — the same shape as
its existing "this user owns dashboards" refusal — rather than letting the
database produce a 500 that explains nothing. Deactivation, which the platform
already supports and already prefers to deletion for exactly this reason,
remains available.

`instance_id` cascades the other way: deleting a connector instance deletes its
log. A history of something that no longer exists, whose action ids resolve
against nothing, is not evidence anyone can use.

### Update checking is a capability, not a Docker feature

`supports_update_checking()` and `check_for_updates(target_id)` join the trait
with defaults, returning `UpdateCheckResult { available, latest_ref }`.

`latest_ref` is opaque text. A structured current-versus-latest comparison
would require every connector to agree on what a version *is* — a digest, a
semver tag, a build number, a date — and they do not. Loom has no business
parsing another ecosystem's version scheme; it has business showing a badge and
naming what it found.

The check is read-only and non-committal by design. Whatever *applies* an
update is an ordinary `ConnectorAction`, which puts it behind
`connectors.control` and into the log above, where an upgrade belongs. That
also means an "update" action can declare `snapshot_data_point_ids` and get its
before-state recorded with no further mechanism.

## Consequences

- The audit-trail item is closed for every connector at once, including ones
  not written yet, as a side effect of the endpoint rather than as a per-
  connector obligation.
- Docker's update management becomes: implement `check_for_updates` against
  registry digests, and add an update action declaring the image reference as
  its snapshot. No platform work, no new endpoint, no new permission.
- `DebugConnector` proves both on a laptop with no homelab: its `recalibrate`
  action is disruptive *and* snapshot-bearing over a reading it then destroys,
  and its update check answers whatever `simulatedUpdateAvailable` says, so a
  clean instance and an out-of-date one are both renderable.
- Log growth is unbounded. Acceptable for now — a homelab produces actions at
  human rates, and the rows are small — but retention will eventually need a
  decision. It is deliberately not being guessed at here: a policy invented
  before anyone has a year of data is a policy about nothing.
- There is no *write* path to the log other than the action endpoint. A future
  automation that acts on connectors must go through it, which is a constraint
  worth keeping rather than working around.
