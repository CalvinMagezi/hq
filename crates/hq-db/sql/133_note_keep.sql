-- The text a note had before HQ changed it, so the owner can put it back. One row per note, the
-- owner's version from before HQ's latest change; removed when the owner keeps the change or the
-- note is deleted. The size cap is enforced in code (`note_keep::MAX_PREV_BYTES`).
CREATE TABLE IF NOT EXISTS note_prev (
    path TEXT PRIMARY KEY,
    text TEXT NOT NULL,
    text_hash TEXT NOT NULL,
    -- The HQ version this text came before (set once HQ's write is recorded), so only the
    -- latest HQ change can be undone.
    for_hash TEXT,
    saved_at TEXT NOT NULL
);

-- Who put a note in the Trash, read by the Trash list. Set by the runtime from the deleting call,
-- never from the note. A row with no entry reads as the owner.
CREATE TABLE IF NOT EXISTS note_trash_meta (
    trash_path TEXT PRIMARY KEY,
    deleted_by TEXT NOT NULL CHECK (deleted_by IN ('you', 'hq')),
    deleted_at TEXT NOT NULL
);

-- One line per note the owner removed for good. Holds the path and the time, never any text.
CREATE TABLE IF NOT EXISTS note_purge_log (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    purged_at TEXT NOT NULL
);
