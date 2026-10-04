CREATE TABLE IF NOT EXISTS tool_usage (
    id          INTEGER PRIMARY KEY,
    tool_name   TEXT NOT NULL,
    agent_name  TEXT NOT NULL,
    session_id  TEXT NOT NULL,
    timestamp   INTEGER NOT NULL,  -- Unix timestamp (seconds)
    success     INTEGER NOT NULL DEFAULT 1,  -- SQLite bool: 1=true, 0=false
    error_msg   TEXT
);
CREATE INDEX IF NOT EXISTS idx_tool_usage_tool ON tool_usage(tool_name);
CREATE INDEX IF NOT EXISTS idx_tool_usage_agent ON tool_usage(agent_name);
CREATE INDEX IF NOT EXISTS idx_tool_usage_timestamp ON tool_usage(timestamp DESC);
