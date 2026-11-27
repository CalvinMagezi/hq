-- Reminders the owner set, from chat or the Reminders screen (crates/hq-db/src/reminders.rs).
-- Times are UTC epoch seconds; `tz` is the IANA zone the owner meant them in, kept so a repeat
-- lands on the same wall-clock time after a change of offset.
CREATE TABLE IF NOT EXISTS reminders (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    note TEXT,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active','done','cancelled','lapsed')),
    due_at INTEGER NOT NULL,
    occurrence_at INTEGER NOT NULL,
    tz TEXT NOT NULL,
    repeat_rule TEXT,
    nudge INTEGER NOT NULL DEFAULT 0,
    nudge_step INTEGER NOT NULL DEFAULT 0,
    fire_seq INTEGER NOT NULL DEFAULT 0,
    last_item_id TEXT,
    task_id TEXT,
    thread_id TEXT,
    tainted INTEGER NOT NULL DEFAULT 0,
    source TEXT NOT NULL DEFAULT 'chat',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_fired_at INTEGER,
    acked_at INTEGER
);
CREATE INDEX IF NOT EXISTS idx_reminders_due ON reminders(status, due_at);

-- One row per ping, so "did it fire, and did I see it" can be answered later.
CREATE TABLE IF NOT EXISTS reminder_firings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    reminder_id TEXT NOT NULL,
    seq INTEGER NOT NULL,
    fired_at INTEGER NOT NULL,
    item_id TEXT,
    outcome TEXT NOT NULL DEFAULT 'fired'
        CHECK (outcome IN ('fired','done','snoozed','dismissed','superseded','missed'))
);
CREATE INDEX IF NOT EXISTS idx_reminder_firings_reminder ON reminder_firings(reminder_id, seq);
