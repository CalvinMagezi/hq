-- Usage read from the files coding agents write is keyed by a stable id from the agent, so reading
-- a growing file again never counts a call twice.
ALTER TABLE task_outcomes ADD COLUMN external_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_outcomes_external ON task_outcomes(external_id)
    WHERE external_id IS NOT NULL;

-- What the collector has already read: for a JSONL file its size and mtime, for a SQLite source the
-- last row id or timestamp it reached (in `size`).
CREATE TABLE IF NOT EXISTS harness_usage_files (
    path TEXT PRIMARY KEY,
    size INTEGER NOT NULL,
    mtime INTEGER NOT NULL
);
