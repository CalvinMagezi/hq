-- Per-task outcome telemetry. Every LLM call in the router records one row
-- with cost, latency, success, and the TaskHint classification. Feeds the
-- model_card_refresher background job that EMA-updates ModelCard stats.

CREATE TABLE IF NOT EXISTS task_outcomes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    turn_idx INTEGER NOT NULL,
    model TEXT NOT NULL,
    provider TEXT NOT NULL,
    task_hint TEXT NOT NULL,
    latency_ms INTEGER NOT NULL,
    input_tokens INTEGER,
    output_tokens INTEGER,
    cost_usd REAL NOT NULL,
    success INTEGER NOT NULL,
    error_class TEXT,
    quality_score REAL,
    tool_calls_issued INTEGER NOT NULL DEFAULT 0,
    tool_calls_succeeded INTEGER NOT NULL DEFAULT 0,
    recorded_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_outcomes_model_task
    ON task_outcomes(model, task_hint, recorded_at);
CREATE INDEX IF NOT EXISTS idx_outcomes_session
    ON task_outcomes(session_id);
CREATE INDEX IF NOT EXISTS idx_outcomes_recorded
    ON task_outcomes(recorded_at);

-- Daily rollup for long-term history (refresher aggregates here after 30 days).
CREATE TABLE IF NOT EXISTS model_card_daily (
    day TEXT NOT NULL,
    model TEXT NOT NULL,
    task_hint TEXT NOT NULL,
    samples INTEGER NOT NULL,
    success_rate REAL NOT NULL,
    avg_latency_ms REAL NOT NULL,
    avg_cost_usd REAL NOT NULL,
    avg_quality REAL,
    PRIMARY KEY (day, model, task_hint)
);

-- Additive columns on usage_records. SQLite can't add columns conditionally,
-- so these are wrapped with a check against the schema_version guard
-- (migrations.rs only runs each file once).
ALTER TABLE usage_records ADD COLUMN task_hint TEXT;
ALTER TABLE usage_records ADD COLUMN session_id TEXT;
ALTER TABLE usage_records ADD COLUMN latency_ms INTEGER;
ALTER TABLE usage_records ADD COLUMN success INTEGER NOT NULL DEFAULT 1;
ALTER TABLE usage_records ADD COLUMN task_outcome_id INTEGER;

CREATE INDEX IF NOT EXISTS idx_usage_session ON usage_records(session_id);
CREATE INDEX IF NOT EXISTS idx_usage_task_hint ON usage_records(task_hint);
