-- The command policy a session started with (docs/plans/remote-driven-teams.md, phase 2),
-- resolved and stored whole, so the supervisor judges its dialogs by the rules it was launched
-- with even after the owner edits the config.
CREATE TABLE IF NOT EXISTS session_policies (
    session_id TEXT PRIMARY KEY,
    -- The policy's name (builder, reviewer, or one of the owner's).
    name TEXT NOT NULL,
    -- JSON: hq_core::command_policy::Policy.
    rules TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
