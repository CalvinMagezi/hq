-- Latest tmux pane snapshot for a running harness session. The pipe-pane
-- logfile is raw pty bytes and unreadable in a chat message, so the supervisor
-- stores a capture-pane snapshot on every sweep and delivers that on exit.
ALTER TABLE harness_sessions ADD COLUMN last_snapshot TEXT;
