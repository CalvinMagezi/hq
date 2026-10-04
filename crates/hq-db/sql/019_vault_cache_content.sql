-- Full note content in vault_cache for read-through caching.
-- NULL means content not yet cached; callers fall back to filesystem read.
ALTER TABLE vault_cache ADD COLUMN content TEXT;

CREATE INDEX IF NOT EXISTS idx_vault_cache_path_mtime ON vault_cache(path, mtime);
