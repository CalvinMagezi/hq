-- Repairs to notes made by the notes check, so each can be taken back, and what the next scan
-- needs to know about the last one.
--
-- `vault_repairs` has one row per run: who started it, how many fixes it did, and where its
-- saved copies are (a folder under `_trash/`, which is emptied after 30 days like the rest of
-- the trash). `undone_at` is set when the run was taken back.
CREATE TABLE IF NOT EXISTS vault_repairs (
    run_id TEXT PRIMARY KEY,
    dir TEXT NOT NULL,
    ran_at TEXT NOT NULL,
    -- 'owner' for a person's tap or command, 'auto' for the daemon.
    source TEXT NOT NULL,
    fixes INTEGER NOT NULL,
    rewritten INTEGER NOT NULL,
    trashed INTEGER NOT NULL,
    undone_at TEXT,
    edited_since INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_vault_repairs_ran_at ON vault_repairs(ran_at DESC);

-- The fixes the last scan found, so a screen can offer them without reading every note again.
-- JSON of the same shape the web reads; the counts are for the card.
ALTER TABLE vault_health_runs ADD COLUMN fixes TEXT NOT NULL DEFAULT '[]';
ALTER TABLE vault_health_runs ADD COLUMN fix_a INTEGER NOT NULL DEFAULT 0;
ALTER TABLE vault_health_runs ADD COLUMN fix_b INTEGER NOT NULL DEFAULT 0;
-- Notes the last scan could not repair, and why, as JSON.
ALTER TABLE vault_health_runs ADD COLUMN skipped TEXT NOT NULL DEFAULT '[]';

-- Every note as the last scan saw it, so the next one can tell a renamed note (the same text
-- under a new name) from one that was deleted and another written.
CREATE TABLE IF NOT EXISTS vault_health_seen (
    path TEXT PRIMARY KEY,
    hash TEXT NOT NULL
);

-- Notes seen to move to a new name. A link to the old name is fixed by following this, for as
-- long as the row is kept (30 days).
CREATE TABLE IF NOT EXISTS vault_health_renames (
    old_path TEXT NOT NULL,
    new_path TEXT NOT NULL,
    seen_at TEXT NOT NULL,
    PRIMARY KEY (old_path, new_path)
);
