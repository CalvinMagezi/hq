-- A work lease: one agent session holding a task for a stretch of time. It is
-- the session lock, the source of time-on-task and the record of which session
-- did the work, for agents HQ spawned and for any MCP client that claims a task.
-- An external lease (harness_session_id NULL) ends at its last heartbeat once it
-- has been silent for the configured TTL. A spawned session's lease follows the
-- session registry and is closed by the same events that move its task.
-- Only the hash of the lease token is stored.
CREATE TABLE IF NOT EXISTS task_work_sessions (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    actor TEXT NOT NULL,
    harness TEXT NOT NULL DEFAULT '',
    external_session_ref TEXT NOT NULL DEFAULT '',
    host TEXT NOT NULL DEFAULT '',
    cwd TEXT NOT NULL DEFAULT '',
    branch TEXT NOT NULL DEFAULT '',
    harness_session_id TEXT,
    token_hash TEXT NOT NULL,
    started_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_heartbeat_at TEXT NOT NULL DEFAULT (datetime('now')),
    ended_at TEXT,
    end_reason TEXT CHECK (end_reason IS NULL OR end_reason IN
        ('released', 'expired', 'superseded', 'session_ended'))
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_task_work_sessions_token ON task_work_sessions(token_hash);
CREATE INDEX IF NOT EXISTS idx_task_work_sessions_task ON task_work_sessions(task_id, started_at);
CREATE INDEX IF NOT EXISTS idx_task_work_sessions_live ON task_work_sessions(task_id) WHERE ended_at IS NULL;
-- A spawned session holds at most one open lease, whatever order its events arrive in.
CREATE UNIQUE INDEX IF NOT EXISTS idx_task_work_sessions_harness_open
    ON task_work_sessions(harness_session_id)
    WHERE harness_session_id IS NOT NULL AND ended_at IS NULL;
-- Who moved a task, and under which lease. NULL on events recorded before this.
ALTER TABLE task_events ADD COLUMN actor TEXT;
ALTER TABLE task_events ADD COLUMN work_session_id TEXT;
