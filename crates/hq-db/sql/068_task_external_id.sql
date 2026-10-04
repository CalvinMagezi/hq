-- Caller-supplied idempotency key for task creation. The space is stored
-- beside it because a task reaches its space only through its initiative, and a
-- partial unique index cannot span a join. Both columns are set together.
ALTER TABLE tasks ADD COLUMN external_id TEXT;
ALTER TABLE tasks ADD COLUMN external_space_id TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS idx_tasks_external_id
    ON tasks(external_space_id, external_id) WHERE external_id IS NOT NULL;
