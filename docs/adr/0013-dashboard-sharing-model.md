# 0013 — Dashboards use a resource-local owner/editor/viewer ACL

- Status: accepted
- Date: 2026-08-21

## Context

Loom already has group-based permission grants for administrative and service
capabilities: managing users, configuring connectors, or invoking connector
actions. Those grants are issued by administrators and carried in access-token
claims. They answer "what system capability may this person exercise?"

Dashboard sharing answers a different question: "what may this person do with
this user-owned object?" The owner needs to share one dashboard with another
user or a group without an administrator inventing a permission key or editing
the instance-wide grant model. Treating shares as `group_permissions` would mix
end-user collaboration with administrative authority and make ordinary sharing
capable of widening service permissions by accident.

Dashboard visibility and connector authority also do not naturally coincide. A
viewer may see a status widget without being allowed to press its restart
button; someone allowed to restart a connector does not thereby gain access to
every dashboard that mentions it.

## Decision

Dashboards use a dedicated database-backed ACL with three ordered roles:

1. `owner` — the creating user; may view, edit placements, rename, delete, and
   manage shares.
2. `editor` — a `role = 'edit'` share; may view and edit placements.
3. `viewer` — a `role = 'view'` share; may view and pin for themselves.

Every dashboard has exactly one owner in `dashboards.owner_user_id`. Shares may
target either a user id or a group id. The polymorphic target cannot be a true
foreign key, so the application validates the corresponding user/group exists
before insertion. A dashboard may have at most one share per target.

Effective role resolution checks ownership first, then combines direct-user
and group-target shares through `user_groups`, selecting the highest applicable
role. Roles are read from the database for each operation rather than copied
into JWT claims, so a share or revocation takes effect immediately. A caller
with no effective role receives 403, including when probing a nonexistent id;
private dashboard existence is not disclosed through role-gated paths.

Pins are per-user rows and require at least Viewer. They are presentation state,
not an ACL entry, and one user's pin cannot affect another user.

Placements reference existing connector instances and store validated Core
`WidgetBinding` values. Editors and owners may change them. Placement creation
checks the live connector's minimum size and, per
[0014](./0014-widget-binding-model.md), resolves each binding against the
namespace its tag names — a `display` binding against the connector's data
points, an `action` binding against its actions. It performs no connector
authorization and grants none: a viewer may see an action widget and still be
refused when they press it.

The dashboard ACL and the existing RBAC system remain deliberately orthogonal:

- A share never grants `connectors.view`, `connectors.control`, or an
  administrative permission.
- A connector or administrative grant never creates a dashboard role.
- Connector actions exposed by a placement continue to call the existing
  action endpoint, which evaluates the current viewer's own
  `connectors.control` grant.

## Consequences

- End users can share individual dashboards without administrator involvement
  or permission-catalog migrations.
- The same group can be both an RBAC grant holder and a dashboard-share target,
  but those rows have no authorization effect on one another.
- Removing a user from a shared group removes their dashboard access on the
  next request. Removing a direct share is equally immediate.
- Deleting a dashboard cascades its shares, pins, and placements. Deleting a
  connector instance cascades only placements that reference it, not the
  dashboards containing them.
- Deleting a user who still owns dashboards is refused with 409. Dashboard
  ownership does not cascade through account deletion because that would turn
  an administrative action into silent user-content loss.
- `dashboard_shares.target_id` cannot have a database foreign key. Stale targets
  are prevented on write, but deleting a user or group may leave a share row
  whose target no longer resolves. A future audit/history policy must decide
  whether those rows should be automatically cleaned up or retained as revoked
  history.
- Both ACL failures and RBAC failures return 403 because the caller is
  authenticated in either case. Documentation and error messages identify
  which authorization system denied the operation.
