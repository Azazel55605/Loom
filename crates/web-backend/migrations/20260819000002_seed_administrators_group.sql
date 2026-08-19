-- The Administrators group: one global grant of every registered permission.
--
-- Seeded by migration rather than created during setup so that the first
-- administrator is assigned to a group that already exists with a known id,
-- and so the grant set is visible in the migration history rather than being
-- assembled by application code at runtime.
--
-- Fixed UUID, not a random one: the setup handler needs to reference this group
-- without a name lookup, and a stable id means that reference cannot break if
-- the group is ever renamed.
--
-- NOTE: this is not self-maintaining. A migration that adds a new permission
-- key must also decide whether Administrators receives it — adding a row to
-- `permissions` alone leaves this group without the new grant.

INSERT INTO groups (id, name, description, created_at) VALUES (
    '00000000-0000-4000-8000-000000000001',
    'Administrators',
    'Full access to every permission across every resource.',
    '2026-08-19T00:00:00Z'
);

-- One global grant per registered permission: NULL resource_type and
-- resource_id together mean "every resource, of every type".
INSERT INTO group_permissions (id, group_id, permission_key, resource_type, resource_id)
SELECT
    lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4'
        || substr(lower(hex(randomblob(2))), 2) || '-a'
        || substr(lower(hex(randomblob(2))), 2) || '-'
        || lower(hex(randomblob(6))),
    '00000000-0000-4000-8000-000000000001',
    key,
    NULL,
    NULL
FROM permissions;
