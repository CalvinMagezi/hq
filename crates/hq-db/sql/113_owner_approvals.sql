-- The call an owner-approval inbox item asks about, as HQ's runtime holds it, so every app quotes
-- the tool and its arguments verbatim and marks a losing request from the tool, never from the
-- model's words (docs/plans/ux-run/web/wave-1/02-dialogs-approvals.md rules 7 and 8).
CREATE TABLE IF NOT EXISTS owner_approvals (
    -- The value_items row the request is.
    item_id    TEXT PRIMARY KEY,
    tool       TEXT NOT NULL,
    -- JSON object, the arguments without the runtime's `_hq_*` keys.
    args       TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS owner_approvals_created ON owner_approvals (created_at);
