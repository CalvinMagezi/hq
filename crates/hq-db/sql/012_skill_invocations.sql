CREATE TABLE IF NOT EXISTS skill_invocations (
    id TEXT PRIMARY KEY,
    skill_name TEXT NOT NULL,
    session_id TEXT NOT NULL DEFAULT '',
    trigger TEXT NOT NULL DEFAULT 'load_skill',
    loaded_at TEXT NOT NULL DEFAULT (datetime('now')),
    outcome_score REAL,
    outcome_source TEXT,
    updated_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_skill_invocations_name ON skill_invocations(skill_name);
CREATE INDEX IF NOT EXISTS idx_skill_invocations_session ON skill_invocations(session_id);
CREATE INDEX IF NOT EXISTS idx_skill_invocations_loaded_at ON skill_invocations(loaded_at DESC);
