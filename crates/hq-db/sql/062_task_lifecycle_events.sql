-- Append-only log of each transition into in_progress or ready_for_review, in
-- UTC 'YYYY-MM-DD HH:MM:SS' like every other tasks timestamp. Tasks that
-- predate this migration have no rows and NULL summaries: unknown, not backfilled.
CREATE TABLE IF NOT EXISTS task_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    event_type TEXT NOT NULL CHECK (event_type IN ('entered_in_progress', 'entered_ready_for_review')),
    occurred_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_task_events_task ON task_events(task_id, id);
-- First-ever transitions only; later attempts live in task_events.
ALTER TABLE tasks ADD COLUMN work_started_at TEXT;
ALTER TABLE tasks ADD COLUMN first_ready_for_review_at TEXT;
