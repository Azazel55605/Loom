-- `connectors.manage`: may add, reconfigure, and remove connector instances.
--
-- Split from the two keys that already existed because they answer a different
-- question. `connectors.view` and `connectors.control` are about *a connector
-- that exists*: may you see it, may you press its buttons — and both are
-- routinely granted scoped to a single connector. Adding or deleting an
-- instance is not a thing that can be scoped to a connector, because the
-- connector is what is being created or destroyed; it is authority over the
-- instance list itself. Folding it into `connectors.control` would mean anyone
-- allowed to restart one service could also delete every connector on the
-- instance, which is not what granting a restart button is meant to say.
--
-- Registered here rather than at runtime because `group_permissions` has a
-- foreign key onto this table: a key that is not registered fails loudly
-- instead of creating a grant that silently matches nothing.

INSERT INTO permissions (key, description) VALUES
    ('connectors.manage', 'Add, reconfigure, and remove connector instances.');

-- Administrators receives it, matching the "full access to every permission"
-- promise in that group's description. The seed migration is explicit that it
-- is not self-maintaining: a new permission key must decide this for itself,
-- and this one decides yes.
INSERT INTO group_permissions (id, group_id, permission_key, resource_type, resource_id)
VALUES (
    lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4'
        || substr(lower(hex(randomblob(2))), 2) || '-a'
        || substr(lower(hex(randomblob(2))), 2) || '-'
        || lower(hex(randomblob(6))),
    '00000000-0000-4000-8000-000000000001',
    'connectors.manage',
    NULL,
    NULL
);
