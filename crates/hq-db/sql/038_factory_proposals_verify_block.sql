-- How many consecutive times a proposal's mission failed for an
-- infrastructure/harness reason (a mechanical test failure, an empty diff, a
-- fabricated completion report) rather than a content rejection. Lets the
-- scout tell "this specific fix was wrong" apart from "the pipeline that
-- would have tried it was broken", and re-admit the latter once the count is
-- still under the give-up threshold instead of burying it in `superseded`
-- next to every finding the owner actually rejected on the merits.
ALTER TABLE factory_proposals ADD COLUMN verify_block_count INTEGER NOT NULL DEFAULT 0;
