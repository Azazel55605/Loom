-- End-user dashboards and their resource-local sharing model.
--
-- This is intentionally separate from `group_permissions`: dashboard shares
-- answer who may view or edit one user-owned object, while RBAC grants answer
-- who may exercise an instance-wide capability. A row here never grants a
-- connector, user, or group permission.

CREATE TABLE dashboards (
    id            TEXT PRIMARY KEY NOT NULL,
    name          TEXT NOT NULL,
    owner_user_id TEXT NOT NULL REFERENCES users (id),
    created_at    TEXT NOT NULL
);

CREATE INDEX idx_dashboards_owner ON dashboards (owner_user_id);

CREATE TABLE dashboard_shares (
    id          TEXT PRIMARY KEY NOT NULL,
    dashboard_id TEXT NOT NULL REFERENCES dashboards (id) ON DELETE CASCADE,
    -- Polymorphic by design. Application code validates `target_id` against
    -- users or groups according to this discriminator before inserting.
    target_type TEXT NOT NULL CHECK (target_type IN ('user', 'group')),
    target_id   TEXT NOT NULL,
    role        TEXT NOT NULL CHECK (role IN ('view', 'edit')),
    created_at  TEXT NOT NULL,
    UNIQUE (dashboard_id, target_type, target_id)
);

CREATE INDEX idx_dashboard_shares_dashboard ON dashboard_shares (dashboard_id);
CREATE INDEX idx_dashboard_shares_target ON dashboard_shares (target_type, target_id);

CREATE TABLE dashboard_pins (
    user_id      TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    dashboard_id TEXT NOT NULL REFERENCES dashboards (id) ON DELETE CASCADE,
    pinned_at    TEXT NOT NULL,
    PRIMARY KEY (user_id, dashboard_id)
);

CREATE INDEX idx_dashboard_pins_dashboard ON dashboard_pins (dashboard_id);

CREATE TABLE dashboard_placements (
    id                    TEXT PRIMARY KEY NOT NULL,
    dashboard_id          TEXT NOT NULL REFERENCES dashboards (id) ON DELETE CASCADE,
    connector_instance_id TEXT NOT NULL REFERENCES connector_instances (id) ON DELETE CASCADE,
    position_x            INTEGER NOT NULL,
    position_y            INTEGER NOT NULL,
    width                 INTEGER NOT NULL CHECK (width > 0),
    height                INTEGER NOT NULL CHECK (height > 0),
    -- JSON-encoded Vec<WidgetBinding>, validated against the live connector
    -- before it reaches this table.
    widget_bindings       TEXT NOT NULL,
    created_at            TEXT NOT NULL
);

CREATE INDEX idx_dashboard_placements_dashboard
    ON dashboard_placements (dashboard_id);
CREATE INDEX idx_dashboard_placements_connector
    ON dashboard_placements (connector_instance_id);
