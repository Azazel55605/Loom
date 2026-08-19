-- Initial schema: users, groups, and resource-scoped permission grants.
--
-- Conventions used throughout:
--
--   * Ids are TEXT holding a UUID v4. Random rather than sequential so an id
--     leaks neither creation order nor how many rows exist.
--   * Timestamps are TEXT holding RFC 3339 in UTC. SQLite has no native date
--     type; storing a fixed-width, lexicographically sortable UTC string means
--     ORDER BY and range comparisons work on the raw column.
--   * Foreign keys use ON DELETE CASCADE where the child row is meaningless
--     without its parent. Note that SQLite only enforces foreign keys when
--     `PRAGMA foreign_keys = ON`, which the connection pool sets explicitly —
--     the default is OFF, and silently so.

CREATE TABLE users (
    id            TEXT    PRIMARY KEY NOT NULL,
    username      TEXT    UNIQUE NOT NULL,
    -- A PHC-format argon2id string. The parameters travel inside the hash, so
    -- raising the cost later does not invalidate existing hashes: they keep
    -- verifying under the parameters they were created with.
    password_hash TEXT    NOT NULL,
    -- Deactivation rather than deletion. Removing a user would cascade away
    -- their group memberships and refresh tokens, which is exactly what you do
    -- not want when disabling an account you may need to audit or restore.
    is_active     BOOLEAN NOT NULL DEFAULT TRUE,
    created_at    TEXT    NOT NULL
);

CREATE TABLE groups (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT UNIQUE NOT NULL,
    description TEXT,
    created_at  TEXT NOT NULL
);

-- Membership. Permissions are granted to groups and never directly to users,
-- so "what can this person do" always has one answer path: their groups.
CREATE TABLE user_groups (
    user_id  TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    group_id TEXT NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, group_id)
);

CREATE INDEX idx_user_groups_group ON user_groups (group_id);

-- The authoritative set of permissions the application understands.
--
-- IMPORTANT: this table is the source of truth. A permission key used anywhere
-- in the codebase MUST have a row here, added by a new migration — not by an
-- INSERT at runtime and not by a constant that exists only in Rust. The foreign
-- key from `group_permissions` enforces this: granting an unregistered key
-- fails loudly instead of creating a grant that silently matches nothing.
--
-- Adding a permission is therefore a schema change, deliberately. It makes the
-- full set auditable from the migration history alone.
CREATE TABLE permissions (
    key         TEXT PRIMARY KEY NOT NULL,
    description TEXT NOT NULL
);

INSERT INTO permissions (key, description) VALUES
    ('connectors.view',    'View connectors and their status.'),
    ('connectors.control', 'Execute actions on connectors.'),
    ('users.manage',       'Create, modify, and deactivate user accounts.'),
    ('groups.manage',      'Create and modify groups and their permission grants.'),
    ('system.settings',    'Change instance-wide settings.');

-- A permission granted to a group, optionally narrowed to one resource.
--
-- This is what makes permissions resource-scoped rather than flat:
--
--   resource_type NULL, resource_id NULL  -> global grant, every resource
--   resource_type SET,  resource_id NULL  -> every resource of that type
--   resource_type SET,  resource_id SET   -> exactly that one resource
--
-- So "may control every connector" and "may control only the media server" are
-- the same permission with different scope, rather than two permissions. A flat
-- role model cannot express the second without inventing a key per resource.
--
-- Enforcement — matching a request against these rows — is deliberately not
-- implemented yet; it lands with the authorization middleware. What exists now
-- is the storage shape and the effective-permission query that reads it, so the
-- claims clients receive are already the real thing.
CREATE TABLE group_permissions (
    id             TEXT PRIMARY KEY NOT NULL,
    group_id       TEXT NOT NULL REFERENCES groups (id) ON DELETE CASCADE,
    permission_key TEXT NOT NULL REFERENCES permissions (key),
    resource_type  TEXT,
    resource_id    TEXT
);

CREATE INDEX idx_group_permissions_group ON group_permissions (group_id);

-- Unique per (group, permission, scope), so the same grant cannot be recorded
-- twice. SQLite treats NULLs as distinct in a UNIQUE index, which would let
-- duplicate global grants through, so the scope columns are coalesced to a
-- sentinel that cannot collide with a real value.
CREATE UNIQUE INDEX idx_group_permissions_unique ON group_permissions (
    group_id,
    permission_key,
    COALESCE(resource_type, ''),
    COALESCE(resource_id, '')
);

-- Opaque, revocable session tokens.
--
-- Only a hash of the token is stored, for the same reason passwords are hashed:
-- a leaked database must not hand over usable credentials. A refresh token is a
-- bearer credential with a seven-day life, so a readable one is worth as much
-- to an attacker as a password.
--
-- Unlike a password this is a high-entropy random value, so a plain SHA-256 is
-- the right primitive — there is nothing to brute-force, and argon2 here would
-- add latency to every refresh for no gain.
CREATE TABLE refresh_tokens (
    id         TEXT PRIMARY KEY NOT NULL,
    user_id    TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash TEXT UNIQUE NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    -- NULL means live. Set on logout and on rotation, so a used token cannot be
    -- replayed.
    revoked_at TEXT
);

CREATE INDEX idx_refresh_tokens_user ON refresh_tokens (user_id);

-- Instance-level key/value settings that must survive a restart.
--
-- Holds the JWT signing secret, generated on first boot rather than supplied by
-- configuration: ADR 0004 requires the server to start with no environment at
-- all, and a signing secret that changed on every restart would invalidate
-- every outstanding access token on each deploy.
CREATE TABLE server_config (
    key        TEXT PRIMARY KEY NOT NULL,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
