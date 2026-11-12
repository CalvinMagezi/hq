-- The version of a note the owner last opened. A note HQ wrote reads as "new" until its current
-- version is here; a later HQ write is a different version and is new again. One row per path:
-- opening replaces the hash, so the table never grows past the number of notes.
CREATE TABLE IF NOT EXISTS note_seen (
    path TEXT PRIMARY KEY,
    content_hash TEXT NOT NULL,
    seen_at TEXT NOT NULL
);
