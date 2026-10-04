-- Bugfix / simplification / feature / chore, so a mission's shape is known
-- without re-reading its goal text. Defaulted at the schema level, not by
-- extending insert_mission's arg list, matching how repo_path/base_ref were
-- added in migration 033 -- every existing insert_mission call site keeps
-- working unchanged.
ALTER TABLE missions ADD COLUMN category TEXT NOT NULL DEFAULT 'chore';
