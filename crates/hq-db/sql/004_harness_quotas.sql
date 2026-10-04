CREATE TABLE IF NOT EXISTS harness_quotas (
    harness TEXT PRIMARY KEY,
    daily_limit INTEGER NOT NULL,
    used_today INTEGER NOT NULL DEFAULT 0,
    reset_at TEXT NOT NULL
);
