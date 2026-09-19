-- An explicit landing dashboard per user, overriding the heuristic the
-- dashboards index otherwise uses (first pinned, else first owned).
--
-- ON DELETE SET NULL is the whole reason this is a real foreign key rather than
-- a loose id: deleting a dashboard someone had set as their default reverts
-- that person to the heuristic instead of leaving them pointed at a row that no
-- longer exists. SQLite permits a REFERENCES clause on an added column only
-- when its default is NULL, which is exactly what is wanted here — every
-- existing user keeps the heuristic until they choose otherwise.
ALTER TABLE users ADD COLUMN default_dashboard_id TEXT NULL REFERENCES dashboards(id) ON DELETE SET NULL;
