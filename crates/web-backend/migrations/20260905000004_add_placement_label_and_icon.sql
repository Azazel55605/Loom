-- Display metadata for a placement that has no connector to borrow it from.
--
-- A connector-backed tile takes its name and icon from the instance it shows,
-- which is why neither column existed until now. A static tile shows nothing,
-- so the two things a user needs to recognise it — what it says and what it
-- looks like — have nowhere else to come from.
--
-- Both stay NULL for every connector-backed placement, and the create/update
-- endpoints refuse to set them there rather than storing a second name that
-- silently disagrees with the connector's own.
--
-- `icon` follows the same `lucide:`/`brand:` reference convention as
-- `connector_instances.icon_override` and `dashboard_placement_groups.icon`; an
-- unresolvable reference falls back in the client rather than failing here.
ALTER TABLE dashboard_placements ADD COLUMN label TEXT NULL;
ALTER TABLE dashboard_placements ADD COLUMN icon TEXT NULL;
