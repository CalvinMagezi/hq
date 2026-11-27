# The phone's memory page cannot edit a note or search them

**Reason skipped:** the first phone page covers what matters most on a shared or lost phone (look, pin, keep, forget, pause) and the Android build could not be run where it was written, so it was kept small.
**Date:** 2026-11-27
**Where:** `apps/android/app/src/main/kotlin/example/hq/spike/ui/settings/MemoryPage.kt`
**Remove when:** the page has an edit box and a search field like Settings, What HQ remembers on the web (`apps/hq-web/src/components/settings/MemoryView.tsx`), and filters beyond the newest 50.

The page lists the newest 50 notes only. Editing and searching are on the web page and in `hq memory`.
