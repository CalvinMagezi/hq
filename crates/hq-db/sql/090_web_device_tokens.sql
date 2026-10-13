-- Web sign-ins beyond the configured web token (docs/security/WEB_AUTH.md). A device token
-- is shown once; only its SHA-256 (hex) is kept. Times are unix seconds.
CREATE TABLE IF NOT EXISTS web_device_tokens (
    id TEXT PRIMARY KEY,
    token_hash TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    last_used_at INTEGER,
    -- Set only for a token a local command mints for one call, so a crash cannot leave it live.
    expires_at INTEGER
);

-- Configured web tokens (`web_auth_token`, `HQ_WEB_AUTH_TOKEN`) the owner signed out, by
-- hash, so an operator who sets a new value turns the configured token back on.
CREATE TABLE IF NOT EXISTS web_revoked_tokens (
    token_hash TEXT PRIMARY KEY,
    revoked_at INTEGER NOT NULL
);

-- The device token a Web Push subscription was saved with, so signing that device out stops its
-- pushes. NULL for one saved with the configured token or before this migration.
ALTER TABLE push_subscriptions ADD COLUMN device_id TEXT;
