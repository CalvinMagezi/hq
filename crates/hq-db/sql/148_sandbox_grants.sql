-- Task-scoped sandbox grants (docs/plans/delegated-sandbox-grants.md). One row per request a
-- helper made: what it asked for, how HQ decided, and when the grant took effect, expired or was
-- revoked. A grant belongs to one session and is read back at every launch of that session.
-- Paths and ports only; never a secret value or file contents.
CREATE TABLE IF NOT EXISTS sandbox_grants (
    id            TEXT PRIMARY KEY,
    session_id    TEXT NOT NULL,
    task_id       TEXT,
    host          TEXT NOT NULL DEFAULT '',
    repo          TEXT NOT NULL DEFAULT '',
    capability    TEXT NOT NULL,
    resource      TEXT NOT NULL,
    -- The additive sandbox change this grant applies, as JSON (hq_core::grants::SandboxDelta).
    delta_json    TEXT NOT NULL DEFAULT '{}',
    -- pending (waiting for the owner), active, declined, refused, revoked.
    state         TEXT NOT NULL,
    -- policy (HQ under the owner's profile), owner, or system for a refusal.
    decided_by    TEXT NOT NULL DEFAULT 'system',
    -- The helper's own words. Shown to the owner as such; never read by the decision.
    helper_reason TEXT NOT NULL DEFAULT '',
    -- The reason HQ asked or refused, in the owner's words.
    decision_note TEXT NOT NULL DEFAULT '',
    requested_at  INTEGER NOT NULL,
    decided_at    INTEGER,
    applied_at    INTEGER,
    expires_at    INTEGER,
    revoked_at    INTEGER
);
CREATE INDEX IF NOT EXISTS idx_sandbox_grants_session ON sandbox_grants (session_id, state);
CREATE INDEX IF NOT EXISTS idx_sandbox_grants_requested ON sandbox_grants (requested_at);
