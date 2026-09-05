# 0036 — Mobile kiosk mode uses an ordinary dedicated user

- Status: accepted
- Date: 2026-09-05

## Context

A tablet mounted on a wall is a useful way to keep selected Loom dashboards
visible and actionable. It is not equivalent to a personal device: it is
physically exposed, commonly left unlocked, and used by more than one person.
Giving it a normal administrator's session would turn convenient physical
access into broad homelab access.

Loom already has users, group-derived permissions, dashboard sharing, auditable
sessions, and revocation. A kiosk needs a constrained composition of those
features and a different mobile presentation, not another identity system.

## Decision

Kiosk mode is attached to a dedicated user in the existing multi-user system.
The `users.is_kiosk` flag is informational: it lets clients distinguish an
account intended for kiosk presentation, but never changes authorization.
Every capability still comes exclusively from ordinary group memberships and
dashboard access from ordinary viewer shares.

Administrators create this composition through the server-backed Kiosk Setup
wizard. It creates the user, assigns one selected group, and shares selected
dashboards with the user as a viewer. Viewer access controls dashboard layout;
the separate connector permissions in the chosen group continue to control
whether an action tile may act.

Mobile stores `kioskModeEnabled` and the expected kiosk user id in its existing
device-local Tauri Store. The toggle is offered only when `GET /account`
reports `isKiosk: true`; it is never synchronized and does not exist in Web or
Desktop. While enabled, Mobile bypasses `AppShell`, fetches the user's ordinary
dashboard list, and renders one full-screen `DashboardView` at a time. A simple
horizontal touch-start/touch-end threshold moves through that server-provided
order, with dot controls showing and selecting the current position. Dashboard
widgets remain ordinary interactive widgets.

An idle screensaver suitable for a continuously mounted display remains planned
for a later phase.

Exiting kiosk presentation is an authentication boundary, not a UI affordance.
It requires signing in as a different, non-kiosk account. A physically unlocked
kiosk must not be able to dismiss one screen and inherit broader controls.

The exit entry point is a three-second hold in the designated screen corner.
The resulting dialog authenticates credentials into an isolated in-memory token
store and resolves both `/auth/session` and `/account` before changing the
active session. Credentials for the current kiosk identity—or another kiosk
identity—are rejected. Their newly issued refresh token is revoked, their
tokens are never persisted, and the original kiosk session remains untouched.
Only a verified different, non-kiosk identity clears the local presentation
flag and replaces the stored Stronghold session.

Session recovery is deliberately a different screen and rule. If Stronghold's
stored kiosk session cannot be refreshed after restart, Mobile presents a
minimal kiosk re-entry screen rather than the normal login page or exit dialog.
It accepts only the exact kiosk user id saved when mode was enabled and confirms
that `/account` still marks it as kiosk. A different identity is discarded and
cannot turn recovery into an exit path.

Credentials use the same mobile Stronghold storage as every other session.
There is no kiosk-specific token or storage path. Kiosk sessions are ordinary
refresh-token sessions, so every device remains independently visible,
auditable, and revocable through the existing administrative Sessions panel
without new session machinery.

## Rejected alternative: Android device-owner or MDM provisioning

Android Enterprise device-owner or MDM enrollment would grant substantially
more operating-system control than Loom needs. It would also make compromise of
a kiosk administration path an OS-management problem. Loom kiosk mode remains
at normal application privilege: it changes presentation and uses server-side
permissions; it does not administer the device.

## Security considerations

Every future kiosk-related change must preserve this checklist:

- grant kiosk accounts the least privilege needed by their assigned dashboards;
- keep exit behind authentication as a real boundary, never a gesture alone;
- route future remote-management commands through the same privileged and
  audited server path as other administrative actions, never an out-of-band
  channel;
- request no elevated operating-system privileges;
- keep each device session independently visible and revocable;
- introduce no kiosk-specific credential-storage path.

## Consequences

Kiosk users can use every existing permission, dashboard, audit, and session
mechanism without special cases. Administrators must deliberately choose an
appropriately narrow group. The flag alone is not protective, and client-side
presentation gating is not authorization; the backend remains the enforcement
point. Presentation, swipe navigation, authenticated exit, and same-identity
recovery are implemented in Mobile; only the idle screensaver remains follow-up
work.
