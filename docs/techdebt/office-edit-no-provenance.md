# office_edit writes are not recorded in note provenance and have no undo

**Reason skipped:** first slice of the agent-edit work. `hq-tools-office` cannot reach `hq-tools::provenance` (`locked_tool_write`, `hq_db::note_keep`) without a layering change, so the tool writes through `hq-suite-adapter` like the REST edits do.
**Date:** 2026-11-20
**Where:** `crates/hq-tools-office/src/edit.rs`, `crates/suite/hq-suite-adapter/src/blocks.rs`, `sheets.rs`
**Remove when:** edits take a write seam that keeps the previous text and records the caller, and `office_undo` exists (`docs/plans/hq-suite.md`, agent-edit phase).

Until then an agent edit shows as `note_changed_outside_tools` and can only be undone by hand.
