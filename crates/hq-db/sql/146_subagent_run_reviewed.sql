-- When the parent chat recorded its verdict on a settled sub-agent run. A run the parent already
-- reviewed is not raised again as unfinished work (crates/hq-agent/src/followup.rs).
ALTER TABLE subagent_runs ADD COLUMN reviewed_at INTEGER;
