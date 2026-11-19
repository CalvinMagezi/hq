# The `updated` date in a doc's front matter is typed by hand

**Reason skipped:** the docs pages show "Checked on <date>", but nothing yet fails when a doc changes and its date does not. A check that compares it with the file's last commit needs full git history in the Pages build.
**Date:** 2026-11-19
**Where:** `docs/*.md` front matter, `apps/site/src/content.config.ts`
**Remove when:** the Pages build can read each doc's last commit date, or CI checks that a changed doc also changed its `updated` line.
