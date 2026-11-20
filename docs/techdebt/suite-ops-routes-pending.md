# The web's sheet structure calls go to routes that the backend branch has not landed

**Reason skipped:** The web UI (`apps/hq-web/src/lib/suiteOps.ts`) was written in parallel with the backend sheet and slide structure endpoints, so it calls the planned routes `POST /api/suite/sheet/rows`, `/sheet/columns`, `/sheet/column-type`, `/slides/card`, `/slides/move`, `/slides/notes` and `/slides/style` with bodies guessed from the plan (`path`, `base` hash, `index`, plus an action and a position). None of them exist in `crates/hq-web/src/suite_api.rs` yet.
**Date:** 2026-11-20
**Where:** `apps/hq-web/src/lib/suiteOps.ts` (`OP_ROUTES` and the bodies in `createSuiteOps`), `apps/hq-web/src/components/suite/SheetView.tsx` (the cell actions sheet)
**Remove when:** the backend branch lands. Check each body in `createSuiteOps` against its handler, change whichever side is wrong, and delete the OPTIONS probe and the "hidden on 404" handling only if the routes are always present.

What the user sees meanwhile: the sheet's insert and delete of rows and columns and the column type picker are hidden once the first probe (an OPTIONS request, which changes nothing) or the first call answers 404, so nothing offers an action that cannot work. Cell edits, paste, fill down, sort, filter and the record card do not use these routes: they are text edits saved through the note's hash guard.

Slides: add, copy, delete, move, layout, transition, colours and speaker notes are written by `apps/hq-web/src/lib/slideEdit.ts` as text patches on the note (the same comments `docs/suite/SLIDES.md` describes), saved through the same hash guard and the offline outbox. The planned `/slides/*` routes are therefore declared in `suiteOps.ts` but not used by any screen. Keep them, or delete them from the interface, once the backend decides whether it wants to own those edits.
