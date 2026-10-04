CREATE TABLE IF NOT EXISTS earn_usage (
    month TEXT PRIMARY KEY, -- e.g. "2026-04"
    spend_usd REAL NOT NULL DEFAULT 0.0
);

CREATE TABLE IF NOT EXISTS bounties (
    id TEXT PRIMARY KEY,
    platform TEXT NOT NULL,
    title TEXT NOT NULL,
    status TEXT NOT NULL, -- "discovered", "evaluated", "in_progress", "submitted", "claimed"
    potential_roi REAL,
    reward_usd REAL,
    discovered_at TEXT NOT NULL DEFAULT (datetime('now'))
);
