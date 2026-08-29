# 0024. Resource-kind presentation hints: `groupByKey` and `applicableTarget`

- Status: accepted
- Date: 2026-08-29
- Supersedes: nothing
- Amends: [0021](./0021-connector-resource-browser.md)

> **Numbering.** This was going to amend ADR 0017, but 0017 is
> `connector-crates-and-async-factories` — the resource browser is
> [0021](./0021-connector-resource-browser.md). Rather than rewrite 0021 in
> place, this is a new record: 0021's reasoning was correct when it was written,
> and the interesting content here is precisely *what a second use case
> changed*, which a silent edit would erase.

## Context

[ADR 0021](./0021-connector-resource-browser.md) defined a resource kind as a
label, some typed columns, and two sets of actions — the smallest thing that
could describe a table. It was proven against `DebugConnector`'s fixture kinds
and then used, unchanged, for Docker's `updates` and the platform's
`recentlyUpdated`. Deliberately small: a descriptor field that only one
connector ever sets is a Docker feature wearing a generic name.

Adding Docker's images, volumes and networks is the first time a *second*
connector-specific use case has arrived, and it broke two assumptions that
0021's shape had quietly made.

**A table can be too long to read flat.** A homelab Docker host holds thirty to
eighty images, of which four are `nginx` and six are `postgres`. Flat, that is a
list you have to search. The information that makes it scannable — that those
rows belong together — is in the rows already, and only the connector knows
which column carries it.

**A kind can be meaningless at one altitude.** `updates` and `recentlyUpdated`
have always been host-scoped in practice: both were written to ignore
`targetId`, and Docker's detail modal for one container showed an "Updates
available" tab that could only ever hold that container's own row or nothing.
Images, volumes and networks make it worse — they are the daemon's, and "the
volumes of one container" is not a narrower question, it is a different one with
no answer. There was no way to say so, and the only signal a client had was an
empty listing, which cannot distinguish *this does not apply here* from *there
are none right now*.

## Decision

Two optional fields on `ResourceKindDescriptor`, both defaulted so every
existing descriptor keeps its behaviour:

- **`group_by_key: Option<String>`** — a column key whose value rows should be
  gathered under. A **hint**: the rows are the same rows either way, and a
  client that ignores it renders a correct flat table. The connector sorts its
  rows by that key so a client's sections are contiguous without re-sorting.
- **`applicable_target: ApplicableTarget`** (`HostOnly` / `TargetOnly` / `Any`,
  defaulting to `Any`) — where the kind is worth showing. A client filters its
  tabs by it. An unrecognised value is *shown*, not hidden, so a newer backend
  inventing a fourth case cannot make a table vanish from an older client
  without explanation.

Docker's `updates` and the platform's `recentlyUpdated` are marked `HostOnly`,
which changes no behaviour and only says out loud what they already were.

## Consequences

Both fields are additive on the wire (`#[serde(default)]`) and additive in the
trait: a connector that sets neither is exactly as correct as before, which is
what let this be added after two connectors were already shipping kinds.

Grouping stays a client concern. Nothing in Core, the backend, or the wire
format changes shape because a kind is grouped — the rows are a flat array
either way — so a client with no room for collapsible sections (a phone, a
future terminal UI) is not obliged to grow one.

`applicable_target` is a **presentation** filter and deliberately not an
enforcement one: the backend does not reject
`GET .../resources/images?targetId=web`. A connector already answers a kind it
cannot scope by ignoring `targetId`, and turning a rendering hint into a
validation rule would mean the descriptor list becomes something a request can
fail against — a second, weaker authorization surface in the one place the
architecture is careful not to put one (see
[0020](./0020-connector-capability-model.md)).

The cost of getting this wrong is asymmetric and the defaults reflect it: a kind
shown where it does not belong is a puzzling empty tab, and a kind hidden where
it does belong is a feature the user cannot find and cannot report. `Any` and
"show what you do not understand" both fail toward the first.

Neither field is a slippery slope toward layout in the descriptor — column
widths, sort direction, colour. The line is that both of these answer a question
only the *connector* can answer (which column carries the grouping, whether the
kind has a per-target meaning), whereas the rest are answers about the *screen*,
which is the client's to give.
