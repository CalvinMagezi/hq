-- Client ids a browser or phone gave to what it made (a chat message, a note, a task), so a resend
-- from its outbox gets back the item HQ already made instead of a second one
-- (docs/plans/ux-run/web/wave-0/02-shared-parts.md rule 13). Kept 7 days.
CREATE TABLE IF NOT EXISTS client_ids (
    kind       TEXT NOT NULL,
    client_id  TEXT NOT NULL,
    item_id    TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL,
    PRIMARY KEY (kind, client_id)
);
CREATE INDEX IF NOT EXISTS idx_client_ids_created ON client_ids(created_at);
