# ADR 0038: Personal dashboard folders and sidebar order

## Context

Dashboard ownership and sharing answer who may view or edit a dashboard. They
do not answer how each viewer wants to arrange an accessible dashboard in their
own sidebar. Storing a folder or list position on the dashboard itself would
make one person's organizational choice affect every recipient of a shared
dashboard.

Loom already keeps pins per user. Folders and manual order need the same
personal scope without becoming another authorization system.

## Decision

Dashboard folders belong to one user and have no sharing or permission model.
A separate sidebar-placement row relates one user to one accessible dashboard,
an optional folder, and a sort position. The existing dashboard ACL remains the
only authority for whether that user may see the dashboard; Viewer access is
enough to organize it.

Placement rows are lazy. Creating or sharing a dashboard does not backfill a
row for every viewer. Absence means ungrouped at a stable, name-ordered fallback
position, and a row is inserted only after that viewer moves or reorders the
dashboard.

Deleting a folder sets its placement rows' `folder_id` to null. It neither
deletes those rows nor changes the dashboards, shares, or access roles beneath
them. Folder ownership checks deliberately give the same answer for a missing
id and an id owned by someone else.

Batch reorder endpoints update a section in one transaction so a drag gesture
does not expose intermediate orderings or require one request per row.

## Consequences

- Two people can arrange the same shared dashboard independently.
- Sidebar organization grants no dashboard or connector authority.
- The common case creates no extra rows until organization is used.
- Clients must read `sidebarFolderId` and `sidebarSortOrder` from the dashboard
  list and fetch folders separately.
- A dashboard that becomes inaccessible may retain a placement row until its
  dashboard is deleted; it is never returned without current ACL access.

This is ADR 0038 rather than the originally proposed 0026 because ADR 0026 was
already assigned to group summaries and status cells.
