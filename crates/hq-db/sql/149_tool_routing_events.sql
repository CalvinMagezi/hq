-- How well the model finds and names tools (docs/plans: offline tool router). One row per event,
-- local only. It holds a kind, a tool name and a session id, never the words a person or a model
-- searched for, so it cannot reveal what anyone was asking about.
CREATE TABLE IF NOT EXISTS tool_routing_events (
    id         INTEGER PRIMARY KEY,
    timestamp  INTEGER NOT NULL,  -- Unix timestamp (seconds)
    session_id TEXT NOT NULL DEFAULT '',
    -- search_hit (tool_search or hq_discover found something), search_miss (found nothing),
    -- activation (a deferred tool was loaded), unknown_tool (a name that does not exist),
    -- unknown_tool_helped (the same, and a close name was offered).
    kind       TEXT NOT NULL,
    -- The tool involved: the one loaded, or the wrong name that was asked for. Cut to 80 chars.
    tool_name  TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_tool_routing_events_time ON tool_routing_events(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_tool_routing_events_kind ON tool_routing_events(kind, timestamp DESC);
