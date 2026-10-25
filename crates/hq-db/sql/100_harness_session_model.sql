-- The model a harness session was asked to run (`harness_session_spawn` / `_handoff` `model`),
-- passed to the CLI again when the session is resumed or its host restarts. NULL is the
-- harness's own default.
ALTER TABLE harness_sessions ADD COLUMN model TEXT;
