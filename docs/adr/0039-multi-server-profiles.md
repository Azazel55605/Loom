# 0039. Server profiles are device-local and credentials are profile-scoped

- Status: accepted
- Date: 2026-09-10

## Context

Desktop and Mobile originally persisted one server URL and one set of tokens.
That made changing servers destructive: the new URL reused credentials issued
by the previous server until the ordinary authentication flow replaced them.
It also left no way to retain several homelab environments on one device.

The two clients already had appropriate but different secure stores. Desktop
uses the operating system credential store through `tauri-plugin-keyring-store-api`;
Mobile uses Tauri Stronghold. Their non-sensitive connection settings are kept
in Tauri Store. Replacing either mechanism would add migration risk without
improving the profile model.

## Decision

`@loom/ui-kit` defines `ServerProfile`, `ServerProfileManager`, and the shared
persistence rules. A profile contains only an id, label, base URL, and last-used
timestamp. Profiles and the active profile id are device-local Tauri Store data;
they are not synced through the Loom server.

Each platform instantiates the same persistence engine over its existing Store.
The base URL adapter resolves the currently active profile, keeping the API
client unaware of profile management. Certificate policy is also stored per
profile because it describes trust for one server rather than a global app
preference.

Authentication tokens never enter a profile or Tauri Store. The shared token
adapter namespaces secure-store records as `<profile-id>:auth.tokens`, backed by
the Desktop OS credential store or Mobile Stronghold. Switching profiles
therefore cannot expose one server's tokens to another. Removing a profile
deletes only that profile's secure credential. Removing the active profile
selects the remaining profile with the most recent `lastUsedAt`, or leaves no
active profile when none remain.

Existing installations migrate once on first access. A populated legacy
`serverUrl` creates one active profile whose label is derived from its host. The
legacy certificate setting is assigned to that profile. The old unscoped
`auth.tokens` credential is moved into that profile's namespaced record on first
read. If the migrated profile is removed before that read, the old credential is
deleted instead. A persisted migration marker prevents deletion of all profiles
from causing the legacy values to be imported again.

Changing the active profile is deliberately only a persistence operation. The
future profile-selection UI must then clear profile-derived client state,
recreate or invalidate the React Query cache, and run the existing connection
bootstrap for the selected profile. Keeping that orchestration outside the
manager makes the storage boundary deterministic and avoids coupling a shared
adapter to React or routing.

## Consequences

Desktop and Mobile can retain multiple independently authenticated server
connections without duplicating profile rules. Web remains unchanged because
its server URL is deployment configuration rather than device-local user
selection.

Profile labels and URLs are not secret and remain inspectable in Tauri Store.
Tokens retain the security properties of each platform's existing backend. A
profile switch is not itself proof that the target is reachable or that its
session is valid; the normal bootstrap owns those checks.

This ADR is numbered 0039 because 0027 was already assigned to Docker stacks
before the multi-server work was introduced.
