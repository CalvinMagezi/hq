# A reminder rings only while the server that holds it is running

**Reason skipped:** Android has no exact-alarm path in this app (no `AlarmManager`, no `WorkManager`), and a phone-side copy of every reminder is its own sync problem. The first version rings from the daemon and reaches the phone through the inbox poll.
**Date:** 2026-11-26
**Where:** `crates/hq-cli/src/commands/start/daemon/tasks_periodic/reminders.rs`, `crates/hq-tools/src/reminders/fire.rs`, `apps/android/app/src/main/java/example/hq/spike/Alerts.java`
**Remove when:** the phone keeps a local copy of the next few reminders and rings them when it cannot reach HQ.

If HQ is down at the due time, the reminder rings once it is back, labelled "Was due ...", and a repeat skips what it missed. A reminder can be up to about 30 seconds late (the daemon tick) plus up to about 20 seconds more before the phone shows it (the inbox poll). `hq reminders doctor` says when reminders are overdue.
