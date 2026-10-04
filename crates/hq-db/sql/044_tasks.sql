-- Native task management (replaces ClickUp). See docs/plans/native-tasks
-- for the design: spaces > initiatives > tasks > comments, tags as a join
-- table (the primary filter/routing dimension), display_id globally unique
-- so a bare "FR-027" resolves unambiguously.

CREATE TABLE IF NOT EXISTS spaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS initiatives (
    id TEXT PRIMARY KEY,
    space_id TEXT NOT NULL REFERENCES spaces(id),
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    id_prefix TEXT NOT NULL UNIQUE,
    next_sequence INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(space_id, slug)
);

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    initiative_id TEXT NOT NULL REFERENCES initiatives(id),
    display_id TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    status TEXT NOT NULL DEFAULT 'to_do',
    priority TEXT,
    due_date TEXT,
    clickup_task_id TEXT UNIQUE,
    created_by TEXT NOT NULL DEFAULT 'unknown',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS task_tags (
    task_id TEXT NOT NULL REFERENCES tasks(id),
    tag TEXT NOT NULL,
    PRIMARY KEY (task_id, tag)
);
CREATE INDEX IF NOT EXISTS idx_task_tags_tag ON task_tags(tag);

CREATE TABLE IF NOT EXISTS task_comments (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    author TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_tasks_initiative ON tasks(initiative_id, status);
CREATE INDEX IF NOT EXISTS idx_tasks_status ON tasks(status);
CREATE INDEX IF NOT EXISTS idx_task_comments_task ON task_comments(task_id, created_at);

INSERT OR IGNORE INTO spaces (id, name, slug) VALUES
    ('personal', 'Personal', 'personal'),
    ('professional', 'Professional', 'professional');
