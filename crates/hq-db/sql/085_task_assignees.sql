-- Who a task is for, apart from what it is about. A tag says what a task concerns; an
-- assignee says which agent or person should do it. Notifications and "my queue" use
-- assignees; tags stay topical. A task can have several.
CREATE TABLE IF NOT EXISTS task_assignees (
    task_id TEXT NOT NULL REFERENCES tasks(id),
    assignee TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (task_id, assignee)
);
CREATE INDEX IF NOT EXISTS idx_task_assignees_assignee ON task_assignees(assignee);
