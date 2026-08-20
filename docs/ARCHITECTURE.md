# Loom Architecture

> Status: early work in progress. This document describes the structure we have
> settled on, not a structure that is fully built. Decisions are recorded in
> [`docs/adr/`](./adr/).

Loom is split into five parts. The split exists to answer one question
consistently: **who is allowed to do what?** There is exactly one place that
answers it — the Web/backend — and everything else is either a library or a
client.

```
        ┌──────────┐   ┌──────────┐   ┌──────────────┐
        │ Desktop  │   │  Mobile  │   │ Web/frontend │   clients
        │ (Tauri)  │   │ (Tauri)  │   │    (SPA)     │
        └────┬─────┘   └────┬─────┘   └──────┬───────┘
             │              │                │
             └──────────────┼────────────────┘
                            │  HTTP API
                     ┌──────▼───────┐
                     │ Web/backend  │  the one running server
                     │  auth · ACL  │
                     │   features   │
                     └──────┬───────┘
                            │ links
                     ┌──────▼───────┐
                     │     Core     │  library only, never standalone
                     └──────────────┘
```

## Core

A shared **library crate** (`crates/core`, package `loom-core`). It holds:

- the connector trait and its implementations — the code that actually talks to
  the services running in a homelab,
- business logic that must behave identically regardless of which client
  triggered it.

Core has **no network surface of its own**: no listener, no daemon, no
standalone mode. It is linked into whatever needs it. This is deliberate — Core
gets linked into clients that run on a user's machine, so it must never be the
thing deciding whether an action is permitted.

## Web/backend

The **one running server** (`crates/web-backend`). It depends on Core and owns:

- **authentication** — who the caller is,
- **access control** — what that caller may do,
- **feature management** — which capabilities are enabled at all.

Every client goes through its API. There is no side channel where a client
reaches a service directly.

## Web/frontend

A browser SPA. Deployed **independently** from the backend, so frontend and
backend can ship on separate cadences — a UI fix does not require redeploying
the server, and a backend release does not require a coordinated frontend push.
It is a client like any other: no privileges the API does not grant it.

## Shared UI/client package

`packages/ui-kit` (`@loom/ui-kit`) contains the React components, design tokens,
API wire types and transport, authentication context, and permission helpers
shared by browser and Tauri webviews. Platform-specific navigation, token
storage, and backend URL discovery are injected by each app. See ADR 0009.

## Desktop

A **Tauri** client. It consumes `@loom/ui-kit` so it looks and behaves like the
web frontend, and connects to Web/backend over the network. Native integration
must not become a way to bypass the API.

## Mobile

A **Tauri mobile** client. Same relationship as Desktop: shared UI/client logic
from `@loom/ui-kit`, all privileged operations over the Web/backend API.

## The invariant

**There is a single source of truth for access and feature logic: the
Web/backend.** Core never runs standalone, and no client is trusted to enforce
permissions on its own — a client may hide a button, but the API is what
actually says no. Any future component that needs to act on a service either
goes through the backend API or becomes part of the backend.
