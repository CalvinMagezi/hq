# skills tests reach up into skill_edit and skill_manage_tool

**Reason skipped:** the layering test reads non-test source, so two references that exist only in test files (`skills -> skill_edit`, 3 references, and `skills -> skill_manage_tool`, 2) are not violations and are not in `allowed_back_edges`. The Plan 1 D2 row also lists them; they were left alone because they do not affect the layering gate.
**Date:** 2026-11-18
**Where:** `crates/hq-tools/src/skills/tests.rs` (and the other `skills/` test files)
**Remove when:** those tests move up into the test modules of `skill_edit` and `skill_manage_tool`, so no `skills` test names a higher module. `python3 -I scripts/ci/module-graph.py` shows the two edges with `non-test=0` until then.
