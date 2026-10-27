-- Folders the owner approved for sessions a bounded caller starts (a claude.ai connector, or the
-- handoff key when agent_host.handoff_cwd_allow lists anything), beyond that list and the host's
-- own HQ folder. A grant covers the folder and everything under it, on one host, until revoked.
-- Written only after the owner's signed approval of a request (docs/AGENT_SESSIONS.md).
CREATE TABLE IF NOT EXISTS session_folder_grants (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    -- The host name as the agent host knows it (`laptop`, `native`).
    host TEXT NOT NULL,
    -- Absolute folder, as the request named it.
    path TEXT NOT NULL,
    -- Who asked: the connector grant's label, or `handoff key`.
    requested_by TEXT NOT NULL,
    -- Unix seconds.
    approved_at INTEGER NOT NULL,
    revoked_at INTEGER
);
CREATE UNIQUE INDEX IF NOT EXISTS session_folder_grants_live
    ON session_folder_grants (host, path) WHERE revoked_at IS NULL;
