ALTER TABLE chat_threads ADD COLUMN platform TEXT NOT NULL DEFAULT 'web';
ALTER TABLE chat_threads ADD COLUMN external_id TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_chat_threads_platform_external
    ON chat_threads(platform, external_id)
    WHERE external_id IS NOT NULL;
