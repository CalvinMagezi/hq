-- Which lane quota alerts were raised, so a lane that stays above the warn or block line is
-- announced once per reset window. A new reset time is a new window, so it may alert again.
CREATE TABLE IF NOT EXISTS lane_quota_alerts (
    host TEXT NOT NULL,
    lane TEXT NOT NULL,
    window TEXT NOT NULL,
    level TEXT NOT NULL,
    resets_at TEXT NOT NULL,
    raised_at TEXT NOT NULL,
    PRIMARY KEY (host, lane, window, level, resets_at)
);
CREATE INDEX IF NOT EXISTS idx_lane_quota_alerts_raised ON lane_quota_alerts(raised_at);
