-- A session started on behalf of another one remembers which, and how deep the
-- chain is, so a parent can be told when its child finishes, a stop can reach the
-- children, and the depth and number of children can be limited.
ALTER TABLE harness_sessions ADD COLUMN parent_session_id TEXT;
ALTER TABLE harness_sessions ADD COLUMN spawn_depth INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_harness_sessions_parent ON harness_sessions(parent_session_id)
    WHERE parent_session_id IS NOT NULL;
