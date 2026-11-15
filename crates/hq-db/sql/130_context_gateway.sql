-- A session secret may carry a deadline. NULL keeps the old behavior: it lives
-- as long as its session runs.
ALTER TABLE harness_session_tokens ADD COLUMN expires_at TEXT;

-- Which vault folders a task's sessions may read through the context gateway.
-- A task with no row here is unmarked and stays owner-only. allow_prefixes is a
-- JSON array of vault-relative folders.
CREATE TABLE IF NOT EXISTS task_context_scope (
    task_id TEXT PRIMARY KEY REFERENCES tasks(id),
    allow_prefixes TEXT NOT NULL DEFAULT '[]',
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
