-- Vault Cache Table: Local-first cache for markdown records
CREATE TABLE IF NOT EXISTS vault_cache (
    path TEXT PRIMARY KEY,
    mtime INTEGER NOT NULL,
    hash TEXT NOT NULL,
    title TEXT,
    metadata TEXT, -- JSON blob of frontmatter
    content_preview TEXT,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_vault_cache_updated_at ON vault_cache(updated_at);
