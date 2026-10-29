-- The dialog a "waiting for you" inbox item is about, so a notification button answers that
-- exact dialog and no other (docs/plans/remote-driven-teams.md, "Design for the answer buttons").
CREATE TABLE IF NOT EXISTS dialog_alerts (
    -- The value_items row the alert is.
    item_id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    -- The dialog's number on its host (`pending_seq`); an answer names it.
    pending_seq INTEGER NOT NULL,
    -- command, edit, path, question, trust or other.
    kind TEXT NOT NULL,
    question TEXT,
    command TEXT,
    -- JSON: [{"id":1,"label":"Yes","remember":false,"typed":false}].
    options TEXT NOT NULL,
    -- The option a button may pick: one that answers once and opens no text field. NULL when none may.
    approve_option INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    -- Set by the first answer, so a second tap (another device, a replayed token) is refused.
    answered_at TEXT,
    answer TEXT
);
CREATE INDEX IF NOT EXISTS dialog_alerts_created ON dialog_alerts (created_at);
