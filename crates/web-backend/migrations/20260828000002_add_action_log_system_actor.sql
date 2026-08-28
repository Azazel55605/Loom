-- Lets the update scheduler own its invocations without impersonating anyone.
--
-- An automatic update is a real action with a real audit entry, but the actor
-- is not a person. Two options were available: a reserved "system" row in
-- `users`, or a flag on the log row. A reserved user row is a login-capable
-- account with a password hash and an active flag that every user-listing
-- query, every permission check, and every "delete this account" path would
-- then have to remember to exclude — a fake person is a special case in every
-- direction. A flag is a special case in one place, here, and it cannot be
-- logged into.
--
-- So `invoked_by_user_id` becomes nullable and `invoked_by_system` records the
-- other case, with a CHECK making the pair total: exactly one of the two
-- identifies the actor, always. A row that claims both or neither cannot be
-- written.
--
-- SQLite cannot relax NOT NULL in place, so this is the standard table rebuild.
-- The table is young and small; the copy is exact.

PRAGMA foreign_keys = OFF;

CREATE TABLE connector_action_log_new (
    id                 TEXT    PRIMARY KEY NOT NULL,
    instance_id        TEXT    NOT NULL REFERENCES connector_instances(id) ON DELETE CASCADE,
    action_id          TEXT    NOT NULL,
    target_id          TEXT,
    params             TEXT    NOT NULL,
    -- Nullable now, and still without an ON DELETE action: attribution that a
    -- later account deletion can rewrite is not an audit trail.
    invoked_by_user_id TEXT    REFERENCES users(id),
    -- 1 when Loom itself invoked the action — today, the update scheduler.
    invoked_by_system  INTEGER NOT NULL DEFAULT 0,
    invoked_at         TEXT    NOT NULL,
    completed_at       TEXT,
    success            INTEGER,
    result_message     TEXT,
    snapshot           TEXT,
    CHECK ((invoked_by_user_id IS NULL) = (invoked_by_system = 1))
);

INSERT INTO connector_action_log_new
    (id, instance_id, action_id, target_id, params, invoked_by_user_id,
     invoked_by_system, invoked_at, completed_at, success, result_message, snapshot)
SELECT id, instance_id, action_id, target_id, params, invoked_by_user_id,
       0, invoked_at, completed_at, success, result_message, snapshot
FROM connector_action_log;

DROP TABLE connector_action_log;

ALTER TABLE connector_action_log_new RENAME TO connector_action_log;

CREATE INDEX idx_connector_action_log_instance
    ON connector_action_log (instance_id, invoked_at DESC);

PRAGMA foreign_keys = ON;
