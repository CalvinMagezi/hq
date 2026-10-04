-- Dead tables from the retired hq-workflow crate and company spend tracking.
DROP TABLE IF EXISTS workflow_run_events;
DROP TABLE IF EXISTS workflow_runs;
DROP TABLE IF EXISTS company_spend;

-- Memory schema that hq-memory used to create ad hoc at startup. The extra
-- memories and consolidations columns are added in Rust first (see
-- migrations::add_missing_memory_columns), since SQLite has no
-- ADD COLUMN IF NOT EXISTS and older databases already have them.
CREATE TABLE IF NOT EXISTS entity_nodes (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    canonical     TEXT NOT NULL UNIQUE,
    display_name  TEXT NOT NULL,
    entity_type   TEXT NOT NULL,
    mention_count INTEGER NOT NULL DEFAULT 1,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS entity_edges (
    source_id        INTEGER NOT NULL,
    target_id        INTEGER NOT NULL,
    relationship     TEXT NOT NULL,
    weight           REAL NOT NULL DEFAULT 1.0,
    source_memory_id INTEGER,
    updated_at       TEXT NOT NULL,
    PRIMARY KEY (source_id, target_id, relationship),
    FOREIGN KEY (source_id) REFERENCES entity_nodes(id),
    FOREIGN KEY (target_id) REFERENCES entity_nodes(id)
);

CREATE INDEX IF NOT EXISTS idx_memories_consolidated ON memories(consolidated);
CREATE INDEX IF NOT EXISTS idx_memories_topics ON memories(topics);
CREATE INDEX IF NOT EXISTS idx_memories_created ON memories(created_at DESC);
CREATE INDEX IF NOT EXISTS idx_memories_replay ON memories(replay_count DESC);
CREATE INDEX IF NOT EXISTS idx_entities_canonical ON entity_nodes(canonical);
CREATE INDEX IF NOT EXISTS idx_edges_source ON entity_edges(source_id);
CREATE INDEX IF NOT EXISTS idx_edges_target ON entity_edges(target_id);
CREATE INDEX IF NOT EXISTS idx_edges_weight ON entity_edges(weight DESC);
