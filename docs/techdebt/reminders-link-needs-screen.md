# A reminder's inbox item links to a Reminders screen that is not built yet

**Reason skipped:** the backend ships first so it can be tested without a client. The link is `/reminders?id=<id>`, which the web app and the phone do not yet route.
**Date:** 2026-11-26
**Where:** `crates/hq-tools/src/reminders/fire.rs` (`build_item`)
**Remove when:** the Reminders screen exists on the phone and the web app and both route `/reminders`.

Until then, tapping the item opens the inbox, which is where Done and Snooze are.
