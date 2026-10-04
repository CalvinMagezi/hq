-- Deterministic limits on a driven session. The driver counts what it sent,
-- remembers whether the last finished turn showed new tool activity, and keeps
-- the reason Drive was last switched off for the Watching panel.
ALTER TABLE harness_sessions ADD COLUMN nudges_sent INTEGER NOT NULL DEFAULT 0;
ALTER TABLE harness_sessions ADD COLUMN last_wake_nudges INTEGER;
ALTER TABLE harness_sessions ADD COLUMN no_progress_streak INTEGER NOT NULL DEFAULT 0;
ALTER TABLE harness_sessions ADD COLUMN progress_mark TEXT;
ALTER TABLE harness_sessions ADD COLUMN drive_off_reason TEXT;
ALTER TABLE harness_sessions ADD COLUMN keys_sent INTEGER NOT NULL DEFAULT 0;
-- Who started the session: 'user' (a chat the owner typed in), 'mcp' (an MCP client with no chat),
-- 'ask' (a chat an hq_ask opened). The last two are capped.
ALTER TABLE harness_sessions ADD COLUMN origin TEXT NOT NULL DEFAULT 'user';
