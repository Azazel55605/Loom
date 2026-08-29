# 0026. Group summaries and status cells are the connector's answers, not the client's

- Status: accepted
- Date: 2026-08-29
- Amends: [0024](./0024-resource-kind-presentation-hints.md)

## Context

[ADR 0024](./0024-resource-kind-presentation-hints.md) added `group_by_key` so a
long table could be read in sections. Docker's image table was the case that
earned it, and the sections turned out to be nearly useless closed: a heading
reading `nginx` and a row count says nothing about whether there is anything in
there worth opening.

Two facts would make it worth opening — how much disk that repository is using,
and whether any of it is unused. Both look like things a client could compute
from the rows it already has. Both are wrong when it does:

- **Size.** Docker lists one row per *tag*. Three tags of one 2 GB image are
  three 2 GB rows, and summing them reports 6 GB of disk that does not exist.
  Deduplicating needs the image id, which is not a column and should not become
  one — what a reader wants to see is the short id, once.
- **Usage.** "Some of these are unused" is not derivable from a column of
  per-row verdicts either: two tags of one used image are one used image, not
  two, and a client counting rows would call that repository half-unused.

Separately, the per-row verdict itself needed somewhere to live. A `text` cell
reading `unused` is not the thing — it is a judgement, and it should look like
one.

## Decision

**`group_summary: Vec<ColumnDescriptor>`** on `ResourceKindDescriptor`. Each
descriptor's key names a field every row of a group carries with the *same*
value; the client reads it off any row of the group and renders it on the
heading. The connector computes it.

The alternative — a generic "aggregate these columns" instruction, with the
client summing or counting — was rejected because the two real cases are both
wrong under it, and a mechanism whose first two uses are both wrong is not
generic, it is merely untyped.

**`ColumnValueType::Status`**, whose value is a `{ label, tone }` object rather
than a bare string. **The connector supplies the tone.** A client cannot know
that "unused" is reclaimable disk for an image and a failure for a backup job,
and a lookup table of known words in the frontend would be a connector's
vocabulary living in someone else's code. Tones name *sentiment* — neutral,
positive, caution, negative — not colours, so a client is free to render them in
a high-contrast or colour-blind palette without any connector having encoded a
hex value.

Two client-side decisions that follow, and are the client's to make:

- **Grouped kinds start collapsed.** A kind is grouped because its list is long;
  opening all of it by default hands the reader the wall of text the grouping
  exists to spare them.
- **Every kind gets a search box**, filtering on the text a reader can see — the
  declared columns, the grouping value, a status pill's label — and never on the
  raw `fields` object or a row id nobody typed. Finding rows for reasons nothing
  on screen explains is worse than not finding them.

## Consequences

Both additions are `#[serde(default)]` and additive in the trait, so a
descriptor written before them behaves exactly as it did. A kind with no group
summary renders the heading it rendered before; a client that ignores
`group_summary` renders a correct table.

The redundancy is deliberate: a group's summary values are repeated on every one
of its rows. For Docker's image table that is two extra fields across a few
hundred rows, which is nothing next to being able to compute them correctly.
The alternative shapes — a parallel array of group objects, or a second
endpoint — both make the rows and their summary two things that can disagree.

`Status` is the first column type whose value is an object rather than a scalar,
which is a real step. It is justified by the tone genuinely being data the
connector owns; a type whose only argument was "it looks nicer" would not be.

This does put more work on connector authors: a group summary is code somebody
has to write, and getting it wrong is now possible in a way that a generic sum
would not have been. That is the correct trade. A generic sum could not have
been right here at all, and a wrong number on a heading is worse than no
heading, because it looks like an answer.
