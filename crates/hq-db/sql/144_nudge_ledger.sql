-- What happened to each candidate nudge: shown, withheld (with the rule), expired or answered.
-- Counts and ids only, never text. Stays on this device (crates/hq-db/src/nudge_ledger.rs).
CREATE TABLE IF NOT EXISTS nudge_ledger (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    source TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK (outcome IN ('shown','withheld','expired','answered')),
    rule TEXT NOT NULL DEFAULT '',
    at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_nudge_ledger_at ON nudge_ledger (at);
