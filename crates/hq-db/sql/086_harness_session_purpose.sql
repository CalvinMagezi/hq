-- What a session was started to do: code (the default for coding agents), or for a
-- general-purpose agent monitor, research, review or general. A monitor or reviewer
-- never moves its task's status; see hq_tools::harness_session::category.
ALTER TABLE harness_sessions ADD COLUMN purpose TEXT NOT NULL DEFAULT 'code';

-- Sessions a monitor or reviewer was given to watch. When an observed session
-- finishes a turn, blocks or exits, the supervisor leaves the observer a message on
-- the observer's task thread, typed into it when it is next idle.
CREATE TABLE IF NOT EXISTS harness_session_observers (
    observer_id TEXT NOT NULL REFERENCES harness_sessions(id),
    observed_id TEXT NOT NULL REFERENCES harness_sessions(id),
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY (observer_id, observed_id)
);
CREATE INDEX IF NOT EXISTS idx_harness_session_observers_observed ON harness_session_observers(observed_id);
