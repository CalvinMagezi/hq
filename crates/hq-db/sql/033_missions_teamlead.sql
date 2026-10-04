-- Team-lead mission engine: work-order/receipt contract, verification, worktree
-- isolation, transport selection, and an audit trail. Additive only — every
-- column is defaulted so existing rows load unchanged.

-- Assignment (what the step is and who should do it)
ALTER TABLE mission_steps ADD COLUMN role TEXT;
ALTER TABLE mission_steps ADD COLUMN acceptance_criteria TEXT NOT NULL DEFAULT '[]';
ALTER TABLE mission_steps ADD COLUMN files_in_scope TEXT NOT NULL DEFAULT '[]';
ALTER TABLE mission_steps ADD COLUMN test_commands TEXT NOT NULL DEFAULT '[]';
ALTER TABLE mission_steps ADD COLUMN model TEXT;
ALTER TABLE mission_steps ADD COLUMN max_turns INTEGER;
ALTER TABLE mission_steps ADD COLUMN timeout_secs INTEGER;
ALTER TABLE mission_steps ADD COLUMN deadline_at TEXT;
ALTER TABLE mission_steps ADD COLUMN transport TEXT NOT NULL DEFAULT 'native';

-- Isolation (where the step did its work)
ALTER TABLE mission_steps ADD COLUMN isolation TEXT NOT NULL DEFAULT 'none';
ALTER TABLE mission_steps ADD COLUMN worktree_path TEXT;
ALTER TABLE mission_steps ADD COLUMN branch TEXT;
ALTER TABLE mission_steps ADD COLUMN merged_sha TEXT;

-- Receipt (what came back)
ALTER TABLE mission_steps ADD COLUMN step_dir TEXT;
ALTER TABLE mission_steps ADD COLUMN report_json TEXT;
ALTER TABLE mission_steps ADD COLUMN diff_stat TEXT;
ALTER TABLE mission_steps ADD COLUMN files_changed TEXT NOT NULL DEFAULT '[]';
ALTER TABLE mission_steps ADD COLUMN test_summary TEXT;
ALTER TABLE mission_steps ADD COLUMN session_id TEXT;
ALTER TABLE mission_steps ADD COLUMN cost_usd REAL NOT NULL DEFAULT 0.0;
ALTER TABLE mission_steps ADD COLUMN external_ref TEXT;
ALTER TABLE mission_steps ADD COLUMN events_path TEXT;
ALTER TABLE mission_steps ADD COLUMN events_offset INTEGER NOT NULL DEFAULT 0;
ALTER TABLE mission_steps ADD COLUMN last_activity_at TEXT;

-- Review (whether HQ accepted it)
ALTER TABLE mission_steps ADD COLUMN verify_mode TEXT NOT NULL DEFAULT 'critic';
ALTER TABLE mission_steps ADD COLUMN verdict_json TEXT;
ALTER TABLE mission_steps ADD COLUMN last_failure_reason TEXT;
ALTER TABLE mission_steps ADD COLUMN attempt_history TEXT NOT NULL DEFAULT '[]';

-- Mission-level integration target
ALTER TABLE missions ADD COLUMN repo_path TEXT;
ALTER TABLE missions ADD COLUMN integration_branch TEXT;
ALTER TABLE missions ADD COLUMN base_ref TEXT NOT NULL DEFAULT 'HEAD';

-- Audit trail: answers "why did this mission take three days?" without logs.
CREATE TABLE IF NOT EXISTS mission_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    mission_id TEXT NOT NULL REFERENCES missions(id),
    step_id INTEGER,
    kind TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_mission_events_mission ON mission_events(mission_id, id);

-- Occupancy: nothing has ever asked "who else is working in this directory".
CREATE INDEX IF NOT EXISTS idx_harness_sessions_cwd ON harness_sessions(cwd);
