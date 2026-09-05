-- `dashboards.manage`: instance-wide administration of every dashboard.
--
-- Ordinary dashboard editing remains governed by the dashboard-local
-- owner/editor/viewer ACL. This permission is deliberately separate: it is the
-- administrative escape hatch for renaming, reassigning, hiding, or deleting
-- dashboards regardless of their local membership.

INSERT INTO permissions (key, description) VALUES
    ('dashboards.manage', 'Administer every dashboard, including ownership.');

-- Administrators receives every instance-wide administrative capability.
INSERT INTO group_permissions (id, group_id, permission_key, resource_type, resource_id)
VALUES (
    lower(hex(randomblob(4))) || '-' || lower(hex(randomblob(2))) || '-4'
        || substr(lower(hex(randomblob(2))), 2) || '-a'
        || substr(lower(hex(randomblob(2))), 2) || '-'
        || lower(hex(randomblob(6))),
    '00000000-0000-4000-8000-000000000001',
    'dashboards.manage',
    NULL,
    NULL
);
