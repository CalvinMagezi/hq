# A dashboard snapshot shows a plot as a table of values, not a drawn chart

**Reason skipped:** `hq-export` has no chart in its document model, and adding one touches every writer (Word, HTML, Typst, the tabular ones). The snapshot for sharing ships now with tiles as labelled numbers and plots and `chart` blocks as tables of the values they would draw.
**Date:** 2026-11-20
**Where:** `crates/suite/hq-suite-adapter/src/snapshot.rs`, `crates/hq-tools-office/src/author.rs`
**Remove when:** `hq-export` can draw a bar, line, pie or donut chart from the `docs/CHART_BLOCKS.md` JSON, and `snapshot::chart_table` hands it the chart instead of a table.

The fidelity line of the export says "charts are shown as tables of their values, not drawn", so
nobody is told a picture was kept. The fix is one new `Block::Chart` in `crates/hq-export/src/doc.rs`
with a Typst drawing for PDF and PNG and a table fallback for the other writers.
