-- Who produced which version of a note. Derived from the write paths that know, never from the
-- note's own text and never inferred: a version no write path recorded has no row. One row per
-- version of a note (path plus the hash of the raw file), so attribution cannot drift onto a
-- later edit.
CREATE TABLE IF NOT EXISTS note_provenance (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    -- 'unknown' when the write path could not say. Never guessed.
    actor_id TEXT NOT NULL DEFAULT 'unknown',
    source_kind TEXT NOT NULL DEFAULT 'unknown'
        CHECK (source_kind IN ('human', 'agent', 'imported', 'generated', 'system', 'unknown')),
    session_id TEXT,
    task_id TEXT,
    written_at TEXT NOT NULL,
    review_state TEXT NOT NULL DEFAULT 'unreviewed'
        CHECK (review_state IN ('unreviewed', 'reviewed', 'accepted', 'superseded', 'disputed')),
    reviewed_by TEXT,
    reviewed_at TEXT,
    review_reason TEXT,
    -- Attribution that was added after the version was first recorded.
    recorded_late INTEGER NOT NULL DEFAULT 0,
    -- Identifying fields wiped on request; the row stays so counts and history still add up.
    redacted INTEGER NOT NULL DEFAULT 0,
    -- Set when the note was deleted; the row is kept for a retention period.
    deleted_at TEXT,
    UNIQUE (path, content_hash)
);

CREATE INDEX IF NOT EXISTS idx_note_provenance_path ON note_provenance(path, id DESC);
CREATE INDEX IF NOT EXISTS idx_note_provenance_task ON note_provenance(task_id) WHERE task_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_note_provenance_session ON note_provenance(session_id) WHERE session_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_note_provenance_deleted ON note_provenance(deleted_at) WHERE deleted_at IS NOT NULL;

-- Every change of review state, so a decision can be traced to who made it and why.
CREATE TABLE IF NOT EXISTS note_review_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provenance_id INTEGER NOT NULL REFERENCES note_provenance(id) ON DELETE CASCADE,
    state TEXT NOT NULL,
    reviewer TEXT NOT NULL,
    reason TEXT,
    at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_note_review_events_row ON note_review_events(provenance_id, id);
