-- A web chat can watch a harness session: its events post into that thread
-- instead of the relay, and with `drive` on HQ answers the agent itself.
-- `pm_wake` is the durable "this thread owes the session a look" marker the
-- supervisor sets and the web driver clears, so a restart loses no event.
ALTER TABLE harness_sessions ADD COLUMN owner_thread TEXT;
ALTER TABLE harness_sessions ADD COLUMN drive INTEGER NOT NULL DEFAULT 0;
ALTER TABLE harness_sessions ADD COLUMN pm_wake TEXT;
ALTER TABLE harness_sessions ADD COLUMN last_driven_at TEXT;
-- Herdr's agent status as of the supervisor's last sweep, so the web panel
-- shows it without asking every host on each page load.
ALTER TABLE harness_sessions ADD COLUMN last_agent_status TEXT;
ALTER TABLE harness_sessions ADD COLUMN last_seen_at TEXT;
CREATE INDEX IF NOT EXISTS idx_harness_sessions_owner ON harness_sessions(owner_thread);
