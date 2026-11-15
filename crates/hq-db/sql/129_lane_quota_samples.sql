-- What each coding-agent lane's allowance looked like when a host last read it. Owner-only data:
-- never sent to a model provider or a third party (docs/LANE_QUOTAS.md). Only readings with a
-- live source are kept, so the newest row per lane is the last real reading to fall back on.
CREATE TABLE IF NOT EXISTS lane_quota_samples (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    host TEXT NOT NULL,
    lane TEXT NOT NULL,
    observed_at TEXT NOT NULL,
    account TEXT,
    source TEXT NOT NULL,
    confidence TEXT NOT NULL,
    windows TEXT NOT NULL,
    reason TEXT
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_lane_quota_samples_reading
    ON lane_quota_samples(host, lane, observed_at);
CREATE INDEX IF NOT EXISTS idx_lane_quota_samples_observed ON lane_quota_samples(observed_at);
