-- A placement no longer has to name a connector instance.
--
-- Until now every tile on a dashboard was a view onto a connector, so
-- `connector_instance_id NOT NULL` was an accurate statement about the model.
-- Click behaviour makes it inaccurate: a tile whose whole purpose is to
-- navigate somewhere, or to fire one pre-configured action, has no data to
-- show and therefore no connector to read it from. Such a tile is still a
-- placement — same grid, same geometry, same grouping — with one column
-- absent, which is a smaller change than a second kind of tile row would be.
--
-- SQLite cannot relax a column's nullability in place, so this is the standard
-- rebuild: create the table as it should be, copy every row across, drop the
-- old one, rename the new one into place. Every other column, constraint,
-- foreign key and index below is reproduced exactly as it stands after
-- migrations 20260821000003, 20260823000002 and 20260827000002 — the single
-- intended difference is the missing NOT NULL on `connector_instance_id`.

CREATE TABLE dashboard_placements_rebuilt (
    id                    TEXT PRIMARY KEY NOT NULL,
    dashboard_id          TEXT NOT NULL REFERENCES dashboards (id) ON DELETE CASCADE,
    -- NULL for a placement that shows nothing and only acts: see
    -- `placement_action`, added by the migration that follows this one.
    connector_instance_id TEXT NULL REFERENCES connector_instances (id) ON DELETE CASCADE,
    position_x            INTEGER NOT NULL,
    position_y            INTEGER NOT NULL,
    width                 INTEGER NOT NULL CHECK (width > 0),
    height                INTEGER NOT NULL CHECK (height > 0),
    -- JSON-encoded Vec<WidgetBinding>, validated against the live connector
    -- before it reaches this table. An empty array for a connector-less tile,
    -- which has nothing to bind.
    widget_bindings       TEXT NOT NULL,
    created_at            TEXT NOT NULL,
    group_id              TEXT NULL REFERENCES dashboard_placement_groups (id),
    group_order           INTEGER NULL
        CHECK ((group_id IS NULL) = (group_order IS NULL)),
    target_id             TEXT
);

INSERT INTO dashboard_placements_rebuilt
    (id, dashboard_id, connector_instance_id, position_x, position_y, width, height,
     widget_bindings, created_at, group_id, group_order, target_id)
SELECT id, dashboard_id, connector_instance_id, position_x, position_y, width, height,
       widget_bindings, created_at, group_id, group_order, target_id
FROM dashboard_placements;

DROP TABLE dashboard_placements;

ALTER TABLE dashboard_placements_rebuilt RENAME TO dashboard_placements;

CREATE INDEX idx_dashboard_placements_dashboard
    ON dashboard_placements (dashboard_id);
CREATE INDEX idx_dashboard_placements_connector
    ON dashboard_placements (connector_instance_id);
CREATE UNIQUE INDEX idx_dashboard_placements_group_order
    ON dashboard_placements (group_id, group_order)
    WHERE group_id IS NOT NULL;
