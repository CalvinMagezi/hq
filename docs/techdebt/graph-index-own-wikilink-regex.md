# The entity graph index reads wikilinks with its own regex

**Reason skipped:** concept pages under `_graph/` are written by HQ itself in one fixed shape, link by slug rather than by note name, and are never edited by hand, so the shared link reader (`hq_vault::links`) gains nothing there today. Moving it would also change how slugs are matched, which is a graph-index decision, not a link-reading one.
**Date:** 2026-11-26
**Where:** `crates/hq-memory/src/graph_index.rs` (`wikilink_re`, `typed_line_re`)
**Remove when:** concept pages can be written or edited by anyone but HQ, or the graph index starts resolving links to ordinary notes: then it must read links through `hq_vault::links` like everything else.
