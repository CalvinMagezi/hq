-- When the owner has HQ forget a memory, a hash of its normalised text is kept so the same words
-- are not learned again from the next chat. Only the hash is kept: the text is deleted with the
-- memory. A hash can only confirm a guess at the exact sentence, it cannot be read back.
CREATE TABLE IF NOT EXISTS memory_tombstones (
    content_hash TEXT PRIMARY KEY,
    forgotten_at TEXT NOT NULL
);
