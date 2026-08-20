-- Self-service profile fields on `users`.
--
-- Both nullable, and both absent for every account created before this
-- migration: a display name is optional by design (the username is always
-- there to fall back on), and an avatar is something a user opts into.
--
-- `avatar_path` holds a path **relative to the data directory** — for example
-- `avatars/2f1c….png` — not an absolute filesystem path. Storing an absolute
-- path would bake the deployment's layout into the database, so moving the data
-- directory, or mounting the same volume at a different point in a container,
-- would silently break every avatar reference. A relative path stays correct
-- wherever LOOM_DATA_DIR points.
ALTER TABLE users ADD COLUMN display_name TEXT;
ALTER TABLE users ADD COLUMN avatar_path TEXT;
