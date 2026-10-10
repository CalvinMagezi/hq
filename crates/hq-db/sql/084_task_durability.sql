-- Durability for long work. blocked_reason / waiting_on say why a task is stuck and
-- on what; both clear when it leaves blocked. long_horizon keeps a session's finished
-- turn or exit from moving the task. archived_at replaces deleting: an archived task
-- is hidden and can be restored. Checkpoints are what a session leaves for the next
-- one to resume from. task_audit outlives a purge, so a removal is always on record.
ALTER TABLE tasks ADD COLUMN blocked_reason TEXT;
ALTER TABLE tasks ADD COLUMN waiting_on TEXT;
ALTER TABLE tasks ADD COLUMN blocked_since TEXT;
ALTER TABLE tasks ADD COLUMN long_horizon INTEGER NOT NULL DEFAULT 0 CHECK (long_horizon IN (0, 1));
ALTER TABLE tasks ADD COLUMN archived_at TEXT;
CREATE INDEX IF NOT EXISTS idx_tasks_archived ON tasks(archived_at) WHERE archived_at IS NOT NULL;

CREATE TABLE IF NOT EXISTS task_checkpoints (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    work_session_id TEXT,
    actor TEXT NOT NULL,
    summary TEXT NOT NULL DEFAULT '',
    next_step TEXT NOT NULL DEFAULT '',
    open_questions TEXT NOT NULL DEFAULT '',
    files TEXT NOT NULL DEFAULT '[]',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_task_checkpoints_task ON task_checkpoints(task_id, id);

CREATE TABLE IF NOT EXISTS task_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    display_id TEXT NOT NULL,
    title TEXT NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('archived', 'restored', 'purged')),
    actor TEXT NOT NULL DEFAULT 'unknown',
    detail TEXT NOT NULL DEFAULT '',
    at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_task_audit_task ON task_audit(task_id, id);

-- A recurring watch can follow a task: each changed result is noted on it, and the
-- watch stops by itself once the task is complete or gone.
ALTER TABLE background_turns ADD COLUMN watch_task_id TEXT;
