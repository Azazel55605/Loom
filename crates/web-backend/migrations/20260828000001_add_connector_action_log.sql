-- Every connector action anyone invokes, and what happened to it.
--
-- Generic audit infrastructure, not a feature of any one connector: the
-- action-execution endpoint writes here for every action on every instance,
-- which is what makes "who restarted the media server, and when?" answerable
-- without a connector having to opt in or a client having to ask for logging.
--
-- A row is written *before* the action is dispatched, with `completed_at`,
-- `success` and `result_message` NULL, and updated when it returns. A row that
-- stays pending is therefore meaningful in itself: the action was authorized
-- and dispatched, and Loom never learned the outcome — a restart that took the
-- process down with it looks exactly like that.
--
-- See docs/adr/0022-action-log-and-update-checking.md.

CREATE TABLE connector_action_log (
    id                 TEXT    PRIMARY KEY NOT NULL,
    instance_id        TEXT    NOT NULL REFERENCES connector_instances(id) ON DELETE CASCADE,
    action_id          TEXT    NOT NULL,
    -- The sub-target the action addressed, or NULL for the instance itself.
    target_id          TEXT,
    -- The parameters as submitted, JSON-encoded. Stored as sent rather than
    -- normalized: the point of an audit trail is what was asked for.
    params             TEXT    NOT NULL,
    -- No ON DELETE action, deliberately. Deleting a user who has invoked
    -- actions is refused rather than silently rewriting the history to say
    -- nobody did them; the delete-user route turns that refusal into an
    -- explanation. Attribution that a later delete can erase is not an audit
    -- trail.
    invoked_by_user_id TEXT    NOT NULL REFERENCES users(id),
    invoked_at         TEXT    NOT NULL,
    -- NULL until the action returns. See the pending-row note above.
    completed_at       TEXT,
    success            INTEGER,
    result_message     TEXT,
    -- Values of the action's declared `snapshotDataPointIds`, read from the
    -- poll cache just before dispatch and JSON-encoded as
    -- `{ "<dataPointId>": <value> }`. NULL when the action declared none.
    snapshot           TEXT
);

-- The one read this table has: an instance's history, newest first, optionally
-- narrowed by action. Ordering is part of the index because "newest first" is
-- not a refinement of the query, it is the query.
CREATE INDEX idx_connector_action_log_instance
    ON connector_action_log (instance_id, invoked_at DESC);
