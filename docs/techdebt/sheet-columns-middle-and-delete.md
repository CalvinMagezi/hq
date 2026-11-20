# The sheet screens cannot insert a column in the middle or delete a column

**Reason skipped:** HQ's `add_column` only adds on the right and there is no `delete_column`: moving or removing a column changes every formula, tile and plot that reads the columns after it, and the row reference shifter (`ref_shift.rs`) has no column twin yet. The web sheet's action sheet therefore offers "Add a column on the right" and a column type, and no insert-left or delete.
**Date:** 2026-11-20
**Where:** `crates/suite/hq-suite-adapter/src/sheet_ops.rs`, `apps/hq-web/src/lib/suiteOps.ts`, `apps/hq-web/src/components/suite/SheetView.tsx`
**Remove when:** `sheet_ops` can insert and delete columns with the same refuse-when-a-formula-reads-them guard, and the sheet screens offer them again.

Slides are edited on the web by text patches (`apps/hq-web/src/lib/slideEdit.ts`), which keeps the offline outbox and the hash guard. HQ's own `add_card`, `move_card` and the rest are for the agent and the apps that want them.
