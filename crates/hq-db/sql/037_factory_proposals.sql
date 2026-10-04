-- What the factory scout has already proposed, so a finding the owner rejected is
-- never proposed again. Keyed on a fingerprint that survives line-number drift:
-- a fingerprint carrying a line number would re-propose the same finding after
-- any edit above it.
CREATE TABLE IF NOT EXISTS factory_proposals (
    fingerprint TEXT PRIMARY KEY,
    repo TEXT NOT NULL,
    source TEXT NOT NULL,
    rule TEXT NOT NULL,
    file TEXT NOT NULL,
    summary TEXT NOT NULL,
    severity INTEGER NOT NULL DEFAULT 0,
    state TEXT NOT NULL DEFAULT 'proposed',
    mission_id TEXT,
    pr_url TEXT,
    first_seen TEXT NOT NULL DEFAULT (datetime('now')),
    last_seen TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_factory_proposals_repo_state
    ON factory_proposals(repo, state);
CREATE INDEX IF NOT EXISTS idx_factory_proposals_mission
    ON factory_proposals(mission_id);
