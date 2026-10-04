-- Facts were extracted and conflict-checked but never reached the prompt;
-- memories are the single store now. The lock only guarded fact writes.
DROP TABLE IF EXISTS facts_fts;
DROP TABLE IF EXISTS facts;
DROP TABLE IF EXISTS lessons;
DROP TABLE IF EXISTS memory_locks;
