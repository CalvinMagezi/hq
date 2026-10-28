-- A second connector profile (Gemini Apps, docs/plans/gemini-connector.md) beside claude.ai.
-- Every existing row is Claude's, so the default keeps it exactly as it was.
ALTER TABLE mcp_oauth_requests ADD COLUMN profile TEXT NOT NULL DEFAULT 'claude';
ALTER TABLE mcp_oauth_codes ADD COLUMN profile TEXT NOT NULL DEFAULT 'claude';
ALTER TABLE mcp_oauth_grants ADD COLUMN profile TEXT NOT NULL DEFAULT 'claude';

-- Confidential clients the owner creates (`hq connector client add`). The secret is shown once
-- and kept only as its SHA-256. Revoking a client ends every grant it holds.
CREATE TABLE IF NOT EXISTS mcp_oauth_clients (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL,
    profile TEXT NOT NULL CHECK (profile IN ('gemini')),
    secret_hash TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    revoked_at INTEGER
);
