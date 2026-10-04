CREATE TABLE IF NOT EXISTS proxy_calls (
    call_id       TEXT    PRIMARY KEY,
    harness       TEXT    NOT NULL,
    model_requested TEXT  NOT NULL,
    prompt_tokens INTEGER NOT NULL DEFAULT 0,
    output_tokens INTEGER NOT NULL DEFAULT 0,
    latency_ms    INTEGER NOT NULL DEFAULT 0,
    success       INTEGER NOT NULL DEFAULT 1,
    error_msg     TEXT,
    fleet_mode    TEXT,
    created_at    TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_proxy_calls_harness  ON proxy_calls(harness, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_proxy_calls_created  ON proxy_calls(created_at DESC);
