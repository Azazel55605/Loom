-- Combining several placements into one wider dashboard tile.
--
-- A group is its own entity rather than a "parent placement" that other
-- placements point at. A parent-placement model would make every placement
-- row ambiguous — is this a real connector tile or a container? — and would
-- put a connector instance, widget bindings, and a container's geometry in one
-- row where two of the three are meaningless at any given time. A group has no
-- connector and no bindings; it has a bounding box and an ordered membership,
-- and that is all it ever has. See docs/adr/0015-dashboard-tile-grouping.md.
--
-- Nothing here is pairwise. A group holds any number of members from two
-- upward, of any connector types, in any mix.

CREATE TABLE dashboard_placement_groups (
    id           TEXT PRIMARY KEY NOT NULL,
    dashboard_id TEXT NOT NULL REFERENCES dashboards (id) ON DELETE CASCADE,
    -- The tile's own footprint on the dashboard grid. While a placement is a
    -- member, *this* box is what the grid lays out; the member's own
    -- position/size columns are not consulted.
    position_x   INTEGER NOT NULL,
    position_y   INTEGER NOT NULL,
    width        INTEGER NOT NULL CHECK (width > 0),
    height       INTEGER NOT NULL CHECK (height > 0),
    created_at   TEXT NOT NULL
);

CREATE INDEX idx_dashboard_placement_groups_dashboard
    ON dashboard_placement_groups (dashboard_id);

-- Membership lives on the placement, not in a join table: a placement is in at
-- most one group at a time, so a join table would model a many-to-many that is
-- forbidden and would need a unique constraint to forbid it again.
--
-- No `ON DELETE` action on purpose. Deleting a group whose members still point
-- at it fails loudly instead of silently orphaning them, which forces every
-- dissolve path to clear membership first — exactly the invariant the
-- auto-dissolve rule depends on. (A dashboard's deletion still cascades
-- cleanly: its placements and its groups are both removed by that cascade.)
ALTER TABLE dashboard_placements
    ADD COLUMN group_id TEXT NULL REFERENCES dashboard_placement_groups (id);

-- Sort key within the group, not an array index: removing a member leaves a
-- gap, and a gap is fine because only the relative order is read.
--
-- The CHECK makes "grouped" a single fact rather than two that can disagree.
-- Half-applied membership — a group id with no order, or an order with no group
-- — would sort nondeterministically and would be invisible until it did.
ALTER TABLE dashboard_placements
    ADD COLUMN group_order INTEGER NULL
        CHECK ((group_id IS NULL) = (group_order IS NULL));

-- Partial, so the many standalone placements (both columns NULL) are exempt.
-- Two members of one group at the same position is not an ordering.
CREATE UNIQUE INDEX idx_dashboard_placements_group_order
    ON dashboard_placements (group_id, group_order)
    WHERE group_id IS NOT NULL;

-- ---------------------------------------------------------------------------
-- Why a member keeps its own position_x/position_y/width/height
-- ---------------------------------------------------------------------------
--
-- Those four columns are **retained and ignored** while `group_id` is set. The
-- group's bounding box governs grid placement; the member's own geometry is
-- simply not read by the renderer.
--
-- They are not cleared, and that is the entire mechanism that makes ungrouping
-- lossless. Ungrouping is a write of NULL to two columns: every placement
-- immediately renders standalone again, exactly where and at what size it was
-- before it was grouped. Clearing the geometry on grouping would mean
-- inventing a position at ungroup time, and a dashboard that rearranges itself
-- because a user tried a grouping and changed their mind is a dashboard people
-- stop experimenting with.
--
-- The consequence to keep in mind: those columns are *stale-by-design* for a
-- grouped placement. `PATCH .../placements/{id}` still writes them, and what it
-- is editing is the geometry the placement will return to.
