# 0018 — Offline and disruptive states: an overlay, a backoff, and a TCP probe

- Status: accepted
- Date: 2026-08-24

## Context

Three failures of the connector status pipeline show up the moment it points at
something real rather than at `DebugConnector`.

**A restarting service reports Down, and that is useless.** `restart` takes a
container away and brings it back. Every poll landing in that window reports
`Down`, correctly, and a dashboard turns red for something the user asked for
ten seconds ago. The reading is true; it is the wrong sentence.

**A dead host is polled at full speed forever.** The poller ticked every five
seconds and asked every instance. A connector whose host has gone away spends a
full timeout — ten seconds, for the Docker connector — before answering. That is
a poll that costs twice its own interval, repeated indefinitely, for a service
nobody expects back soon.

**"Down" does not say why.** A connector can only report that it stopped
answering. Whether the DNS name stopped resolving, the host stopped accepting
connections, or the service crashed behind a perfectly healthy socket are three
different afternoons, and Loom knew which one and was not saying.

## Decision

### The pending operation is an overlay, not a status

`ConnectorAction` gains `is_disruptive: bool`, defaulting to `false`. While such
an action runs, the backend records a `PendingOperation { action_label,
started_at }` for that instance, and the status payload carries it **beside**
`status` rather than inside it.

`ConnectorStatus` is unchanged, deliberately. It is a Core type that every
connector implements and all three clients deserialize; adding a
`Restarting` variant would mean every connector author and every `match` in the
tree learning about a platform concern they cannot observe. Worse, it would
*destroy information*: a service mid-restart really is Down, and a client that
wants to know that should still be able to read it. The overlay adds context; it
does not rewrite the reading.

`is_disruptive` is not "is this dangerous" and not "confirm before running" —
both are worth having and neither is this. The test is whether a user would be
**surprised** by the gap. `stop` makes a service stop answering, and the person
who pressed Stop is not surprised. `restart` is the case where a service
vanishes and is expected back, and nobody knows how long that should take. The
default is `false` because an action wrongly marked disruptive hides a genuine
outage behind "Performing…", which is a worse failure than the flicker this
prevents.

**A timed marker rather than a lock.** The marker is set immediately before the
action is dispatched, cleared when it returns either way, and dropped by a
safety net after two minutes if it never returns. Without the timeout, a
connector that hangs on a socket with no timeout of its own would pin an
instance to "Performing: Restart" until the process restarted — a longer-lived
lie than the one being fixed.

### Backoff is per-instance, and the poller schedules rather than sweeps

Each instance carries a `next_due` timestamp and a consecutive-failure count.
The interval is the base doubled once per failure, capped at two minutes, and
reset to the base by a single good poll — not decremented, because a connector
that has just answered is not "slightly less broken".

The poller wakes every second and polls whoever is due. The tick is the
schedule's *resolution*, not its rate. One loop, not a timer task per instance:
a hundred connectors is a hundred map entries and one wakeup a second, rather
than a hundred sleeping tasks whose lifetimes have to be reconciled with
instances being created and deleted.

**A failing poll is `Err` *or* a successful `Down`.** Counting only `Err` would
be the tidier definition and would miss the case that motivates the feature: the
Docker connector reports an unreachable daemon as `Ok(Down)` after spending its
whole timeout, so backing off only on `Err` would leave the most common outage
polling at full frequency.

The cost is honest and worth stating: a service fixed **outside** Loom can take
up to the cap to be noticed, because nothing tells us it was fixed. Two minutes
is chosen as the low end of defensible for exactly that reason. Anything fixed
*through* Loom is immune — every action brings its instance's next poll forward
and runs it, since pressing a button is the strongest available signal that the
state is about to change and that somebody is watching. The failure history
survives that refresh, so an instance that is still broken drops straight back
to the interval it had earned.

### The diagnostic is a TCP connect, and explicitly not ICMP

A connector may publish a `NetworkTarget { host, port }`. When an instance goes
Down and publishes one, the backend resolves the host and attempts a TCP
connection with a three-second timeout, yielding one of three sentences: DNS
failed, the host is unreachable on that port, or the host is reachable and the
service is not.

**Ping is the reflex and it is wrong twice over.**

*It is not portable.* An ICMP echo needs a raw socket, which needs `CAP_NET_RAW`
or root. Loom runs unprivileged in a container by design ([0004](./0004-zero-config-startup.md)),
so a ping-based diagnostic would report "unreachable" for every host on most
real deployments — the worst possible failure mode for a feature whose entire
job is to explain a failure.

*It answers a different question.* Plenty of hosts drop ICMP at the firewall
while serving happily on their ports; plenty of others answer ping from a
network stack whose services are all dead. "Ping fails" and "the service is
unreachable" are independent facts, and reporting one as the other is worse than
saying nothing. A TCP connect to the port the connector actually uses is the
same question the connector is asking, one layer down.

The probe has a real cost — it opens a connection to a service that is already
struggling — so it is debounced to once a minute per instance while it stays
Down, and it runs only against a port the user configured. A connector with
nothing meaningful to probe returns `None` and gets no diagnosis at all, rather
than a reassuring sentence that means nothing: a Unix socket is a file on this
machine, and "the host is reachable" about `localhost` is a tautology dressed up
as a diagnosis.

## Consequences

- Clients gain two nullable fields on the status payload, always present, and
  one rule: `pendingOperation` takes visual precedence over health. That rule
  lives in `lib/connector-availability.ts` so the card, the modal and the widget
  dispatcher cannot disagree about the same instance.
- Action controls are disabled with a themed tooltip when a connector is Down —
  but **not** while a pending operation is running, because the service is
  expected back and greying out a routine restart reads as an outage from the
  other direction. Degraded leaves controls enabled: disabling the restart
  button on the one connector someone is trying to fix would be backwards.
- `execute_action` now calls `actions()` before dispatching, to read the flag
  from the connector rather than from a name this route recognises. One extra
  call on a user-initiated endpoint; the alternative is hardcoding `"restart"`,
  which is right for Docker and wrong for the next connector that calls it
  `recreate`.
- Backoff is observable in the logs: a failing instance emits a line carrying
  `next_poll_in_secs`, and successive lines for one instance both grow and move
  further apart.
- `DebugConnector` gains a configurable `networkTarget`, so all three diagnostic
  outcomes are reachable on a laptop with no homelab — the same role its
  `fail_mode` already plays, one layer further out.

## Recovery and correlated outages

A later report that recovered services appeared permanently Down prompted a
full trace of all three stages. The capped scheduler continued scheduling, a
successful poll already reset its failure counter and updated the single status
cache/broadcast path, and the shared socket client neither deduplicated recovery
frames nor dropped subscriptions across reconnects. The observable gap was the
documented two-minute maximum backoff: correct eventual recovery could still
look stuck to someone who had just restored a service outside Loom.

Scheduled recovery remains automatic and a regression test now follows an
instance from Down through a due successful poll, asserting both the restored
five-second cadence and the pushed Healthy snapshot. In addition,
`POST /connector-instances/{id}/reconnect` lets any caller with
`connectors.view` bypass the due time for an immediate observation. It uses the
same poll/cache/broadcast path; success resets the failure history, while a
failed retry remains backed off and returns the fresh diagnosis to the caller.

The TCP probe result is also retained as structured internal evidence. A
rolling two-minute tracker counts **distinct resolved IP addresses** currently
failing specifically at TCP connect. Three or more activate a system-wide
network advisory, broadcast to every authenticated status socket; falling below
three or ageing out clears it. DNS failures and reachable ports with an
unresponsive service are excluded because they do not support the same
cross-host inference. Multiple connector instances aimed at one host count
once. Clients may dismiss one activation for their current UI session, but a
later inactive-to-active transition is presented again.

## Construction failures are retryable, like every other failure

A later report of connectors going permanently dark after a backend restart
traced to the one failure mode this ADR had not covered. Everything above is
about a connector that **exists** and is failing. Building one was treated as a
different kind of event: `ConnectorRuntime::load` called each type's factory in
turn, and a factory that returned an error produced a log line and a `continue`,
leaving that row out of the instance map entirely.

Nothing polls what is not in the map. An instance excluded that way could not
produce a failed poll, could not earn a backoff, could not be diagnosed, and
could not recover — it reported "no reading" forever, and the only way back was
to edit and re-save it by hand, one instance at a time. That was the only
failure mode in the pipeline with no recovery path, and it was reached by the
most ordinary event there is: the process restarting while a dependency was not
yet up.

Construction fails for the same reasons and just as temporarily as a poll. A
Docker daemon is pinged, a Music Assistant socket is opened, a Pi-hole session
is authenticated — all at construction time, all against a service that might be
thirty seconds behind this one in a boot order nobody controls.

**An instance is therefore `Live` or `Pending`, never absent.** A row whose
factory fails is inserted as `Pending`, holding the type id, the decrypted
configuration and the real error, with a status of Down carrying that error and
a due-now poll schedule. `poll_due` treats a due `Pending` entry as a
construction retry rather than a status poll; success promotes it to `Live` and
polls it immediately, and failure goes down the **same** `record_poll_outcome`
path a failed poll uses — one backoff curve, one diagnosis debounce, one
broadcast. There is deliberately no second retry mechanism, because two
implementations of "keep trying, but less often" is exactly how one of them ends
up never firing.

A pending instance is therefore indistinguishable from a live-but-Down one in
every way that matters: it backs off to the same two-minute ceiling, it is
probed by the same TCP diagnostic — asked of the type's no-I/O connection-test
constructor, since there is no connector object to ask — it contributes to the
same correlated-outage advisory, and it recovers on its own the moment its
service answers.

**Loading is concurrent and bounded.** Sequential construction under a single
write lock made startup cost the *sum* of every connector's handshake, and the
HTTP listener waited behind all of it. Each row is now built in its own task
under a twelve-second `CONSTRUCTION_TIMEOUT`, and the map lock is taken per
insert rather than held across every factory call. A factory that hangs is a
failed attempt like any other, retried on the ordinary schedule.

**What is still skipped, and why.** A row that cannot be *addressed* — an
unparseable id, stored configuration that is not JSON, a type this build does
not register, configuration that will not decrypt — is still logged and left out
of the map. None of those become true later, so retrying them would be a loop
with no exit. Such a row remains listed with stand-in metadata and a
`statusError` naming its type, exactly as before, so it can still be fixed or
deleted.

**Requests act on a pending instance rather than refusing it.** Reconnect,
discovery, sub-targets, resource browsing, media control and actions all go
through a helper that builds a pending instance on demand before answering.
A person pressing a button is the strongest available signal that somebody is
waiting, and the attempt takes about as long as the request they already made.
A failed on-demand attempt is recorded through the ordinary path, so pressing
repeatedly cannot reset a backoff the connector has earned, and the refusal now
carries the connector's own objection instead of the bare "not loaded", which
named a state rather than a cause.

The startup log line changed with the behaviour. `"skipping connector
instance…"` described a permanent exclusion that no longer happens; a factory
failure now logs that the instance is pending and will be retried
automatically, and the summary line reports live and pending counts separately.

One scheduling detail changed with it. An instance's next-due time used to move
only when its attempt *finished*, so anything outlasting the one-second tick —
a connector spending its full timeout, a construction retry waiting on a dead
host — was dispatched again on every tick until it returned, piling concurrent
attempts onto the one service least able to absorb them. The due time is now
claimed when the attempt is dispatched and overwritten by the interval the
outcome earns, so one instance has at most one attempt in flight.
