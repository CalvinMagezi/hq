# office_edit refuses row inserts and deletes that formulas, tiles or plots read, instead of rewriting references

**Reason skipped:** Rewriting `A1` references after rows move needs range shrinking, `#REF!` for deleted cells, cross-sheet names and the `from=` of tiles and plots, and a test for each. Refusing with a sentence that names the formula is safe and small, so that is what ships.
**Date:** 2026-11-20
**Where:** `crates/suite/hq-suite-adapter/src/sheet_ops.rs` (`guard_row_shift`, `mentions_row_from`)
**Remove when:** a reference shifter exists that handles ranges, absolute references, other sheets and block `from=` ranges, with tests; then `add_rows` with `at` and `delete_rows` can adjust and the guard can go.

The check is conservative: it looks at every word that reads as a cell (`D12`, `$B$3`), so a sheet name or a quoted name that looks like a cell can cause a refusal that is not needed. Adding rows at the end and adding a column on the right never need it. Whole-column references (`A:A`) have no row and are not checked. A second shortcut in the same change: a preview token is kept only in the running process (it is a keyed hash of the note's revision and the request), so a preview made before a restart cannot be applied and has to be asked for again.
