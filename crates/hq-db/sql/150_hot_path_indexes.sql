-- Indexes for two queries that ran as table scans.
--
-- `adoption::user_messages_since` reads chat_messages by role and time on every chat reply;
-- the only index was (thread_id, created_at). The mission lookups in the harness session
-- registry filter on mission_id, which had no index.
CREATE INDEX IF NOT EXISTS idx_chat_messages_role_created ON chat_messages(role, created_at);
CREATE INDEX IF NOT EXISTS idx_harness_sessions_mission ON harness_sessions(mission_id, created_at DESC);
