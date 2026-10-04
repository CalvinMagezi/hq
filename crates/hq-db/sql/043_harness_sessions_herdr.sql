-- Sessions now run in Herdr, possibly on another machine, instead of tmux.
ALTER TABLE harness_sessions RENAME COLUMN tmux_session TO agent_name;
ALTER TABLE harness_sessions ADD COLUMN host TEXT NOT NULL DEFAULT 'local';
ALTER TABLE harness_sessions ADD COLUMN pane_id TEXT;
ALTER TABLE harness_sessions ADD COLUMN workspace_id TEXT;
-- Herdr state_change_seq of the last blocked-agent alert sent for this session.
ALTER TABLE harness_sessions ADD COLUMN blocked_notified_seq INTEGER;
-- A tmux-era row has no Herdr agent to reconcile against, and marking it
-- exited would send a completion message for a session that never ended.
UPDATE harness_sessions SET status = 'orphaned' WHERE status = 'running';
