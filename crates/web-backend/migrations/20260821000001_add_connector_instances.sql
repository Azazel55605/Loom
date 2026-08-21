-- Connector instances: one row per connector a user has actually added.
--
-- A connector is stored as a *type id plus an opaque JSON configuration*, not
-- as a table per connector kind. The alternative — a column per setting — would
-- mean a migration every time any connector gained an option, and the set of
-- options is defined by the connector's own JSON Schema rather than by this
-- schema. The backend never interprets `config`; it hands it to the connector
-- type's factory, which is the only thing that knows what the keys mean.
--
-- `connector_type` deliberately has **no foreign key**. The set of types is
-- compiled into the binary (see crates/web-backend/src/connectors/registry.rs)
-- because a registration carries a factory function, and code cannot live in a
-- row. There is therefore no table for this column to reference. A row whose
-- type is not registered in the running build is possible and is handled at
-- load time by skipping it with a warning, not by refusing to start.

CREATE TABLE connector_instances (
    id             TEXT PRIMARY KEY NOT NULL,
    connector_type TEXT NOT NULL,
    name           TEXT NOT NULL,
    -- JSON, validated by the connector's own factory before it is written.
    config         TEXT NOT NULL,
    created_at     TEXT NOT NULL
);

-- Listing is ordered by name, and the startup load groups by type.
CREATE INDEX idx_connector_instances_name ON connector_instances (name);
CREATE INDEX idx_connector_instances_type ON connector_instances (connector_type);
