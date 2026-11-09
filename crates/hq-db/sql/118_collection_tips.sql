-- Saved notes the weekly tip has already brought up (crates/hq-tools/src/collection_tip.rs).
CREATE TABLE IF NOT EXISTS collection_tips (
    path TEXT PRIMARY KEY,
    surfaced_at TEXT NOT NULL
);
