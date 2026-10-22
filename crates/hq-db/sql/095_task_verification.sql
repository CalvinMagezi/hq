-- Verifying completion without a person reading each task.
-- docs/plans/task-verification.md.
--
-- task_verdicts is append-only: what a verifier decided about a task, on what artifact, with
-- the facts HQ observed before any model ran. A verdict is never edited; a later one replaces
-- it in effect. Only the purge of an archived task removes them.
CREATE TABLE IF NOT EXISTS task_verdicts (
    id TEXT PRIMARY KEY,
    task_id TEXT NOT NULL REFERENCES tasks(id),
    -- pr:<owner>/<repo>#<n>@<sha>, note:<path>@<sha256>, or none
    artifact TEXT NOT NULL DEFAULT 'none',
    -- Hash of the criteria and checks the verdict was made against.
    contract_hash TEXT NOT NULL DEFAULT '',
    tier INTEGER NOT NULL CHECK (tier BETWEEN 0 AND 2),
    verdict TEXT NOT NULL CHECK (verdict IN ('pass', 'fail', 'inconclusive')),
    -- How well the pass is evidenced.
    level TEXT NOT NULL CHECK (level IN ('machine_peer', 'peer', 'machine', 'owner', 'none')),
    -- An identity HQ issued: the verifier runner, a sub-agent run id, or the owner.
    verifier TEXT NOT NULL,
    independence TEXT NOT NULL DEFAULT 'none'
        CHECK (independence IN ('none', 'different_session', 'different_harness', 'different_model')),
    -- Facts HQ recorded before any model ran, and the per-criterion results that cite them.
    observations TEXT NOT NULL DEFAULT '[]',
    criteria TEXT NOT NULL DEFAULT '[]',
    reasons TEXT NOT NULL DEFAULT '',
    -- 1 when the verdict moved the task, 0 in shadow mode.
    applied INTEGER NOT NULL DEFAULT 0 CHECK (applied IN (0, 1)),
    -- 1 for a seeded test of the verifier, not a real task.
    canary INTEGER NOT NULL DEFAULT 0 CHECK (canary IN (0, 1)),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    -- The task_events id of the move into review the verdict is about. Timestamps are only
    -- accurate to the second, so a verdict is tied to a submission by this id and never by time.
    submission INTEGER
);
CREATE INDEX IF NOT EXISTS idx_task_verdicts_task ON task_verdicts(task_id, created_at);
CREATE INDEX IF NOT EXISTS idx_task_verdicts_recent ON task_verdicts(created_at);

CREATE TRIGGER IF NOT EXISTS task_verdicts_no_update
BEFORE UPDATE ON task_verdicts
BEGIN
    SELECT RAISE(ABORT, 'verdicts are append-only');
END;
CREATE TRIGGER IF NOT EXISTS task_verdicts_no_delete
BEFORE DELETE ON task_verdicts
WHEN (SELECT archived_at FROM tasks WHERE id = OLD.task_id) IS NULL
BEGIN
    SELECT RAISE(ABORT, 'verdicts are append-only; only purging an archived task removes them');
END;

-- The completion policy in force, one row. Every process that builds the task tools writes its
-- configuration here, so the web server, the daemon and an MCP server all meet the same rule at
-- the one place a task becomes complete.
CREATE TABLE IF NOT EXISTS task_verification_policy (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    config TEXT NOT NULL DEFAULT '{}',
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
INSERT OR IGNORE INTO task_verification_policy (id, config) VALUES (1, '{}');
