-- Choices the owner makes in the web app that HQ's own work reads (crates/hq-db/src/owner_prefs.rs).
CREATE TABLE IF NOT EXISTS owner_prefs (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
