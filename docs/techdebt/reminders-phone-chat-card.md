# A reminder set in chat shows no card under the reply on the phone

**Reason skipped:** the phone's cards under a reply are derived from tool steps (`blocks/StepCards.kt`), and a Reminder card with Change and Cancel needs its own state and tests. The reply already says the time in words, and the reminder is listed and editable in Settings, Reminders.
**Date:** 2026-11-27
**Where:** `apps/android/app/src/main/kotlin/example/hq/spike/ui/chat/blocks/StepCards.kt`
**Remove when:** a Reminder card is derived from the `remind_create` tool step, as a task card is from `task_create`.

The web has the same gap. Until then, the owner confirms the time from HQ's words in the reply.
