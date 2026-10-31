-- The commit a session started at, kept apart from last_head (which moves at every finished
-- turn), so its worktree's commits since the start can be counted. Older rows take the last
-- commit seen, the closest thing they have.
ALTER TABLE session_checkouts ADD COLUMN start_head TEXT;
UPDATE session_checkouts SET start_head = last_head WHERE start_head IS NULL;
