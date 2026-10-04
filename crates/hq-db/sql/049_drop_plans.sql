-- Plans were retired on 2026-09-23 in favor of native tasks. Nothing reads
-- or writes these tables any more.
DROP INDEX IF EXISTS idx_plan_steps_plan;
DROP TABLE IF EXISTS plan_steps;
DROP TABLE IF EXISTS plans;
