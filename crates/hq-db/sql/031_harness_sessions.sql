-- Unified registry of long-lived external harness sessions (claude-code,
-- cursor, opencode, pi, kimi, codex, qwen, hermes) running in tmux.
CREATE TABLE IF NOT EXISTS harness_sessions (
    id TEXT PRIMARY KEY,
    harness TEXT NOT NULL,
    label TEXT NOT NULL DEFAULT '',
    tmux_session TEXT NOT NULL,
    pid INTEGER,
    cwd TEXT NOT NULL,
    logfile TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running',
    resume_token TEXT,
    mission_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_harness_sessions_status ON harness_sessions(status);
CREATE INDEX IF NOT EXISTS idx_harness_sessions_harness ON harness_sessions(harness);
