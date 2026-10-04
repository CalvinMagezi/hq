CREATE TABLE IF NOT EXISTS value_items (
    id            TEXT PRIMARY KEY,
    source_task   TEXT NOT NULL,
    kind          TEXT NOT NULL,
    title         TEXT NOT NULL,
    body          TEXT NOT NULL,
    artifact_path TEXT,
    score         REAL NOT NULL DEFAULT 0,
    dedup_key     TEXT,
    state         TEXT NOT NULL DEFAULT 'pending',
    created_at    TEXT NOT NULL,
    routed_at     TEXT,
    delivered_at  TEXT,
    engaged_at    TEXT,
    expires_at    TEXT,
    engagement    TEXT
);

CREATE INDEX IF NOT EXISTS idx_value_items_state ON value_items(state, score DESC);
CREATE INDEX IF NOT EXISTS idx_value_items_dedup ON value_items(kind, dedup_key);
