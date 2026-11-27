-- Sessions held to the strict sandbox. A row is never removed, so a later launch cannot widen one.
CREATE TABLE IF NOT EXISTS session_strict (
    session_id TEXT PRIMARY KEY
);
-- Sessions already running from a marked task, so what they start is held too.
INSERT OR IGNORE INTO session_strict (session_id)
SELECT s.id FROM harness_sessions s JOIN tasks t ON t.id = s.mission_id
WHERE t.origin_tainted = 1;
