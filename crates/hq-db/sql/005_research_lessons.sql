CREATE TABLE IF NOT EXISTS research_lessons (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    iteration_id INTEGER NOT NULL,
    category TEXT NOT NULL,
    content TEXT NOT NULL,
    approach_tag TEXT NOT NULL DEFAULT '',
    outcome TEXT NOT NULL DEFAULT 'pending',
    weight REAL NOT NULL DEFAULT 1.0,
    harness_used TEXT NOT NULL DEFAULT '',
    model_used TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_research_lessons_session ON research_lessons(session_id);
CREATE INDEX IF NOT EXISTS idx_research_lessons_category ON research_lessons(category);
CREATE INDEX IF NOT EXISTS idx_research_lessons_weight ON research_lessons(weight DESC);
