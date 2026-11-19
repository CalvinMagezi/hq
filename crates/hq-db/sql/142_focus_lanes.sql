-- The lanes the owner works in (crates/hq-db/src/focus_lanes.rs). Stays on this device.
CREATE TABLE IF NOT EXISTS focus_lanes (
    lane TEXT PRIMARY KEY,
    added_at TEXT NOT NULL,
    parked_at TEXT
);
