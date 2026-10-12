-- The connector's call log. It first shipped inside 087 after 087 had already run on
-- instances, which never got it, so it lives in its own migration.
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
