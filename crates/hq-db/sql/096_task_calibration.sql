-- Measuring whether the verifier can be trusted (docs/plans/task-verification.md, "Calibration").
--
-- task_audits: tasks HQ completed that the owner is asked to check, one tap each: agree or not.
-- A disagreement reopens the task and counts against the verifier configuration that made it.
-- source is 'push' (the weekly floor), 'pull' (the owner asked for more), 'second_opinion' (a
-- fresh verifier was asked and disagreed with the first) or 'veto' (the owner reversed a tier 2
-- verdict inside its window).
CREATE TABLE IF NOT EXISTS task_audits (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    verdict_id TEXT NOT NULL REFERENCES task_verdicts(id),
    source TEXT NOT NULL CHECK (source IN ('push', 'pull', 'second_opinion', 'veto')),
    tier INTEGER NOT NULL CHECK (tier BETWEEN 0 AND 2),
    verifier_config TEXT NOT NULL DEFAULT '',
    selected_at TEXT NOT NULL DEFAULT (datetime('now')),
    answer TEXT CHECK (answer IS NULL OR answer IN ('agree', 'disagree')),
    note TEXT NOT NULL DEFAULT '',
    answered_at TEXT,
    answered_by TEXT
);
-- A verdict is audited once.
CREATE UNIQUE INDEX IF NOT EXISTS idx_task_audits_verdict ON task_audits(verdict_id)
    WHERE source <> 'second_opinion';
CREATE INDEX IF NOT EXISTS idx_task_audits_open ON task_audits(selected_at) WHERE answer IS NULL;
CREATE INDEX IF NOT EXISTS idx_task_audits_stratum ON task_audits(tier, verifier_config, answer);

-- Canary runs: a synthetic review request with a planted defect (the reviewer must not pass it)
-- or a clean control (it must), judged by the real reviewer at an unpredictable rate. They are
-- about the verifier, not about any task, so they have no task row.
CREATE TABLE IF NOT EXISTS task_canary_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    kind TEXT NOT NULL,
    -- pass for a control, fail for a planted defect.
    expected TEXT NOT NULL CHECK (expected IN ('pass', 'fail')),
    got TEXT NOT NULL CHECK (got IN ('pass', 'fail', 'unclear', 'error')),
    correct INTEGER NOT NULL CHECK (correct IN (0, 1)),
    verifier_config TEXT NOT NULL DEFAULT '',
    detail TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_task_canary_runs_config ON task_canary_runs(verifier_config, created_at);

-- Tier 2 work passes through a window before it completes: the verdict is recorded and approved,
-- the task stays in review waiting on the owner, and it completes when release_at passes unless
-- the owner vetoes it first. The currency snapshot is what the verdict was about (pull request
-- head commits, note hashes), checked again before the task completes. One row per task.
CREATE TABLE IF NOT EXISTS task_veto_holds (
    task_id TEXT PRIMARY KEY REFERENCES tasks(id),
    verdict_id TEXT NOT NULL REFERENCES task_verdicts(id),
    submission INTEGER,
    -- Unix seconds.
    release_at INTEGER NOT NULL,
    currency TEXT NOT NULL DEFAULT '{}',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_task_veto_holds_release ON task_veto_holds(release_at);
