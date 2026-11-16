-- One row per source a context gateway call returned: which task and session, which operation,
-- which source id and its content hash. Source ids and hashes only, never note text or the query,
-- so the owner can see what a session was shown and a resumed session can fetch it again.
CREATE TABLE IF NOT EXISTS context_gateway_trail (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    op TEXT NOT NULL,
    source_id TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_context_gateway_trail_task ON context_gateway_trail(task_id, id);
