-- One row per context gateway operation: who asked, what kind of operation, how many sources
-- came back and how many were refused. Never the query, the paths or any note text.
CREATE TABLE IF NOT EXISTS context_gateway_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    task_id TEXT NOT NULL,
    mission_id TEXT,
    op TEXT NOT NULL,
    returned INTEGER NOT NULL,
    denied INTEGER NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_context_gateway_audit_task ON context_gateway_audit(task_id, id);
