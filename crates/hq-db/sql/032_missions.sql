-- Durable multi-day missions: daemon-owned long-horizon work units that
-- survive restarts. A mission runs many bounded sessions; per-session caps
-- apply to steps, never to the mission itself.
CREATE TABLE IF NOT EXISTS missions (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    goal TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    checkpoint_json TEXT NOT NULL DEFAULT '{}',
    plan_path TEXT,
    origin_identity TEXT,
    budget_usd REAL NOT NULL DEFAULT 25.0,
    spent_usd REAL NOT NULL DEFAULT 0.0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS mission_steps (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mission_id TEXT NOT NULL REFERENCES missions(id),
    seq INTEGER NOT NULL,
    description TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    kind TEXT NOT NULL DEFAULT 'native',
    harness TEXT,
    session_ref TEXT,
    wait_key TEXT,
    depends_on TEXT NOT NULL DEFAULT '[]',
    retries INTEGER NOT NULL DEFAULT 0,
    max_retries INTEGER NOT NULL DEFAULT 2,
    result_summary TEXT,
    artifact_path TEXT,
    started_at TEXT,
    finished_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_missions_status ON missions(status);
CREATE INDEX IF NOT EXISTS idx_mission_steps_mission ON mission_steps(mission_id, status);
