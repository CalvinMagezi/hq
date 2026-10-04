-- Coding agent registry: populated by daemon discovery, read by dispatcher.

CREATE TABLE IF NOT EXISTS coding_agent_registry (
    agent_name       TEXT PRIMARY KEY,
    binary_path      TEXT,
    version          TEXT,
    installed        INTEGER NOT NULL DEFAULT 0,
    available        INTEGER NOT NULL DEFAULT 0,
    last_checked_at  TEXT NOT NULL,
    metadata_json    TEXT
);

CREATE INDEX IF NOT EXISTS idx_registry_available
    ON coding_agent_registry(available, last_checked_at);

-- Per-agent performance tracking: success rates, duration, cost by task type and date.

CREATE TABLE IF NOT EXISTS coding_agent_performance (
    agent_name       TEXT NOT NULL,
    task_type        TEXT NOT NULL,
    metric_date      TEXT NOT NULL,
    sample_count     INTEGER NOT NULL DEFAULT 0,
    success_count    INTEGER NOT NULL DEFAULT 0,
    success_rate     REAL    NOT NULL DEFAULT 0.0,
    avg_duration_ms  REAL    NOT NULL DEFAULT 0.0,
    avg_cost_usd     REAL    NOT NULL DEFAULT 0.0,
    last_updated_at  TEXT    NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (agent_name, task_type, metric_date)
);

CREATE INDEX IF NOT EXISTS idx_agent_perf_date
    ON coding_agent_performance(metric_date DESC);
