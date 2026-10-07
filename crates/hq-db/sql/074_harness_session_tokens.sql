-- One secret per launched session, given to that agent so HQ can tell which
-- session is calling. Only the SHA-256 of the secret is stored, and a secret
-- works only while its session is running. It is minted before the launch, so
-- it can exist before the session row does.
CREATE TABLE IF NOT EXISTS harness_session_tokens (
    session_id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
