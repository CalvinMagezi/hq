CREATE TABLE IF NOT EXISTS chat_threads (
    thread_id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    thread_type TEXT NOT NULL DEFAULT 'user',
    initiated_by TEXT NOT NULL DEFAULT 'user',
    status TEXT NOT NULL DEFAULT 'active',
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    last_message_preview TEXT,
    unread_count INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS chat_messages (
    message_id TEXT PRIMARY KEY,
    thread_id TEXT NOT NULL REFERENCES chat_threads(thread_id) ON DELETE CASCADE,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_chat_messages_thread
    ON chat_messages(thread_id, created_at ASC);

CREATE INDEX IF NOT EXISTS idx_chat_threads_updated
    ON chat_threads(updated_at DESC);
