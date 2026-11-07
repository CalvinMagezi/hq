-- Fold blocks over a chat thread's older messages (docs/plans/context-fold.md). The original
-- messages stay in chat_messages; a block names the first and last message it summarizes and
-- is sent to the model in their place. Blocks are never rewritten: a tier-2 block absorbs older
-- blocks by setting their superseded_by, and an edit that deletes messages deletes the blocks
-- that reach them.
CREATE TABLE IF NOT EXISTS thread_folds (
    id             TEXT PRIMARY KEY,
    thread_id      TEXT NOT NULL REFERENCES chat_threads(thread_id) ON DELETE CASCADE,
    tier           INTEGER NOT NULL DEFAULT 1,
    first_msg_id   TEXT NOT NULL,
    last_msg_id    TEXT NOT NULL,
    message_count  INTEGER NOT NULL,
    summary        TEXT NOT NULL,
    source_tokens  INTEGER NOT NULL,
    summary_tokens INTEGER NOT NULL,
    tainted        INTEGER NOT NULL DEFAULT 0,
    model          TEXT NOT NULL,
    superseded_by  TEXT,
    created_at     TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_thread_folds_thread
    ON thread_folds(thread_id, superseded_by, created_at);
