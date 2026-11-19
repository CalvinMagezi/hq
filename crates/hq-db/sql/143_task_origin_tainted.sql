-- A task whose text was written by a session that had read outside text. A helper started from it
-- starts narrowed even in a chat that is clean (crates/hq-db/src/tasks/origin.rs).
ALTER TABLE tasks ADD COLUMN origin_tainted INTEGER NOT NULL DEFAULT 0;
