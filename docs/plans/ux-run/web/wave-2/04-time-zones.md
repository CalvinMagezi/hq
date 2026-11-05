# Web wave 2 · 04 · Time zones: instants from HQ, one HQ zone, a display zone per browser (M11)

Boards: **AlignedWebRules** section 9 ("Time"), **AlignedSettingsMore** section 2 (Time zone),
**AlignedSpending** (the reset zone named), **AlignedSettings** ("Clock" moved), AlignedReview M11 and
Drift 11. Follows `../../rules-web.md` section 9. Where this spec and the rules differ, the rules win.

## 1. Finding and the review items it closes

- **M11 Time zones.** The server writes local times. Instants from HQ, one HQ zone plus a display zone,
  named when they differ.
- **Drift 11.** "approved 05:27 AM for Claude" reads as this morning for a grant twelve days old
  (`04-plain-words.md` fixes the words; this spec fixes the time).

Who it hurts: an owner travelling, or one whose HQ runs on a server in another zone: limits "reset at
midnight" at 03:00 their time, a daily summary arrives at an odd hour, a task "due today" is due
yesterday.

## 2. Before (as built, verified at `e5f9d5f`)

- Times in several shapes: tasks use SQLite UTC strings ("2026-10-15 14:39:16",
  `apps/hq-web/src/components/tasks/activity.ts:15`, parsed by `lib/tasksApi.ts` `parseSqliteUtc`); note
  records RFC 3339; the device notice writes "at {YYYY-MM-DD HH:MM} UTC"
  (`crates/hq-web/src/device_auth.rs:252`); `crates/hq-web/src/workbench_api.rs:253` and
  `crates/hq-tools/src/shortcuts/vault.rs:167, 219, 343, 362` format `chrono::Local::now()` (the server's
  own zone, as text).
- `apps/hq-web/src/lib/clock.ts:3, 59-70` a "clock zone" in localStorage for the header clock only;
  `components/HeaderClock.tsx`; `routes/settings.tsx:385` the Clock section ("This device ({city})").
- `crates/hq-db/src/usage_ledger.rs:153` `period_bounds()` uses UTC day, week (Monday) and month starts;
  the web does not say so.
- No HQ zone is reported to the web; `crates/hq-core/src/config` has no time zone key (HQ's zone is the
  host's local zone).

## 3. Decision

**HQ sends instants. HQ has one time zone and each browser a display zone. When they differ, the zone is
named where it matters. Spending says when its day resets.**

Rejected:

- **Show everything in HQ's zone.** A travelling owner would read every time wrong.
- **Show everything in the browser's zone with no names.** "Daily summary at 07:00" would be wrong by
  hours when HQ's zone differs.

## 4. Screens and states

- **Z1 · Settings › This browser › Time zone.** "Show times in" with the choices "Same as this browser
  ({Nairobi})" (default), and any zone (the board lists "Europe/London", "Africa/Kampala", "America/New
  York"); "Kept in this browser only. A phone and a laptop can each have their own." "HQ's time zone"
  / "HQ sends exact times. This setting only changes how they show here." / "{Nairobi}". "Now · 11:32
  London · 14:32 Nairobi". When they differ: "The zones differ, so HQ names them" / "Times show in London
  time. Where HQ's own zone matters, it is named: "Daily summary at 07:00 Nairobi time". Spending limits
  name theirs: "resets at 00:00 UTC"."
- **Z2 · The Settings row.** "Time zone · Nairobi, same as HQ" (same) or "Time zone · London · HQ is in
  Nairobi" (different).
- **Z3 · A time that depends on HQ's zone** (a scheduled task, a watch, a daily summary, a due date that
  HQ computes): named when the zones differ: "Daily summary at 07:00 Nairobi time".
- **Z4 · Spending.** Always named, since limits reset in UTC: "Limits reset at midnight UTC, which is
  03:00 in Nairobi. Weeks start on Monday." and per limit "resets in 12 h", "resets Monday, in 4 days",
  "resets 1 Dec, in 26 days"; over a limit "resets at 03:00, in 12 h".
- **Z5 · Ordinary times** (a message at 14:02, "Saved in this browser at 14:32", "seen 3 min ago"): in the
  display zone, never named.
- Widths, Lite and the HQ window: the same. Lite has no spending, so Z4 never shows there. In the HQ
  window showing the phone's HQ, "HQ's time zone" is the phone's.

## 5. Exact copy

| Key | String |
|-----|--------|
| `tz_show_in` | "Show times in" |
| `tz_same_as_browser` | "Same as this browser ({Nairobi})" |
| `tz_kept_here` | "Kept in this browser only. A phone and a laptop can each have their own." |
| `tz_hq_zone` | "HQ's time zone" · "HQ sends exact times. This setting only changes how they show here." |
| `tz_now` | "Now" · "{11:32} {London} · {14:32} {Nairobi}" |
| `tz_differ` | "The zones differ, so HQ names them" · "Times show in {London} time. Where HQ's own zone matters, it is named: "Daily summary at 07:00 Nairobi time". Spending limits name theirs: "resets at 00:00 UTC"." |
| `tz_row_same` | "{Nairobi}, same as HQ" |
| `tz_row_differ` | "{London} · HQ is in {Nairobi}" (Recommended; the board draws the same-zone row only) |
| `tz_named_time` | "{07:00} {Nairobi} time" |
| `spend_reset_line` | "Limits reset at midnight UTC, which is {03:00} in {Nairobi}. Weeks start on Monday." |

## 6. Behaviour rules

1. **Instants on the wire.** Every time HQ sends is RFC 3339 with an offset or Unix seconds. SQLite
   `datetime('now')` strings are converted on the way out (`parseSqliteUtc` stays only as a fallback).
   Server code stops formatting `chrono::Local::now()` into text that reaches the web (`workbench_api.rs:253`
   names a folder, which is fine; anything shown as a time is not).
2. **HQ's zone.** `/api/hello` returns `tz` (IANA, from the host, `iana-time-zone` crate) and its current
   offset.
3. **Display zone.** Per browser, in localStorage `hq-display-tz` (the old `clock.ts` key migrates);
   default the browser's own (`Intl.DateTimeFormat().resolvedOptions().timeZone`).
4. **Named when it matters.** A time HQ computes in its own zone is shown with that zone's city when the
   display zone differs; ordinary instants never are.
5. **Spending always names UTC** and adds the display zone's time (Z4).
6. **Formatting** through `Intl.DateTimeFormat` with `timeZone` set, the browser's language and 12 or
   24-hour cycle. City names from the zone id's last part, spaces for underscores ("New York").

## 7. Browser and platform notes

- `Intl.DateTimeFormat(..., {timeZone})` works in every current browser; `Intl.supportedValuesOf('timeZone')`
  lists zones (Chrome 99+, Safari 15.4+, Firefox 93+).
- A browser's zone changes when the laptop travels; "Same as this browser" follows it.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/lib/time.ts`, `lib/clock.ts` | Display zone, naming rule, formatters. | yes |
| `apps/hq-web/src/lib/tasksApi.ts` | Instants from the server; `parseSqliteUtc` as fallback. | yes |
| `apps/hq-web/src/components/settings/TimeZonePage.tsx` | Z1. | NEW |
| `crates/hq-web/src/api.rs` | `tz` in `/api/hello`. | NEW route |
| `crates/hq-web/src/tasks_api.rs`, `notifications_api.rs`, `threads_api.rs`, `device_auth.rs` | Serialise times as RFC 3339; the device notice without "UTC" text. | yes |

## 9. Server, desktop or release changes

Server: instants on every route the web reads and HQ's zone in `/api/hello` (`../../changes.md` "Times
sent as instants; one HQ time zone plus a display zone per browser").

## 10. Acceptance checks (each fails on today's build)

1. With HQ's zone `Africa/Nairobi` and the browser's `Europe/London`, a daily watch at 07:00 HQ time
   reads "07:00 Nairobi time" and a message time has no city. Today HQ's zone is unknown to the page.
2. Spending's reset line names UTC and the local time. Today it says neither.
3. `GET /api/tasks` returns `created_at` with an offset. Today it is "YYYY-MM-DD HH:MM:SS" without one.
4. The header has no clock; Time zone is under This browser. (Shared with `../wave-1/06-settings-groups.md`.)

## 11. Check on a laptop or phone

- Set the laptop's zone to another continent (or the browser's via DevTools, Sensors, Location): times
  move; Spending names UTC; a scheduled task names HQ's city.

## 12. Open decisions it depends on

- **M11** (rules-web 9). Depends on `../wave-0/02-shared-parts.md` (`/api/hello`),
  `../wave-1/04-plain-words.md` (dates by age), `../wave-4/09-spending.md`.
