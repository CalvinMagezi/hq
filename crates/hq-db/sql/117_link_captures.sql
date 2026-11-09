-- Links the owner shared into a note, waiting for a summary (crates/hq-tools/src/link_capture.rs).
CREATE TABLE IF NOT EXISTS link_captures (
    path TEXT PRIMARY KEY,
    url TEXT NOT NULL,
    base_hash TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    reason TEXT,
    next_try_at TEXT NOT NULL,
    last_try_at TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_link_captures_due ON link_captures (state, next_try_at);
