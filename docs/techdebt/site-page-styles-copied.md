# The site's page styles repeat the tab, step and card rules per page

**Reason skipped:** `apps/site` styles are scoped per page, so the segmented picker, rail steps and bento rules in `index.astro` and `compare.astro` could not be reused by `/get/`. The Get page needed them inside components, so a copy went into `global.css`.
**Date:** 2026-11-19
**Where:** `apps/site/src/styles/global.css` (Get page block), `apps/site/src/pages/index.astro`, `apps/site/src/pages/compare.astro`
**Remove when:** the `.seg`, `.rail-steps` and `.bento` rules are deleted from the page files and only the `global.css` copy remains, checked with screenshots of Home and Compare.

The new guide pages (`/start`, `/guides`) will use the global copy too, which is why it was not made page-scoped.
