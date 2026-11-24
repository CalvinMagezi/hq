# The web draws only the plot types the plot format has

**Reason skipped:** The task asked for area, stacked bar and scatter only if the server and the plot format already support them. `docs/suite/SHEETS.md` lists `bar`, `line`, `pie` and `donut` for a `plot` block, and `crates/hq-web/src/suite_api.rs` plots only those, so the web draws only those (now with axes, value labels, a tap-to-read line and several series from `group`).
**Date:** 2026-11-20
**Where:** `apps/hq-web/src/components/suite/PlotView.tsx`, `apps/hq-web/src/lib/dashQuery.ts`
**Remove when:** the plot format gains `area`, `stacked` or `scatter` in `hq-suite-md` and the server. Then add the marks to `PlotView.tsx` and the type names to `docs/suite/HQMD.md` and `SHEETS.md` together.

Also left: the dashboard's `where` and `group` are worked out in the browser (`dashQuery.ts`) so a dropdown can narrow a view without writing the note. The server's `/api/suite/plot` and `/tile` do not take a `where` from the request, so the browser reads the sheet rows itself. If the engines disagree on an edge (a text compare, a cell that is a formula the browser cannot work out) the page and the exported file can differ. A cell whose formula the browser's reader cannot work out counts as empty in a filter or a tile.
