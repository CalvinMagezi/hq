ALTER TABLE proxy_calls ADD COLUMN user_id TEXT;
ALTER TABLE proxy_calls ADD COLUMN source TEXT NOT NULL DEFAULT 'unknown';
CREATE INDEX IF NOT EXISTS idx_proxy_calls_user ON proxy_calls(user_id, created_at DESC);
