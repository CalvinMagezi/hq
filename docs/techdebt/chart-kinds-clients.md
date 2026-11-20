# The web and phone chart renderers do not draw area, stacked or scatter

**Reason skipped:** the web app (`apps/hq-web/src/lib/chart.ts`) and the phone (`ChartSpec.kt`, `ChartViews.kt`) are owned by other changes, and a renderer that gets a type it does not know shows the block as a code block with a plain reason, so nothing breaks.
**Date:** 2026-11-20
**Where:** `apps/hq-web/src/lib/chart.ts`, `apps/android/app/src/main/kotlin/example/hq/spike/ui/chat/blocks/ChartSpec.kt`, `crates/suite/hq-suite-adapter/src/sheets.rs`
**Remove when:** both clients draw `area`, `stacked` and `scatter` from the JSON in `docs/CHART_BLOCKS.md`.

`plot` blocks with `type=area`, `stacked` or `scatter` produce chart JSON of that type (scatter adds `xValues`). Clients known to lack them: the web chart renderer (accepts line, bar, pie, donut only) and the Android chart card (same four). The native `.xlsx` export draws them, and the dashboard snapshot shows a table of values for every kind (see `dashboard-snapshot-charts.md`). Telegram, Discord and the CLI never drew charts.
