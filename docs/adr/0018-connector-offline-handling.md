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
