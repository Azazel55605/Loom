-- A dashboard that exists and is reachable, but is not offered in a list.
--
-- Hiding is presentation, not access control: the row is still returned by
-- `GET /dashboards` with `hidden: true`, and `GET /dashboards/{id}` is
-- unaffected. It exists so a dashboard reached only by clicking a tile that
-- navigates to it does not also clutter the sidebar. Enforcing it server-side
-- would break exactly the case it was added for.
ALTER TABLE dashboards ADD COLUMN hidden BOOLEAN NOT NULL DEFAULT false;
