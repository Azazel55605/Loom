-- Free-form labels assigned to connector instances by administrators.
--
-- There is deliberately no tags catalog table. The active vocabulary is the
-- distinct set present here, so deleting the final use of a tag also removes
-- it from autocomplete and filtering without orphan cleanup.

CREATE TABLE connector_instance_tags (
    instance_id TEXT NOT NULL REFERENCES connector_instances(id) ON DELETE CASCADE,
    tag         TEXT NOT NULL,
    PRIMARY KEY (instance_id, tag)
);

-- The primary key starts with instance_id and serves per-instance reads. The
-- reverse index serves the distinct, alphabetically sorted vocabulary query.
CREATE INDEX idx_connector_instance_tags_tag ON connector_instance_tags (tag);
