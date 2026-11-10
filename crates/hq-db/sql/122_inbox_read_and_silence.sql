-- The inbox as a person keeps one (crates/hq-db/src/inbox_prefs.rs). An update can be read
-- without being put away, and a finished item can be cleared off the Done list for good.
ALTER TABLE value_items ADD COLUMN read_at TEXT;
ALTER TABLE value_items ADD COLUMN cleared_at TEXT;
-- Everything already there has been seen: only what arrives from now on starts unread.
UPDATE value_items SET read_at = created_at;

-- Topics the owner silenced: their items still land in the inbox, but nothing alerts.
-- `until` is null for "until I turn it back on". The topic `all` is the pause.
CREATE TABLE IF NOT EXISTS inbox_silences (
    topic      TEXT PRIMARY KEY,
    until      TEXT,
    created_at TEXT NOT NULL
);
