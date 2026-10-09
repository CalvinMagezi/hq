-- Typed links from a task to what it came from or produced: a vault note, a chat
-- thread, an agent session, a commit, a pull request, a URL or another task.
-- `ref` is normalised per kind before it is stored, so one thing is one row and
-- "which tasks mention this note" is an index lookup. Replaces the free-text
-- "Source:" line as the thing code reads; that line stays for people.
CREATE TABLE IF NOT EXISTS task_links (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    kind TEXT NOT NULL CHECK (kind IN
        ('vault_note', 'chat_thread', 'session', 'commit', 'pr', 'url', 'task')),
    ref TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    direction TEXT NOT NULL DEFAULT 'related'
        CHECK (direction IN ('origin', 'related', 'produced')),
    created_by TEXT NOT NULL DEFAULT 'unknown',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (task_id, kind, ref)
);
CREATE INDEX IF NOT EXISTS idx_task_links_ref ON task_links(kind, ref);
