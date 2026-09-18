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

Idle presentation splits configuration by ownership. Administrators select an
ordered `screensaverConfig` of connector-instance, target, and data-point
references on the kiosk user; the backend validates those references through
the same live-descriptor path as dashboard display bindings. Each mobile device
separately stores whether its screensaver is enabled and its idle timeout in
Tauri Store, because two tablets using the same account can reasonably have
different wake/sleep expectations.

The kiosk shell records global pointer/touch activity and compares the last
interaction timestamp with that device-local timeout. Once idle it replaces the
dashboard tree with a dedicated ambient view: a large clock and date plus one
configured live reading at a time, rotating every eight seconds. A wake tap
only resumes the already-authorized dashboard session; it never crosses or
weakens the separately authenticated exit boundary. A faint corner button
starts the same ambient view immediately, for someone who wants the display
dark now rather than after the timeout; it is placed in the opposite corner
from the exit hold so a tap and a three-second hold cannot be confused. Reading transitions are
instant when either Loom or the operating system requests reduced motion.

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

### Immersive presentation and adjacent-dashboard preloading

Kiosk presentation hides the Android status and navigation bars while it is
active, and restores them the moment it ends. Tauri 2 offers no way to ask for
this: its window API, `setFullscreen` included, is desktop-only, and hiding the
system bars on Android remains an open request against the framework. No
first-party plugin covers it either — the community plugins in this area manage
the status bar's appearance rather than an immersive window, and adding one
would be a dependency for a single call.

Loom therefore makes the documented platform calls itself, over JNI from the
mobile crate: `WindowInsetsController.hide(systemBars())` with
`BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE` on API 30 and above, and the deprecated
`setSystemUiVisibility` sticky-immersive flags below it. It is exposed as one
command, `set_immersive_mode`, driven by the device-local kiosk flag, so the
bars are hidden for dashboard browsing and the screensaver alike and return on
the authenticated exit. The JNI environment and the Activity come from Tauri's
own webview handle, `PlatformWebview::jni_handle`, which runs the closure on the
thread owning the webview. They must not come from `ndk_context`: nothing in
Tauri's Android stack initializes that context, and its accessor panics when
uninitialized — on the UI thread, which ends the process rather than failing the
call. The calls live in Rust rather than in `MainActivity`
because `gen/android` is generated and untracked — Kotlin added there would
have to be re-applied by the configure script on every machine and would still
need a bridge for the frontend to toggle it. This changes window decoration
only: back gestures are still dispatched to the activity and still reach the
existing back handler. It also requests no elevated operating-system privilege,
so the checklist above is unaffected.

With the bars gone, kiosk chrome reads `env(safe-area-inset-*)` directly
instead of the app-wide insets, which floor at a touch-safe minimum for screens
that keep their system bars. The rotation selector then sits against the true
bottom edge when immersive mode is in effect and against the real inset when it
is not, with no fixed assumption either way.

Swipe navigation prefetches the structure query of the next and previous
dashboards in the rotation once the visible one has settled, so an arriving
dashboard is already laid out instead of showing a skeleton. Deliberately only
that query: a prefetch fills the cache without mounting a view, and mounting is
what subscribes a dashboard's connector instances to the status socket.
Subscribing adjacent dashboards ahead of time would put many instances on the
socket for readings nobody is looking at. Live status stays the business of the
visible dashboard.

### Home-app (launcher) registration

Mobile's Activity declares a second intent filter — `MAIN` + `HOME` +
`DEFAULT` — which makes Loom *selectable* in Android's home-app chooser. It
does not make Loom the launcher: Android owns the home role, the user grants it
explicitly through the system's own dialog or settings screen, and it is
revocable from Settings at any time. That is the distinction this ADR's
least-privilege position rests on. A launcher is an ordinary, user-granted,
user-revocable app capability; it administers nothing, survives no factory
policy, and grants Loom no authority over the device or over any other app.
Device-owner and MDM provisioning, rejected above, remain rejected: nothing
here requests an elevated operating-system privilege, and Loom still holds only
normal application permissions.

Tauri's configuration has no manifest customization — its Android config covers
`minSdkVersion` and the version code and nothing else — so the filter is
applied by `scripts/configure-mobile-android.mjs` alongside the network-security
policy, launcher icons and orientation already patched there, idempotently,
because `gen/android` is generated and untracked. The generated activity is
already `launchMode="singleTask"`, which is what a home app needs so Home does
not stack instances; the script adds `stateNotNeeded="true"` for the same
reason.

Being the home app and being a kiosk are independent settings that compose.
Kiosk mode decides how Loom presents dashboards once open; the home role
decides what opens when the Home button is pressed. Either is useful alone, and
a wall-mounted tablet typically wants both.

Back behaviour is the one thing that changes with the home role, and it changes
only at the last tier of the back-handler stack. A launch is classified per
instance by asking the Activity's own start Intent whether it carried
`CATEGORY_HOME`, which is what the system's home intent sets and what tapping
the app icon (`CATEGORY_LAUNCHER`) does not. Nothing is persisted: the same
installation is the home screen on one start and an ordinary app on the next,
and only the starting Intent knows which. When this instance is the home
screen, back at the true root does nothing, matching every other Android home
screen — there is nowhere behind the home screen, and quitting would leave the
device with no home app on screen. When it is not, the existing quit
confirmation is unchanged. The overlay-dismissal and router-history tiers above
it, and the authenticated kiosk exit gesture, behave identically either way.

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
point. Kiosk-user setup, presentation and swipe navigation, authenticated exit,
same-identity recovery, and the idle screensaver are implemented. This ADR's
planned kiosk phases are complete.
