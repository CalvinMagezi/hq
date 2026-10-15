-- Evidence for improving skills: which version ran, for which task, and how it turned out.
--
-- Everything here is a hash, an id, or a value from a small fixed list. No prompt, no document
-- text and no free-form label is stored: `outcome_ref` points at a task comment or a review
-- record, it does not copy what that says. Rows written before this migration keep NULLs, and
-- NULL always means "not recorded".

-- SHA-256 (hex) of the skill's SKILL.md when it was loaded, the same digest the skill audit
-- trail (skills/_audit.jsonl) keeps, so an outcome reads against the right version.
ALTER TABLE skill_invocations ADD COLUMN skill_hash TEXT;

-- The HQ task the loading session was working on, when that is known.
ALTER TABLE skill_invocations ADD COLUMN task_id TEXT;

-- A coarse category for the work, taken from the task's tags. Never free text.
ALTER TABLE skill_invocations ADD COLUMN purpose TEXT
    CHECK (purpose IS NULL OR purpose IN
        ('code', 'research', 'writing', 'review', 'planning', 'operations', 'general'));

-- Whether the skill was only suggested, was loaded into the session, or what the post-session
-- review made of it.
ALTER TABLE skill_invocations ADD COLUMN used TEXT
    CHECK (used IS NULL OR used IN ('suggested', 'loaded', 'helped', 'hurt', 'unused'));

-- What an observable acceptance signal said about the work, and where to look for it
-- ("task:<id>", "comment:<id>", "run:<id>"). The session's own exit is not one.
ALTER TABLE skill_invocations ADD COLUMN outcome_status TEXT
    CHECK (outcome_status IS NULL OR outcome_status IN ('accepted', 'reworked', 'rejected', 'unknown'));
ALTER TABLE skill_invocations ADD COLUMN outcome_ref TEXT;

CREATE INDEX IF NOT EXISTS idx_skill_invocations_task
    ON skill_invocations(task_id) WHERE task_id IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_skill_invocations_version
    ON skill_invocations(skill_name, skill_hash);
