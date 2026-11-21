# A waiting comment is told apart from a sent one by its block, label, words and day

**Reason skipped:** `add_comment` takes no client id, so a replay cannot be matched to the comment HQ wrote except by comparing what it holds. The comment has a date, not a time.
**Date:** 2026-11-21
**Where:** `apps/hq-web/src/lib/commentOutbox.ts` (`sameComment`, `replayComments`)
**Remove when:** `add_comment` accepts a client id (or a time) that HQ stores and `comments` lists back.

Two comments with the same label and words on the same block on the same day look like one. If a
person adds both while offline, the second is not sent (HQ already has the first), and the sheet shows
one. The cost is a repeated sentence lost, never a different one.
