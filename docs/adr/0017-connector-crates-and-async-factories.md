# 0017 — Real connectors live in their own crates, and factories are async

- Status: accepted
- Date: 2026-08-23

## Context

Until now every connector was in `crates/core`: the trait, and one
implementation of it — `DebugConnector`, which contacts nothing.
[`ARCHITECTURE.md`](../ARCHITECTURE.md) accordingly described Core as holding
"the connector trait and its implementations", which was true and cost nothing
while the only implementation had no dependencies.

The Docker connector is the first one that talks to something real, and it
brings two problems the fixture never could.

**It has a dependency tree.** `bollard` pulls in an HTTP client stack, a Unix
socket transport, and their transitive dependencies. Core is linked into the
desktop and mobile clients ([0001](./0001-workspace-and-crate-boundaries.md)),
so putting it there would put an entire Docker Engine API client into every
binary Loom ships, including the ones running on a phone that will never speak
to a Docker daemon. Multiply that by every future integration — a hypervisor
SDK, a reverse-proxy client, a NAS API — and Core stops being a shared library
and becomes the union of every service Loom has ever integrated with.

**Its construction does I/O.** `ConnectorFactory` was
`fn(Value) -> Result<Box<dyn Connector>, ConnectorError>`: synchronous, because
building a fixture is synchronous. But a connector to a real service can only
validate a configuration by *using* it, and the Docker connector's two failure
modes need to stay apart — "the daemon is unreachable" points at a socket, a
bind mount, or a network; "the daemon has no such container" points at a text
field. Distinguishing them requires a connect and an inspect, which are async.

## Decision

### One crate per real connector

`crates/connector-docker` (package `loom-connector-docker`) depends on
`loom-core` for the trait and the shared wire types, and on whatever client
library it needs. `crates/web-backend` depends on it and registers it. Core
gains nothing and keeps its dependency list as it was.

`DebugConnector` **stays in Core** and is the deliberate exception. It is a
fixture used by Core's own tests, it depends on nothing, and moving it out would
mean Core's test suite could not exercise the trait it defines. That is the
rule: *the trait and a dependency-free fixture live in Core; anything that opens
a connection to a real service lives in its own crate.*

`ARCHITECTURE.md` is updated to say so, since it previously said the opposite.

### `ConnectorFactory` is async

```rust
pub type ConnectorFactory =
    fn(Value) -> Pin<Box<dyn Future<Output = Result<Box<dyn Connector>, ConnectorError>> + Send>>;
```

`ConnectorRuntime::build` becomes `async` with it. Both call sites — startup
loading and the create/update instance handlers — were already async, so this
is `.await` in three places and no restructuring.

The alternative was keeping the signature synchronous and having the Docker
connector block on a runtime of its own — `block_in_place` (which panics on a
current-thread runtime, i.e. in every `#[tokio::test]`) or an OS thread with a
throwaway runtime joined synchronously (which serialises startup across every
stored Docker instance, at up to the connection timeout each). Both are worse
answers to a question the type can simply be honest about: constructing a
connector to a real service is I/O, and pretending otherwise only moves the
blocking somewhere less visible.

### Type-level descriptors come from `const`s when there is no default instance

The registry builds each registration by snapshotting a default connector
instance — schema, icon, setup guide, discoverable type — so that those cannot
come from different throwaway objects. The Docker connector cannot be
default-constructed: there is no default endpoint that is reachable and no
default container that exists. Its type-level descriptors (`TYPE_ID`,
`DISPLAY_NAME`, `ICON`, `config_schema()`) are therefore free items in the
connector crate, read both by the registration *and* by the connector's own
`metadata()`. That is stronger than the snapshot it replaces: the two cannot
drift, because there is only one of each value.

## Consequences

- Adding an integration is: a new crate, a dependency line in `web-backend`, and
  a registration. Nothing in Core changes, and no client grows.
- Each connector versions independently through `versions.json`, which is what
  `ConnectorMetadata.version` has always promised the API — a connector can be
  revised without a platform bump.
- A connector crate is a natural extraction boundary if connectors ever become
  dynamically loaded or sandboxed, which [0002](./0002-connector-contract-tbd.md)
  leaves open. This decision does not settle that question; it stops making it
  harder.
- Construction cost is now real. A stored Docker instance whose host is down
  delays startup by that connector's connection timeout, and the runtime's
  existing log-and-skip behaviour applies — a bad connector still cannot stop
  the server from starting. Loading connectors concurrently is the obvious next
  step if that ever bites, and is not done here because one connector type does
  not justify it.
- `web-backend`'s dependency list grows per integration. That is the honest
  place for it: `web-backend` is the only thing that decides which connectors a
  build contains.
