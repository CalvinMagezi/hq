-- The spend ledger learns what a call really consumed and where it came from.
-- cost_usd stays NOT NULL; cost_source says whether it is a real figure. A row whose
-- source is 'unpriced' has an unknown cost, not a zero one.
ALTER TABLE task_outcomes ADD COLUMN cache_read_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE task_outcomes ADD COLUMN cache_write_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE task_outcomes ADD COLUMN reasoning_tokens INTEGER NOT NULL DEFAULT 0;
ALTER TABLE task_outcomes ADD COLUMN provider_cost_usd REAL;
ALTER TABLE task_outcomes ADD COLUMN cost_source TEXT NOT NULL DEFAULT 'table';
ALTER TABLE task_outcomes ADD COLUMN origin TEXT NOT NULL DEFAULT 'legacy';

-- Streamed calls used to be recorded at the first chunk, before any usage arrived.
UPDATE task_outcomes SET cost_source = 'unpriced' WHERE success = 1 AND input_tokens IS NULL;
UPDATE task_outcomes SET cost_source = 'none' WHERE success = 0;

CREATE INDEX IF NOT EXISTS idx_outcomes_origin ON task_outcomes(origin, recorded_at);

-- Raw rows older than the retention window are folded into one row per day, provider, model
-- and origin so reports stay fast and the totals survive pruning.
CREATE TABLE IF NOT EXISTS usage_daily (
    day TEXT NOT NULL,
    provider TEXT NOT NULL,
    model TEXT NOT NULL,
    origin TEXT NOT NULL,
    calls INTEGER NOT NULL,
    failed_calls INTEGER NOT NULL,
    unpriced_calls INTEGER NOT NULL,
    input_tokens INTEGER NOT NULL,
    output_tokens INTEGER NOT NULL,
    cache_read_tokens INTEGER NOT NULL,
    cache_write_tokens INTEGER NOT NULL,
    reasoning_tokens INTEGER NOT NULL,
    cost_usd REAL NOT NULL,
    PRIMARY KEY (day, provider, model, origin)
);

-- The retired fleet's second token stream. Nothing reads or writes it.
DROP TABLE IF EXISTS proxy_calls;
