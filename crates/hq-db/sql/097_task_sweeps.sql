-- Sweeps and the legacy guard (docs/plans/task-verification.md, "Clearing a queue on demand").
--
-- A sweep is a persisted batch over a scope of tasks waiting for review. Every phase (planning,
-- running, applying, undoing) is a status the daemon advances a few items at a time, so a sweep
-- survives a restart and a web request never waits for GitHub or a model.

-- When enforcement began. Tasks that entered review before then are the legacy backlog: the live
-- queue leaves them to a sweep, so turning enforcement on does not bulk-process old work.
ALTER TABLE task_verification_policy ADD COLUMN enforcing_since TEXT;

CREATE TABLE IF NOT EXISTS task_sweeps (
    id TEXT PRIMARY KEY,
    -- {"kind":"all"} | {"kind":"initiative","id":..} | {"kind":"since","date":..} | {"kind":"tasks","ids":[..]}
    scope TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN
        ('planning', 'planned', 'running', 'paused', 'cancelled', 'ready',
         'applying', 'applied', 'undone')),
    -- Counts per bucket, the estimate, and how many tasks would wait on the owner.
    plan TEXT NOT NULL DEFAULT '{}',
    note TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    applied_at TEXT,
    undone_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_task_sweeps_active ON task_sweeps(status)
    WHERE status IN ('planning', 'running', 'applying');

CREATE TABLE IF NOT EXISTS task_sweep_items (
    sweep_id TEXT NOT NULL REFERENCES task_sweeps(id),
    task_id TEXT NOT NULL REFERENCES tasks(id),
    -- Order of work: tasks others wait on, then tier 2, then oldest. Lower first.
    rank INTEGER NOT NULL,
    -- The submission the sweep looked at.
    submission INTEGER,
    status_before TEXT NOT NULL,
    tier INTEGER NOT NULL DEFAULT 0 CHECK (tier BETWEEN 0 AND 2),
    -- unplanned | needs_review | passed | failed | no_artifact | undecidable | needs_you
    bucket TEXT NOT NULL DEFAULT 'unplanned',
    -- unplanned: not looked at yet. planned: bucket known, no verdict yet.
    -- verdict: a verdict is recorded, waiting for apply. applied: apply moved it.
    -- kept: apply left it where it was. stale: the work changed, so it was not applied.
    -- undone: undo restored it. left_alone: undo found it changed since and left it.
    state TEXT NOT NULL DEFAULT 'unplanned' CHECK (state IN
        ('unplanned', 'planned', 'verdict', 'applied', 'kept', 'stale', 'undone', 'left_alone')),
    verdict_id TEXT REFERENCES task_verdicts(id),
    finding TEXT NOT NULL DEFAULT '',
    -- What the verdict was about, read again before it is applied.
    currency TEXT NOT NULL DEFAULT '{}',
    tries INTEGER NOT NULL DEFAULT 0,
    -- Unix seconds before which a deferred item is not tried again.
    retry_at INTEGER NOT NULL DEFAULT 0,
    applied_status TEXT,
    applied_updated_at TEXT,
    detail TEXT NOT NULL DEFAULT '',
    PRIMARY KEY (sweep_id, task_id)
);
CREATE INDEX IF NOT EXISTS idx_task_sweep_items_work ON task_sweep_items(sweep_id, state, rank);

-- A task the owner undid or chose to keep out of automatic verification for its current
-- submission. A new submission is looked at again.
CREATE TABLE IF NOT EXISTS task_verification_skips (
    task_id TEXT PRIMARY KEY REFERENCES tasks(id),
    submission INTEGER,
    reason TEXT NOT NULL DEFAULT '',
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

-- A second opinion the owner asked for on one task: a fresh verifier, ideally another model,
-- judges it again, and a disagreement counts as an audit datum against the first verdict.
CREATE TABLE IF NOT EXISTS task_reverify_requests (
    task_id TEXT PRIMARY KEY REFERENCES tasks(id),
    requested_at TEXT NOT NULL DEFAULT (datetime('now')),
    state TEXT NOT NULL DEFAULT 'pending' CHECK (state IN ('pending', 'agreed', 'disagreed', 'failed')),
    detail TEXT NOT NULL DEFAULT '',
    answered_at TEXT
);
