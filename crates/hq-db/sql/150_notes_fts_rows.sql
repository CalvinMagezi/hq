-- Path to FTS row id for the keyword index.
--
-- `notes_fts.path` is an ordinary FTS5 column, so `WHERE path = ?` scans every stored note body.
-- Indexing a note (delete the old row, insert the new one) therefore cost a full scan per note,
-- and a title lookup per semantic hit cost one more. This table maps a path to the row id of its
-- FTS row, so both become a primary-key lookup. Every write to `notes_fts` keeps it in step
-- (`search::fts::index_note`, `rebuild_index`, `search::reconcile::delete_content_rows`).
CREATE TABLE IF NOT EXISTS notes_fts_rows (
    path TEXT PRIMARY KEY,
    fts_rowid INTEGER NOT NULL
) WITHOUT ROWID;

-- Backfill. If an index ever held two rows for one path, keep the newest and drop the rest.
INSERT OR REPLACE INTO notes_fts_rows (path, fts_rowid)
    SELECT path, MAX(rowid) FROM notes_fts GROUP BY path;
DELETE FROM notes_fts WHERE rowid NOT IN (SELECT fts_rowid FROM notes_fts_rows);
