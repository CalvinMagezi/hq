-- Memories written by the cognition layer (dream and deep-sleep cycles),
-- removed in the 2026-09-23 teardown. On the owner's VPS they were 87% of
-- all memories and mostly sat at the decay floor. Filters on `source`, a
-- base-schema column, so this runs on databases of any age.
DELETE FROM memories WHERE source = 'deep-cognition';
