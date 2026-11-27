-- One row per chat message for which HQ worked out which memories fit it (shadow and query mode
-- of `daemon.memory_recall`). It exists so the choice can be checked before it is switched on,
-- and watched afterwards. It holds ids, counts and timings only: never the message, never a
-- memory's text. It stays on this device, is read by `hq memory recall-report`, and is trimmed to
-- the last 30 days as it is written.
CREATE TABLE IF NOT EXISTS memory_recall_log (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    at             TEXT    NOT NULL,
    session_id     TEXT    NOT NULL DEFAULT '',
    mode           TEXT    NOT NULL,
    memory_ids     TEXT    NOT NULL DEFAULT '[]',
    items          INTEGER NOT NULL DEFAULT 0,
    tokens         INTEGER NOT NULL DEFAULT 0,
    silent_because TEXT,
    latency_ms     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_memory_recall_log_at ON memory_recall_log(at);
