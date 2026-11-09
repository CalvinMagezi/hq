-- What HQ Coder must not lose about a task's run (crates/hq-tools/src/github/dispatch.rs).
CREATE TABLE IF NOT EXISTS coder_jobs (
    task_id TEXT PRIMARY KEY REFERENCES tasks(id) ON DELETE CASCADE,
    repo TEXT NOT NULL,
    budget_usd REAL NOT NULL,
    credit_cap REAL,
    credit_start REAL,
    model TEXT,
    phase TEXT NOT NULL DEFAULT 'queued',
    attempts INTEGER NOT NULL DEFAULT 0,
    fix_rounds INTEGER NOT NULL DEFAULT 0,
    pr_number INTEGER,
    head_sha TEXT,
    next_check_at TEXT,
    spent_usd REAL NOT NULL DEFAULT 0,
    stop_reason TEXT,
    origin_thread TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_coder_jobs_phase ON coder_jobs (phase, next_check_at);
