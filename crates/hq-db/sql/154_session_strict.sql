-- Sessions held to the strict sandbox. A row is never removed, so a later launch cannot widen one.
CREATE TABLE IF NOT EXISTS session_strict (
    session_id TEXT PRIMARY KEY
);
