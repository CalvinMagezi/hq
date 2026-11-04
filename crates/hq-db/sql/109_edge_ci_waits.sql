-- The edge worker's CI waits (docs/plans/edge-worker-ci-loop.md): a commit the worker pushed,
-- whose checks the daemon reads instead of the worker holding a turn open. `thread_id` is the
-- worker's chat thread, where a failed round resumes.
CREATE TABLE IF NOT EXISTS edge_ci_waits (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id        TEXT NOT NULL,
    repo           TEXT NOT NULL,
    branch         TEXT NOT NULL,
    sha            TEXT NOT NULL,
    pull_request   TEXT,
    thread_id      TEXT NOT NULL,
    round          INTEGER NOT NULL,
    -- waiting, passed, failed, parked, replaced, closed
    state          TEXT NOT NULL DEFAULT 'waiting',
    detail         TEXT,
    created_at     TEXT NOT NULL DEFAULT (datetime('now')),
    checked_at     TEXT,
    last_read_at   TEXT,
    resolved_at    TEXT
);
CREATE INDEX IF NOT EXISTS idx_edge_ci_waits_state ON edge_ci_waits(state);
CREATE INDEX IF NOT EXISTS idx_edge_ci_waits_task ON edge_ci_waits(task_id);
