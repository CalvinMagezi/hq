-- What each vault health scan found, and what the owner told it to stop mentioning.
--
-- `vault_health_runs` keeps one row per scan: the counts as columns, so a trend over weeks is
-- a plain query, and the report with its capped example lists as JSON, so the screens read
-- the last scan without reading the vault again. Only the newest few rows are kept.
-- Nothing here holds note text; the report names notes and the links written in them, and
-- stays in this database on the device.
CREATE TABLE IF NOT EXISTS vault_health_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ran_at TEXT NOT NULL,
    notes INTEGER NOT NULL,
    links INTEGER NOT NULL,
    dead INTEGER NOT NULL,
    ambiguous INTEGER NOT NULL,
    broken_anchors INTEGER NOT NULL,
    loose INTEGER NOT NULL,
    duplicates INTEGER NOT NULL,
    empty INTEGER NOT NULL,
    frontmatter INTEGER NOT NULL,
    unread INTEGER NOT NULL,
    report TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_vault_health_runs_ran_at ON vault_health_runs(ran_at DESC);

-- Things the owner chose to leave alone: `kind` says which list (dead_link, duplicate, loose,
-- and so on) and `key` names the item in it. An ignored item is left out of the counts.
CREATE TABLE IF NOT EXISTS vault_health_ignores (
    kind TEXT NOT NULL,
    key TEXT NOT NULL,
    ignored_at TEXT NOT NULL,
    PRIMARY KEY (kind, key)
);
