-- Every status transition is logged, not only the first moves into in_progress
-- and ready_for_review. SQLite cannot widen a CHECK, so the table is rebuilt;
-- existing rows keep their ids and times, with the new columns NULL (unknown).
-- completed_at holds the latest completion and is cleared when a task reopens.
CREATE TABLE task_events_new (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    event_type TEXT NOT NULL CHECK (event_type IN (
        'entered_to_do', 'entered_in_progress', 'entered_blocked',
        'entered_ready_for_review', 'entered_complete')),
    occurred_at TEXT NOT NULL DEFAULT (datetime('now')),
    from_status TEXT,
    to_status TEXT
);
INSERT INTO task_events_new (id, task_id, event_type, occurred_at)
    SELECT id, task_id, event_type, occurred_at FROM task_events;
-- AUTOINCREMENT must not hand out an id the old table already used for an event
-- that was since deleted, so the new counter starts at the old one.
INSERT INTO sqlite_sequence (name, seq)
    SELECT 'task_events_new', seq FROM sqlite_sequence
    WHERE name = 'task_events'
      AND NOT EXISTS (SELECT 1 FROM sqlite_sequence WHERE name = 'task_events_new');
UPDATE sqlite_sequence
    SET seq = MAX(seq, COALESCE((SELECT seq FROM sqlite_sequence WHERE name = 'task_events'), 0))
    WHERE name = 'task_events_new';
DROP TABLE task_events;
ALTER TABLE task_events_new RENAME TO task_events;
CREATE INDEX IF NOT EXISTS idx_task_events_task ON task_events(task_id, id);
ALTER TABLE tasks ADD COLUMN completed_at TEXT;
