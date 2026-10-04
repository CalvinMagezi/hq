-- Hash of the screen tail at the last survey dismissal. An unchanged tail on the next sighting
-- means the key did nothing, so the supervisor stops instead of typing again.
ALTER TABLE harness_sessions ADD COLUMN last_dismiss_tail TEXT;
