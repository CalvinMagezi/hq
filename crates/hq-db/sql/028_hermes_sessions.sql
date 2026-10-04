CREATE TABLE IF NOT EXISTS hermes_sessions (
    chat_key   TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
