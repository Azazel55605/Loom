# 0011 — Connector types are a code registry; connector instances are rows

- Status: accepted
- Date: 2026-08-21

## Context

[0007](./0007-in-process-connector-trait.md) defined the host-side `Connector`
trait but stopped short of saying where connectors come from. The backend
consequently held a hardcoded `Vec` with exactly one entry — the mock fixture —
and `GET /connectors` listed it. That is fine as scaffolding for the auth frame
and useless as a product: a homelab has two Docker hosts and three proxies, not
one of each, and the user has to be able to add them.

Two things are needed, and they are not the same thing:

1. **What kinds of connector does this build know about?** Answering this
   requires code — a Docker connector needs a Docker client, error mapping, and
   an action set that nobody can express as data. This set changes when the
   binary changes.
2. **Which connectors does this deployment actually have?** Answering this
   requires storage. It changes when a user clicks "add", and it must survive a
   restart.

Conflating them produces one of two bad designs. If connector types live in the
database, the rows have to carry executable behaviour, which is the plugin-
transport question [0002](./0002-connector-contract-tbd.md) deliberately left
open — and answering it by accident, here, would be the worst way to answer it.
If connector instances live in code, adding a second Docker host is a
recompile.

There is also an authorization gap. `connectors.view` and `connectors.control`
both describe a connector that already exists. Neither says anything about who
may decide that it exists.

## Decision

**Split the two, along the code/data line.**

### A connector *type* registry, in code

`crates/web-backend/src/connectors/registry.rs` holds a
`HashMap<&'static str, ConnectorTypeRegistration>`, built once at startup. Each
registration carries:

- `type_id` — the stable identifier stored on every instance of this type.
- `display_name` — for the type picker.
- `factory: fn(Value) -> Result<Box<dyn Connector>, ConnectorError>` — turns a
  stored configuration into a live connector, **or refuses it**.
- `schema: fn() -> Value` — the type's `config_schema()`, obtainable without an
  instance.

The schema function is separate from the factory because the "add connector"
form needs the schema *before* there is a configuration to build from;
`Connector::config_schema` is an instance method, and requiring an instance in
order to ask what an instance needs is a chicken-and-egg the frontend should not
have to solve. It is served by `GET /connector-types`, so the form is generated
from data and registering a new type needs no frontend change.

**Validation belongs to the factory, not to the schema.** The backend never
checks a submitted configuration against the published schema. It calls the
factory and reports what comes back. Only the connector knows that `baseLoad` is
a percentage or that two options are mutually exclusive, and a shape check would
pass configurations that the connector cannot actually be built from.

### Connector *instances*, as rows

`connector_instances` stores `id`, `connector_type`, `name`, `config` (JSON
text), `created_at`. The backend never interprets `config`; it is the factory's
input and nothing else's.

`connector_type` has **no foreign key**, because there is no table for it to
reference — the set of types is compiled in. A row naming a type this build does
not register is therefore possible, and is a case to handle rather than a case
to prevent.

### A runtime map of live instances

`Arc<RwLock<HashMap<Uuid, Arc<dyn Connector>>>>`, populated at startup from the
table and kept in step by every write: create builds and inserts, update rebuilds
and replaces, delete removes. A connector is not a value that can be rebuilt per
request — a real one will hold an HTTP client, a connection pool, a token cache —
so the row is the durable record and the map is the running thing it describes.

`Arc<dyn Connector>` rather than `Box`, because a handler must hold a connector
across an `await` without holding the map's lock; cloning the `Arc` out and
releasing the guard is what keeps one slow connector from blocking every other
request.

A row that cannot be turned into a live connector is **logged and skipped, not
fatal**. It is still listed by the API, with a `statusError` saying so, so it can
be fixed or deleted. Refusing to start would take authentication and every other
connector down over one bad configuration, against
[0004](./0004-zero-config-startup.md).

### A third permission: `connectors.manage`

Registered by migration alongside the other keys, and granted to the seeded
Administrators group.

| Key | Question | Scopeable? |
| --- | --- | --- |
| `connectors.view` | May you see this connector? | yes |
| `connectors.control` | May you press its buttons? | yes |
| `connectors.manage` | May you decide which connectors exist? | no |

`connectors.manage` cannot be scoped to a connector because the connector is
what is being created or destroyed. Folding it into `connectors.control` would
mean that granting someone a restart button also granted them the ability to
delete every connector on the instance.

## Consequences

- The old `GET /connectors` and `POST /connectors/{id}/actions/{actionId}` are
  **removed**, superseded by `/connector-types` and `/connector-instances`.
  Clients built against them break; the shapes are close but the paths and the
  id semantics are not. `{id}` is now an instance UUID, not a type name, which
  also means resource-scoped `connectors.control` grants now name a real
  instance instead of a hardcoded string.
- Adding a connector type is a code change plus one registration. Adding an
  instance is an HTTP request. That is the boundary this ADR exists to draw.
- `config` is returned by `GET /connector-instances/{id}` under
  `connectors.view`, so an edit form can be pre-filled. **This is only safe
  while no registered type stores a credential.** The first one that does needs
  either a redaction pass or a stricter permission on that field.
- The registry is a `HashMap` with `&'static str` keys, so a connector type
  cannot be registered at runtime. If a plugin transport ever lands (0002), the
  key type and the `fn` pointers both have to widen — to owned strings and boxed
  closures. That is a contained change, and it is the reason the factory is
  behind a type alias.
- Status is still request/response. Nothing polls, nothing caches, nothing is
  pushed. `GET /connector-instances` calls `status()` once per instance in
  sequence, so the list is as slow as the connectors in it. A caching poller
  with WebSocket push is the intended follow-up and was deliberately left out of
  this change rather than half-built into it.
- Deleting an instance cascades to nothing, because nothing references it yet.
  Once dashboards store widget placements against an instance id, that table
  will need an `ON DELETE` decision.
