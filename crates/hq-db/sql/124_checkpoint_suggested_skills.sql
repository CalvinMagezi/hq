-- Skill names a session suggests the next one load. A JSON array of short slugs, a hint
-- from another session and never an instruction. Older rows read as an empty list.
ALTER TABLE task_checkpoints ADD COLUMN suggested_skills TEXT NOT NULL DEFAULT '[]';
