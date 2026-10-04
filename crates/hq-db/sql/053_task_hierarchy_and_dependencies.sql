-- Sub-tasks (one level, enforced in hq_db::tasks), start dates for timeline
-- views, and finish-to-start dependencies. Purely additive: existing tasks
-- get NULL parent/start_date and no dependency rows.

ALTER TABLE tasks ADD COLUMN parent_task_id TEXT REFERENCES tasks(id);
ALTER TABLE tasks ADD COLUMN start_date TEXT;
CREATE INDEX IF NOT EXISTS idx_tasks_parent ON tasks(parent_task_id);

CREATE TABLE IF NOT EXISTS task_dependencies (
    task_id TEXT NOT NULL REFERENCES tasks(id),
    depends_on_task_id TEXT NOT NULL REFERENCES tasks(id),
    created_by TEXT NOT NULL DEFAULT 'unknown',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (task_id, depends_on_task_id),
    CHECK (task_id <> depends_on_task_id)
);
CREATE INDEX IF NOT EXISTS idx_task_deps_blocker ON task_dependencies(depends_on_task_id);
