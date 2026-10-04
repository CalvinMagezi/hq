-- Memory system extensions.
-- Uses pragma table_info to check before adding columns.

-- Replays table (always safe to CREATE IF NOT EXISTS)
CREATE TABLE IF NOT EXISTS replays (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    trigger_type   TEXT NOT NULL,
    trigger_source TEXT NOT NULL,
    trigger_ref    TEXT NOT NULL,
    memory_ids     TEXT NOT NULL,
    sequence       TEXT NOT NULL DEFAULT '[]',
    credit_delta   REAL NOT NULL DEFAULT 0,
    created_at     TEXT NOT NULL
);

-- Indexes (all IF NOT EXISTS — safe to re-run)
CREATE INDEX IF NOT EXISTS idx_memories_consolidated ON memories(consolidated);
CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_replays_trigger ON replays(trigger_type, created_at DESC);
