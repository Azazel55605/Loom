-- User-facing identity for a combined dashboard tile.
--
-- Existing groups receive the neutral name "Group" and the shared client
-- supplies the generic boxes icon when `icon` is NULL. New groups get a more
-- descriptive generated name from the create handler and can be renamed or
-- assigned another generic icon through PATCH.

ALTER TABLE dashboard_placement_groups
    ADD COLUMN name TEXT NOT NULL DEFAULT 'Group';

ALTER TABLE dashboard_placement_groups
    ADD COLUMN icon TEXT NULL;
