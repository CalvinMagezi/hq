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

## On the web

Settings, Work, **Reminders** (`/reminders`) lists upcoming reminders grouped as overdue, today,
tomorrow and later, with the finished ones on their own tab. **New reminder** takes a title, a time
(in an hour, this evening, tomorrow morning, next Monday, or a day and time you pick), a repeat and
"Keep reminding me until I mark it done". Each reminder has Done, Snooze and Stop. A reminder's
ping in the Inbox shows Done and the same snooze times. "This evening" and "tomorrow morning" are
18:00 and 09:00 in your saved time zone, and "this evening" is left out when it is nearly 18:00.

## On your phone

A reminder that rings is a card in **Needs you** (first after a sign-in) with **Done** and two snoozes,
in an hour and tomorrow morning. The notification has the same **Done** and **Snooze 1 hour** buttons.
They work without turning on notification details and without unlocking the phone, because they change
nothing you would regret.

The notification shows what you asked to be reminded of, even on the lock screen. If you would rather
it said only that a reminder is due, turn off **Show reminder text on the lock screen** in Settings,
Work, **Reminders**. The same page lists what is coming (overdue, today, tomorrow, later) and what has
finished, and **New reminder** takes a title, a quick time or a picked day and time, a repeat, and
"Keep reminding me until I mark it done". Times are worked out in your saved time zone, else the phone's.

## From a chat

Ask in any chat, on the phone, the web, Telegram or Discord: "remind me at 4 to call Moses about
the 50,000, and keep reminding me until it's done". HQ sets it with `remind_create`, says the time it
set in words, and the ping arrives in your inbox. HQ knows your time from the zone you saved (set by
the first phone that reports one) and says the current time to itself at the start of each of your
turns, so "tomorrow at 4" means your clock.

The tools are `remind_create`, `remind_list`, `remind_update` (change the time, mark it done) and
`remind_cancel`. They are only for you: a guest in a shared chat does not get them, and a session
running unattended, such as a watch, cannot set reminders. A reminder set after HQ read a web page,
an email or a document is marked "Set while HQ was reading outside content" and has no link, so text
from outside cannot send you anywhere. At most 30 can be set in an hour and 200 can be active.

A task can be named (`task_id`): the reminder then stops when the task is complete or archived.

For a time, use a reminder, not `/watch`. A watch asks the model a question on every check and only
works from Telegram and Discord; a reminder costs nothing and works everywhere.

## For apps and scripts

All of these need your sign-in, like the rest of the web API.

| Route | What it does |
|---|---|
| `GET /api/reminders?status=active\|done\|all` | The list, soonest first, with your saved `time_zone` |
| `POST /api/reminders` | `title`, then `at` (local `YYYY-MM-DDTHH:MM`) or `in_minutes`; optional `tz`, `repeat`, `nudge`, `note`, `task_id` |
| `PATCH /api/reminders/{id}` | Change `title`, `note`, the time, `tz`, `repeat` (`none` clears it) or `nudge` |
| `POST /api/reminders/{id}/done` | Done; a repeat moves to its next time |
| `POST /api/reminders/{id}/snooze` | `{ "minutes": 60 }` or `{ "until": <epoch seconds> }`, at most 90 days |
| `DELETE /api/reminders/{id}` | Stop it |

Done, snooze and delete also take the waiting ping out of the inbox. The inbox item itself takes
the actions `done`, `snooze` (with `minutes` or `until`) and `dismiss` on
`POST /api/notifications/{id}/action`. The saved zone is the `owner.timezone` preference on
`GET` and `PUT /api/prefs`.
