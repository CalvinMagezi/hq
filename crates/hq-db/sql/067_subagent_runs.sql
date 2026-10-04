-- Durable record of every delegated child run, plus an outbox of terminal
-- events so a settled child can wake its parent turn. Timestamps are unix
-- epoch seconds, like background_turns. Additive: nothing here alters an
-- existing table.
CREATE TABLE IF NOT EXISTS subagent_runs (
    run_id TEXT PRIMARY KEY,
    parent_run_id TEXT,
    parent_turn_id TEXT,
    platform TEXT,
    chat_id TEXT,
    thread_id TEXT,
    identity TEXT,
    task_id TEXT,
    child_id TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'general',
    goal TEXT NOT NULL,
    success_criteria TEXT NOT NULL DEFAULT '[]',
    required_tools TEXT NOT NULL DEFAULT '[]',
    detached INTEGER NOT NULL DEFAULT 0,
    owner_pid INTEGER,
    followup_depth INTEGER NOT NULL DEFAULT 0,
    exec_status TEXT NOT NULL DEFAULT 'queued',
    accept_status TEXT NOT NULL DEFAULT 'unverified',
    missing_deliverables TEXT NOT NULL DEFAULT '[]',
    blocker_reason TEXT,
    next_action TEXT,
    output_full TEXT,
    output_preview TEXT,
    error TEXT,
    resolved_backend TEXT,
    telemetry_seen INTEGER NOT NULL DEFAULT 0,
    started_at INTEGER NOT NULL,
    last_activity_at INTEGER NOT NULL,
    deadline_at INTEGER,
    settled_at INTEGER,
    escalated_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_subagent_runs_chat ON subagent_runs(platform, chat_id, started_at);
CREATE INDEX IF NOT EXISTS idx_subagent_runs_open ON subagent_runs(settled_at, last_activity_at);
CREATE INDEX IF NOT EXISTS idx_subagent_runs_task ON subagent_runs(task_id);

CREATE TABLE IF NOT EXISTS subagent_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    run_id TEXT NOT NULL REFERENCES subagent_runs(run_id),
    dedupe_key TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    claimed_at INTEGER,
    claimed_by TEXT,
    next_attempt_at INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    delivered_at INTEGER,
    last_error TEXT
);
CREATE INDEX IF NOT EXISTS idx_subagent_events_due ON subagent_events(status, next_attempt_at);
