-- Known harmless prompts (such as Claude Code's feedback survey) that the supervisor
-- dismissed in a driven session. Separate from nudges_sent and keys_sent so a dismissal
-- never spends the driver's allowances; capped on its own.
ALTER TABLE harness_sessions ADD COLUMN dismissals INTEGER NOT NULL DEFAULT 0;
