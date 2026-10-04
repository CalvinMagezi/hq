-- Checkpointed self-update lifecycle: every run of HQ modifying its own
-- source is tracked from branch creation through install or rollback.
CREATE TABLE IF NOT EXISTS self_update_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    description TEXT NOT NULL,
    branch TEXT NOT NULL,
    base_rev TEXT NOT NULL,
    prev_binary_path TEXT,
    status TEXT NOT NULL DEFAULT 'open',
    test_output_tail TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    installed_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_self_update_runs_status ON self_update_runs(status);
