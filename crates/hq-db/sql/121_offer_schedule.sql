-- A next step the owner asked to see again later. A row is deleted once it has been delivered.
CREATE TABLE IF NOT EXISTS offer_schedule (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    thread_id TEXT NOT NULL,
    offer TEXT NOT NULL,
    line TEXT NOT NULL,
    reply TEXT NOT NULL,
    send INTEGER NOT NULL DEFAULT 0,
    due_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_offer_schedule_due ON offer_schedule(due_at);
