ALTER TABLE proxy_calls ADD COLUMN turn_id TEXT;
CREATE INDEX IF NOT EXISTS idx_proxy_calls_turn_id ON proxy_calls(turn_id);
