ALTER TABLE proxy_calls ADD COLUMN task_type TEXT NOT NULL DEFAULT 'unknown';
CREATE INDEX IF NOT EXISTS idx_proxy_calls_task_type ON proxy_calls(task_type, harness, created_at DESC);
