# 0023 — Docker update management

- Status: accepted
- Date: 2026-08-28

## Context

"Is my container running an old image, and can Loom fix that?" is the request
that makes a homelab dashboard into a homelab *manager*. It needs four things
that did not exist: a way to ask a registry what a tag points at now, a way to
replace a running container without losing what makes it that container, a
schedule, and a way back when an update turns out to be wrong.

The pieces already in the tree did most of the fourth: the action log records
every invocation with a pre-action snapshot of whatever data points an action
declares ([ADR 0022](./0022-action-log-and-update-checking.md)), and the
connector trait already had `check_for_updates` with a default. What was missing
was a real implementation and the machinery around it.

## Decision

### Checking is a manifest `HEAD`, and authentication is discovered

An update check is `HEAD /v2/<repository>/manifests/<tag>` against the registry
in the image reference, reading `Docker-Content-Digest` and comparing it with
the daemon's own `RepoDigests` for the local image. Nothing is downloaded.

Authentication is **discovered rather than configured**: an unauthenticated
request that needs a token comes back `401` with an RFC 7235
`WWW-Authenticate: Bearer realm=…,service=…,scope=…` challenge naming the token
service, which is then asked and the request retried exactly once. Following the
challenge is what makes one code path work against Docker Hub, GHCR, and a
registry that needs no token at all, with no hostname special-cased anywhere.
The only Docker Hub specifics are the two Docker itself defines: the default
registry host for an unqualified reference, and the `library/` namespace for a
bare official name.

Verified against the live APIs while implementing (August 2026), because a
stale understanding of either would have been invisible until someone's homelab
was quietly reporting nothing:

- Docker Hub answers a bare manifest request with exactly that challenge, and
  the token service returns both `token` and `access_token`; either is read.
- The `Accept` header must offer all four manifest media types (OCI image and
  index, Docker v2 manifest and manifest list) — a modern multi-architecture
  image is an *index*, and a request accepting only a manifest is answered with
  a 404 for an image that is plainly there.
- Docker's current documentation states that **version checks do not count**
  towards the pull limit, and a live `HEAD` did indeed leave
  `ratelimit-remaining` untouched.

That last point matters for the default interval, and it is the opposite of the
obvious assumption. The six-hour default is **not** "each check is a pull". It
is that the limit is enforced per source address, and a homelab's address is
shared with everything else on it — the CI runner, the compose pull, the other
update tool. Six hours keeps Loom's contribution to a shared budget negligible
for any realistic number of containers, for images that do not change faster
than a working day. Anyone can set it lower; the *default* should not be the
thing that puts a household over someone else's limit.

### Rollback is `applyUpdate` with the previous reference

One action, `applyUpdate { targetImageRef }`, pulls a reference and recreates
the container on it. Given a newer reference it is an update; given the
reference the container ran before, it is a rollback. There is deliberately
**no `rollback` action**.

The alternative needs somewhere to remember what to roll back *to* — a
`previous_image` column, a per-container history table, a bespoke undo record —
and that store already exists as the action log. `applyUpdate` declares
`snapshot_data_point_ids: ["currentImageRef"]`, so every invocation records what
the container was running immediately before it, through the generic mechanism,
with no Docker-specific bookkeeping at all. The "recently updated" table is that
log rendered as rows, and its row action is `applyUpdate` again with the
snapshotted value. The feature is a *view*, not a subsystem.

This is what the snapshot mechanism was built for, and it is worth stating that
the fit was checked rather than assumed: the value a rollback needs is a data
point the connector already reports, on the target the action already names, at
the moment the action already runs.

### Order of operations, and where a failure leaves you

Pull, inspect, stop, remove, create, start. The pull is **first** so the most
likely failure — a bad tag, an unreachable registry, a rate limit — costs
nothing: the container is still running, untouched, and the result says so in
those words. Every later failure point reports which one it was and what state
the host is now in, because "the old container was removed but the new one could
not be created" and "the new image was pulled but the container would not stop"
call for completely different next actions by whoever is reading.

What is carried across a recreate is the container's creation config *and* its
`HostConfig` *and* its network attachments — environment, command, entrypoint,
labels, exposed ports, volume binds, port bindings, restart policy, resource
limits. Losing the `HostConfig` is the failure that silently detaches someone's
data volume, so the preservation is asserted field by field in a unit test and
end to end against a real daemon.

### The scheduler is a second background task, and it staggers

Not folded into the status poller, and the separation is not organisational. The
poller asks a *local* daemon how something is doing every few seconds and backs
off when it fails; this asks a *third party* what exists every few hours and is
rate-limited by someone else. One schedule cannot serve both without a status
poll eventually waiting behind a registry.

Within a tick, checks run **sequentially with a pause between them**. A host
with thirty containers would otherwise open thirty registry connections in the
same instant from one address, which is both rude and the fastest way to be
told to go away. There is no deadline: a check that finishes a minute later than
it could have is indistinguishable from one that did not.

An auto-applied update goes through `invoke_action` — the same function the HTTP
endpoint calls — so it lands in the audit log, raises the pending-operation
overlay, and triggers the same immediate re-poll. An automation with a quieter
path of its own is an automation whose actions are invisible exactly when
someone is trying to work out what happened overnight. Its actor is the system:
`invoked_by_user_id` is null and `invoked_by_system` is set, rather than a
reserved "system user" row that every user listing, permission check and account
deletion would have to remember to exclude.

### Settings are a convention, not a backend type

The scheduler reads `checkForUpdates`, `checkIntervalMinutes`,
`autoApplyUpdates`, `autoApplyAtTime` and `excludeFromAutoUpdate` out of the
instance's stored configuration by name. The keys are published by each
connector in its *own* config schema, with its own descriptions.

A Rust type in the backend naming Docker's fields would make the second
connector to want scheduled updates a backend change; a new trait method would
make every connector implement a policy struct it has no opinion about. A
documented set of key names costs a paragraph in the API contract and works for
the next connector without either.

### Two resource kinds, one of them the platform's

`updates` is connector-declared, served from the connector's own cache of what
its last check found — a browse must not spend registry budget.

`recentlyUpdated` is **provided by the backend**, and it is the one deliberate
exception to the rule that resource kinds are connector-declared. Its rows are
the action log's, and no connector can see the action log or should be given
it. It is offered for any instance whose connector reports
`supports_update_checking`, which is exactly the set for which "what did we
change, and what was it before?" has answers.

## Consequences

- Rollback exists as a consequence of the audit log rather than as a feature
  with its own state. Nothing has to be kept in sync, and any future connector
  that declares a snapshot on its own replace-style action gets the same
  behaviour for free.
- **Private registries and private repositories are not supported.** Requests
  are anonymous, so a private repository fails with an `AuthFailed` naming it
  and saying plainly that Loom has no credential to offer — not with a silent
  "up to date", which is the failure that would matter. Fixing it is a decision
  about where secrets live, not about HTTP, and belongs with the wider
  credential-storage question the connector config schema has been deferring.
- A container pinned to a digest, or built locally and never pushed, reports
  "up to date" rather than an error. There is genuinely no newer version of one
  exact digest, and a permanent error on a working container would train people
  to ignore the column.
- Update state is **not persisted**. A restart re-checks everything once,
  sooner than it strictly had to, which is the harmless direction; a persisted
  timestamp could hold a check back for six hours after an upgrade.
- `reqwest` with `rustls-tls` joins the tree. bollard speaks only to the daemon,
  and a registry query is an ordinary HTTPS request to a different host; rustls
  rather than native-tls keeps it off the host's OpenSSL.
- Log growth and check volume both scale with container count. Neither is a
  problem at homelab scale, and neither is being pre-solved here.
