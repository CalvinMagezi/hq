-- Periodic snapshots of the Copilot credit balance, the input to the burn-rate meter.
CREATE TABLE IF NOT EXISTS copilot_usage_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ts TEXT NOT NULL,
    login TEXT,
    credits_used REAL NOT NULL,
    remaining REAL NOT NULL,
    entitlement REAL NOT NULL,
    reset_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_copilot_usage_samples_ts ON copilot_usage_samples(ts);
