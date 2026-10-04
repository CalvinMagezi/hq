-- What a harness session is for and how anyone can tell it is finished. HQ may
-- only drive a session whose goal and definition of done pass the drive gate.
ALTER TABLE harness_sessions ADD COLUMN goal TEXT;
ALTER TABLE harness_sessions ADD COLUMN done_criteria TEXT;

-- Audit trail of goal changes and drive-mode changes, with the goal and
-- criteria in force at the time.
CREATE TABLE IF NOT EXISTS harness_session_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    actor TEXT NOT NULL,
    goal TEXT,
    done_criteria TEXT,
    detail TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_harness_session_events_session ON harness_session_events(session_id, id);
