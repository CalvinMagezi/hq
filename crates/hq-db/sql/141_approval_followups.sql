-- One reminder and one expiry note per owner approval (crates/hq-db/src/approval_followup.rs).
CREATE TABLE IF NOT EXISTS approval_followups (
    item_id TEXT PRIMARY KEY,
    reminded_at TEXT,
    expiry_noted_at TEXT
);
