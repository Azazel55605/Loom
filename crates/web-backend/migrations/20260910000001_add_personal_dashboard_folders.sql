-- Personal sidebar organization layered on top of dashboard access.
--
-- A folder belongs to one viewer, never to a dashboard. Shared dashboards can
-- therefore be organized differently by every person who can see them.

CREATE TABLE dashboard_folders (
    id         TEXT PRIMARY KEY NOT NULL,
    user_id    TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name       TEXT NOT NULL,
    sort_order INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX idx_dashboard_folders_user_order
    ON dashboard_folders (user_id, sort_order, name, id);

CREATE TABLE dashboard_sidebar_placements (
    user_id     TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    dashboard_id TEXT NOT NULL REFERENCES dashboards (id) ON DELETE CASCADE,
    folder_id   TEXT REFERENCES dashboard_folders (id) ON DELETE SET NULL,
    sort_order  INTEGER NOT NULL,
    PRIMARY KEY (user_id, dashboard_id)
);

CREATE INDEX idx_dashboard_sidebar_placements_user_folder_order
    ON dashboard_sidebar_placements (user_id, folder_id, sort_order, dashboard_id);
