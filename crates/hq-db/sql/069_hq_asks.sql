-- One row per question an external MCP client put to HQ's chat agent (hq_ask).
-- The chat thread holds the conversation; this row is what lets a different
-- MCP request find the outcome later, and what keeps a retried external_id from
-- posting the question twice. Timestamps are RFC 3339 like chat_threads.
CREATE TABLE IF NOT EXISTS hq_asks (
    ask_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL,
    turn_id TEXT,
    external_id TEXT,
    scope TEXT NOT NULL,
    mode TEXT NOT NULL,
    caller TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    answer TEXT,
    answer_message_id TEXT,
    error TEXT,
    created_at TEXT NOT NULL,
    answered_at TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_hq_asks_external
    ON hq_asks(scope, external_id) WHERE external_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_hq_asks_pending ON hq_asks(status) WHERE status = 'pending';
