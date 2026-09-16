-- A host-level media-player placement owns its device selection. The JSON
-- array is deliberately placement state rather than connector configuration:
-- two dashboards may present different groups from the same connector.
ALTER TABLE dashboard_placements
    ADD COLUMN selected_target_ids TEXT NOT NULL DEFAULT '[]';
