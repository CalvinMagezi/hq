-- What is kept of skill invocations once their raw rows age out (90 days by default).
--
-- One row per skill, version and month, holding only counts and a score sum. No invocation id,
-- session, task or anything else that could identify a use survives the roll-up, so counts and
-- mean outcomes keep working after the raw rows are gone and the retained data names no one.
CREATE TABLE IF NOT EXISTS skill_invocation_rollups (
    skill_name TEXT NOT NULL,
    -- The skill version the invocations ran against; '' when it was not recorded.
    skill_hash TEXT NOT NULL DEFAULT '',
    -- 'YYYY-MM' of the loads.
    month TEXT NOT NULL,
    invocations INTEGER NOT NULL DEFAULT 0,
    scored INTEGER NOT NULL DEFAULT 0,
    score_sum REAL NOT NULL DEFAULT 0,
    accepted INTEGER NOT NULL DEFAULT 0,
    reworked INTEGER NOT NULL DEFAULT 0,
    rejected INTEGER NOT NULL DEFAULT 0,
    helped INTEGER NOT NULL DEFAULT 0,
    hurt INTEGER NOT NULL DEFAULT 0,
    unused INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (skill_name, skill_hash, month)
);
