-- Durable background turns: relay turns that outlive the ack window detach
-- into persisted background tasks so they survive daemon restarts and can be
-- reaped, cancelled, or recorded back into the chat thread on completion.
CREATE TABLE IF NOT EXISTS background_turns (
    id TEXT PRIMARY KEY,
    platform TEXT NOT NULL,           -- telegram | discord | web | cli
    chat_id TEXT NOT NULL,
    thread_id TEXT,
    identity TEXT,                    -- caller identity for thread recording
    prompt TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running',  -- running | completed | failed | interrupted
    created_at INTEGER NOT NULL,      -- unix epoch seconds
    completed_at INTEGER,             -- unix epoch seconds
    result_text TEXT,
    child_session_ids TEXT,           -- JSON array of harness session ids
    cancel_token TEXT
);

CREATE INDEX IF NOT EXISTS idx_background_turns_status ON background_turns(status);
