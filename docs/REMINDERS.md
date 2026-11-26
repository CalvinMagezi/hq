---
audience: owner
order: 82
summary: "Reminders that ring at a time you choose, and follow up until you answer."
updated: 2026-11-26
---

# Reminders

HQ can hold a reminder for a clock time and ring it on your phone. It rings as an inbox item that
is pushed, with Done and Snooze on it. No model is involved, so a reminder costs nothing to ring.

## What a reminder is

- **A reminder** rings once at the time you chose, in your own time zone.
- **A nudge** keeps going until you answer. After the first ping it follows up 30 minutes later, then
  2 hours later, then at 09:00 on the next two mornings. The fifth ping says it is the last and asks
  whether to reschedule or drop it. After that it stops and shows as lapsed.
- **A repeat** rings again at the same time of day: every day, every N days, every weekday, on chosen
  weekdays, or on a day of the month (a short month rings on its last day). Done on a repeat answers
  that day only. Dismissing a repeat's ping does not end the repeat either.

Follow-up pings wait out your quiet hours (`reminders.quiet_hours`, default `21:00-07:00`). The time
you chose always rings, even inside them.

A reminder can follow a task (`task_id`). When the task is complete or archived, the reminder stops.

## Your time zone

Times mean your zone, not the server's. Set it once as the `owner.timezone` preference
(for example `Africa/Kampala`). Each reminder keeps the zone it was made in, so a repeat stays on the
same wall-clock time across a clock change. A time a clock change skips rings just after the gap.

## From the command line

```bash
hq reminders add "Call Moses about the 50,000" --at "2026-11-27 16:00" --tz Africa/Kampala --nudge
hq reminders add "Stand-up notes" --in-minutes 30
hq reminders add "Pay the water bill" --at "2026-12-01 09:00" --repeat monthly:1
hq reminders list
hq reminders done <id>
hq reminders doctor
```

`--repeat` takes `daily`, `every:3`, `weekdays`, `weekly:1,4` (1 is Monday) or `monthly:15`.

## Answering a ping

- **Done** ends the reminder, or carries a repeat to its next time.
- **Snooze** moves the next ping to a time you pick (an hour by default, up to 90 days).
- **Dismiss** stops a nudge (Undo brings it back within 10 minutes). On a repeat it only answers that day.

Leaving a ping alone is not an answer. A nudge keeps following up.

## Settings

| Setting | Default | What it does |
|---|---|---|
| `daemon.reminders` | on | Rings reminders. Off means nothing rings. |
| `reminders.quiet_hours` | `21:00-07:00` | Follow-ups wait for the morning. Empty turns it off. |
| `reminders.follow_up_hour` | 9 | The hour a nudge's next-day follow-ups ring. |
| `reminders.max_active` | 200 | Most reminders at once. |
| `reminders.max_per_hour` | 30 | Most created in an hour. |

## Limits

- Reminders ring from the machine that runs HQ. If it is off at the due time, the reminder rings when
  HQ is back, labelled "Was due ...". `hq reminders doctor` says when any are overdue.
- A reminder can arrive up to about 30 seconds late, plus the phone's own check of the inbox.
- A reminder's text is part of its inbox item. It is pushed like other inbox items, and a push shows
  the title only unless you turned details on for that device.
