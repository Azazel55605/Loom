-- Mark groups that must not be deleted.
--
-- The Administrators group is the only way an instance grants `users.manage`
-- and `groups.manage` out of the box, so deleting it can strand an instance
-- with no route back to administering itself. That has to be refused.
--
-- A flag rather than matching on the name: a group can be renamed, and a check
-- comparing against the literal string 'Administrators' silently stops
-- protecting it the moment someone does. Matching on the seeded id would
-- survive a rename but hardcodes one id in application code and cannot express
-- "this other group is also protected" later. A column says what is actually
-- meant.
--
-- The flag guards *deletion*, not editing. An administrator may still rename
-- the group or change its grants; the last-administrator safeguard in the user
-- endpoints is what stops the instance losing its final admin.

ALTER TABLE groups ADD COLUMN is_protected BOOLEAN NOT NULL DEFAULT FALSE;

UPDATE groups
SET is_protected = TRUE
WHERE id = '00000000-0000-4000-8000-000000000001';
