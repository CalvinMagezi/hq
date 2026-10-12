-- The claude.ai connector's OAuth state (docs/plans/claude-connector.md). Every secret is
-- stored as its SHA-256; times are unix seconds.

-- The owner's window for connecting a new client. One row.
CREATE TABLE IF NOT EXISTS mcp_connector_arming (
    id INTEGER PRIMARY KEY CHECK (id = 1),
    armed_until INTEGER NOT NULL
);

-- An authorization request waiting on the owner's decision on the consent page. Only the
-- browser that started it (holding the binder, an HttpOnly cookie) can see or decide it, and
-- only within the arming window it arrived in.
CREATE TABLE IF NOT EXISTS mcp_oauth_requests (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL,
    client_name TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    state TEXT,
    code_challenge TEXT NOT NULL,
    resource TEXT NOT NULL,
    scope TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    armed_until INTEGER NOT NULL,
    binder_hash TEXT NOT NULL
);

-- An approved request, exchanged once for tokens. `grant_id` is set at the exchange, so a
-- second use can revoke what the first produced.
CREATE TABLE IF NOT EXISTS mcp_oauth_codes (
    code_hash TEXT PRIMARY KEY,
    client_id TEXT NOT NULL,
    client_name TEXT NOT NULL,
    redirect_uri TEXT NOT NULL,
    code_challenge TEXT NOT NULL,
    resource TEXT NOT NULL,
    scope TEXT NOT NULL,
    grant_scope TEXT NOT NULL CHECK (grant_scope IN ('full', 'read_only', 'tasks')),
    label TEXT NOT NULL,
    expires_at INTEGER NOT NULL,
    used INTEGER NOT NULL DEFAULT 0,
    grant_id TEXT
);

-- One owner approval: what a connected client may reach, until it is revoked.
CREATE TABLE IF NOT EXISTS mcp_oauth_grants (
    id TEXT PRIMARY KEY,
    client_id TEXT NOT NULL,
    client_name TEXT NOT NULL,
    label TEXT NOT NULL,
    grant_scope TEXT NOT NULL CHECK (grant_scope IN ('full', 'read_only', 'tasks')),
    scope TEXT NOT NULL,
    resource TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    expires_at INTEGER NOT NULL,
    last_used_at INTEGER,
    revoked_at INTEGER,
    revoked_reason TEXT
);

-- Access and refresh tokens. A refresh token is used once: `rotated_at` marks the use, and
-- presenting it again after a short grace revokes the grant.
CREATE TABLE IF NOT EXISTS mcp_oauth_tokens (
    token_hash TEXT PRIMARY KEY,
    grant_id TEXT NOT NULL REFERENCES mcp_oauth_grants(id) ON DELETE CASCADE,
    kind TEXT NOT NULL CHECK (kind IN ('access', 'refresh')),
    expires_at INTEGER NOT NULL,
    rotated_at INTEGER
);

CREATE INDEX IF NOT EXISTS idx_mcp_oauth_tokens_grant ON mcp_oauth_tokens(grant_id);

-- One row per tool call a connector grant makes. The arguments are never stored, only their
-- SHA-256, so the log shows what ran without keeping what Claude sent.
CREATE TABLE IF NOT EXISTS mcp_connector_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    grant_id TEXT NOT NULL,
    tool TEXT NOT NULL,
    outcome TEXT NOT NULL,
    args_sha256 TEXT NOT NULL,
    at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mcp_connector_audit_grant_at ON mcp_connector_audit(grant_id, at);
CREATE INDEX IF NOT EXISTS idx_mcp_connector_audit_at ON mcp_connector_audit(at);
