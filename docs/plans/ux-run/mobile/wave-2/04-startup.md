# Wave 2 · 04 · Startup: real progress, one cover, a truthful running notice (R4, M9)

Board: **FixStartup** ("Fix startup"), spec-card items from First run and Alerts, revised after review,
5 Nov 2026. Area: startup; real progress, one cover on the first launch only, and a running notice that
says the truth. This board follows FixOffline and rules section 6 (R4): the start steps live in the
Connection sheet (`../wave-4/01-offline.md`). `../../rules-shared.md` sections 1, 2, 3, 6, 7 win.

Headline: **Show the start as the steps the app can see, in the Connection sheet after the first launch,
and call HQ stopped only when it is.**

Note on order (`../../resolutions.md` D-5): the Connection sheet's data model and words are wave 0
(`data/Connection`, `ConnectionWords`); its UI is built once, in `ui/connection/`, here in wave 2 with
Startup (the start steps, "usually about", "Stopped at", "almost full"); wave 4 (FixOffline) extends it
with saved copies and server rows. There is one "almost full" formula: under 200 MB free for the app's
files (N15).

## 1. Finding and the review items it closes

**Problem.** The Starting HQ bar is a fixed 40% fill that never moves; at 60 s "HQ did not start" covers
the app on every launch, even while hq is still opening a large vault; and the "HQ is running" notice has
no tap action and keeps saying "running" while HQ is down. (NowFindings, smaller items: "the Starting HQ
bar is a fixed 40%, the running notice has no tap action".)

**Who and how often.** Every new owner on the first start (up to a minute on a mid-range phone, the
moment they judge whether the app works), every owner after each reboot, when the notice is the only
sign of HQ, and owners with large vaults on slow phones, whose start is cut off and restarted again and
again (M9).

What changed in this revision:

- **R4:** the full-screen cover shows only on the very first launch (frame 1). After that the four steps,
  "usually about {N} s" and "Stopped at" live in the Connection sheet (frames 3 to 5), as on FixOffline.
  "HQ did not start" as a cover is gone.
- **M9:** HQ has stopped only when its process is dead, or its step has not moved for the longer of 60 s
  and twice the usual start. While it is alive the sheet says "HQ is still opening your notes. Restarting
  starts over." with Restart second (frame 4).
- **M9:** "The phone is almost full" as a reason, from the rules section 2 catalogue (frame 4).
- **R5, rules section 1:** notice words "HQ is running", "HQ is starting", "HQ is restarting", "HQ has
  stopped · Tap to see why". The old "down" notice and the Try again button are retired; the health check
  and exit code show only in Technical details.
- **Rules sections 3 and 7:** one 4 dp blue bar; "HQ has stopped" is amber with a stop icon; "Open Android
  settings" with the line to tap; "Step 3 of 4".
- **R3:** Save a copy of my notes on the stopped sheet (FixSafeMode).
- **M8, M7, M15:** the server owner (frame 7), Android 8 to 11 (frame 8), 360 dp at font scale 2.0
  (frame 9).

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/` (`K` = `kotlin/example/hq/spike/`).

- `K/ui/startup/Startup.kt:9-14` `FirstLaunch` with `first_done`; `:17` `enum class Startup { None,
  Starting, DidNotStart }`; `:22` `GIVE_UP_SECONDS = 60L`; `:24-28` `decide()` returns `DidNotStart` at
  60 s whether or not the first launch is done.
- `K/ui/startup/StartupScreens.kt:36` `PROGRESS_FILL = 0.4f` (a fixed 40% bar, line 50); `:41`
  `StartingScreen`; `:68` `DidNotStartScreen` (Try again, Battery settings, Copy details).
- `K/data/HqServices.kt:16-22` `HqStatus { Live, Starting(seconds), Saved(age) }` (line 18 `Starting`).
- `java/example/hq/spike/HubService.java:49` holds `hqProc`; `:173` channel `"hub"` at Low; `:193-197`
  `notification()` has no content intent; the text depends only on the run mode.
- `K/ui/shell/` has no `ConnectionSheet.kt`.

### Before frames

- **B1 · Starting HQ, then the 60 s cover.** "Starting HQ" / "HQ is starting on this phone. The first
  start after a reboot can take up to a minute." (pin A) / a fixed bar / "12 s" (pin B) / "Have a
  thought? It is saved now and sent when HQ is up." [Capture]. "At 60 s, on every launch: HQ did not start
  · Health check timed out after 60 s. · Try again · Battery settings" (pin E).
- **B2 · The running notice** (generic shade). Active: "AgentHQ / HQ is running / Answering your chats and
  running scheduled tasks." (pin C). Standby: "HQ is running / Only the app is on. Chats and scheduled
  tasks are paused." "While HQ is still starting, or has stopped": "HQ is running / Answering your chats
  and running scheduled tasks." (pin D).

Pins: **A** a fixed 40% fill (`PROGRESS_FILL = 0.4f`); a slow start and a stuck one look the same. **B**
the seconds are real but give no scale. **C** no tap action (`HubService.java:193-197`). **D** always
"running": the text depends only on the run mode. **E** a 60 s cover on every launch
(`StartupWords.decide()`, `Startup.kt:24-28`); Try again restarts it, so a slow start never finishes (M9).

## 3. Decision

**Progress is the four steps the app can see for itself,** drawn as one 4 dp bar and a checklist; time is
shown as it is, next to what is usual on this phone. The very first launch shows them full screen, since
there is nothing saved yet. Every later start keeps the app usable on its saved copy and puts the steps
in the Connection sheet, reached from the status line.

**Stopped is a fact, not a timer:** the process is dead, or its step has not moved for max(60 s, 2 ×
usual). A slow phone opening a large vault is left to finish.

**The steps:** the hub service is up; hq's process has started; hq is alive but its port does not answer
yet (opening the vault and the database); the port answers but the health check is not yet good.

Rejected:

- **A time-based bar that slows near the end.** Still invented.
- **A spinner with no bar.** Honest but says nothing about how far along the start is.
- **Seconds in the notice.** A notice updated every second is noise and costs battery; it changes only
  with the step.
- **A cover at 60 s on every launch.** It hides a working saved copy and invites a restart that throws
  the start away.
- **Restart by itself after 60 s.** On a slow phone a large vault never finishes opening.

## 4. Screens and states (After)

Four steps the app can observe for itself; no change in HQ is needed. One cover, on the very first launch.

1. **The very first launch only: step 3 of 4.** Full screen: "Starting HQ" (pin 1) / "HQ is starting on
   this phone. The first start after a reboot can take up to a minute." (pin 2). Checklist: "Starting on
   this phone" (done), "Loading HQ" (done), "Opening your notes and tasks" (current, pulses), "Almost
   ready". One 4 dp blue bar filled to the steps done; "Step 3 of 4 · 12 s" (pin 3). "Have a thought? It
   is saved now and sent when HQ is up." [Capture] (kept as shipped).
2. **Every later start: no cover, the status line.** Notes place: status line "Starting HQ · 12 s" (pin
   4); strip "Saved on this phone at 14:32" (pin 5); the saved list "Weekly plan · Edited yesterday",
   "Ideas for the garden · Edited 3 days ago", "Reading list · Edited last week". Caption: "The status line
   is one 48 dp button that opens the Connection sheet (frame 3). The place keeps working on its saved
   copy."
3. **The Connection sheet: HQ starting.** Behind: "Notes · Starting HQ · 19 s". Sheet "HQ is starting" /
   "HQ runs on this phone. After a reboot it can take up to a minute. You can read and write while you
   wait." (one body for this board and FixOffline; `../README.md` "Still open") Facts: "HQ on this phone · Step 3
   of 4 · 19 s"; the checklist with the bar (pin 6); "Usually about 25 s" (pin 7); "Internet · Wi-Fi";
   "Saved on this phone at · 14:32".
4. **Slow, but HQ is alive and moving.** Behind "Starting HQ · 72 s". Sheet "HQ is starting": "HQ on this
   phone · Step 3 of 4 · 72 s", "Usually about 40 s". Amber line with a clock icon (pin 8): "Taking longer
   than usual. HQ is still starting." Then (pin 9) "HQ is still opening your notes. Restarting starts
   over." and buttons [Keep waiting] (primary) [Restart] (second, never green). "When free space is low,
   one more line" (pin 10): amber "The phone is almost full. Free up 120 MB and try again." / [Open
   Android settings] / "Android shows what you can remove."
5. **The sheet: HQ has stopped.** Status line "HQ has stopped" (amber dot with stop icon). Sheet "HQ has
   stopped" / "Android may have stopped it to save battery, or the phone ran low on memory. Nothing you
   wrote is lost." Facts: "HQ on this phone · Stopped", "Stopped at · Opening your notes and tasks",
   "Saved on this phone at · 14:32" (pin 11). Buttons: [Start HQ] (green), [Save a copy of my notes] (pin
   14), [Open Android settings] with "Then set Battery to Unrestricted.", "Technical details: Health check
   timed out after 60 s. · Last start 09:14 · exit code none" (pin 13), [Copy details] (pin 12).
6. **The running notice: rules section 1 words** (generic shade). It has no buttons on any version; a
   tap opens the app (`../../resolutions.md` D-2: FixOffline wins over this board's Standby action).
   - Active (pin 15): "AgentHQ / HQ is running / Answering your chats and running scheduled tasks."
   - Standby (pin 16): "HQ is running / Only the app is on. Chats and scheduled tasks are paused." No
     action.
   - Starting, or restarting (pin 17): "HQ is starting / Step 3 of 4 · Opening your notes and tasks";
     "HQ is restarting / A setting changed. It is back in a few seconds."
   - Stopped (pin 18): "HQ has stopped / Tap to see why."
7. **An owner with a server HQ.** Chat "Connected". Sheet "Connected" / "Your server HQ answers your
   chats. HQ on this phone is starting after a reboot." Facts: "Your server HQ · hq.example.ts.net ·
   Connected" (first), "HQ on this phone · Step 2 of 4 · Only the app is on", "Saved on this phone at ·
   14:32". Its running notice (pin 19): "HQ is running / Only the app is on. Your server HQ answers your
   chats." No action (D-2).
8. **Android 8 to 11.** "Android 8.0 and 8.1: the notice may show at Low" (pin 20): "HQ is starting /
   Step 2 of 4 · Loading HQ". "Check on device: does 8.0 or 8.1 raise a Min channel to Low for a
   foreground service?" What else differs: "Android 8 and 9 have no Active apps panel. Without the notice,
   the status line and the sheet are the only signs of HQ." "The hub starts from BootReceiver with
   startForegroundService on every version from 8 (API 26). Android 12's limits on starting from the
   background exempt BOOT_COMPLETED." "Reduced motion reads ANIMATOR_DURATION_SCALE on every version."
   "The 8.1 emulator is the slow start test: a large vault, a cold cache."
9. **360 dp wide, font scale 2.0** (pin 21). "Notes · Starting HQ · 19 s" (the status line wraps onto two
   lines), sheet "HQ is starting" / body / "Step 3 of 4" / "Opening your notes and tasks" / "Usually about
   25 s ↓" (the step list below the fold; the sheet scrolls).

## 5. Exact copy

First launch cover (title, body, Capture kept as shipped):

| Element | String |
|---------|--------|
| Step 1 | "Starting on this phone" |
| Step 2 | "Loading HQ" |
| Step 3 | "Opening your notes and tasks" |
| Step 4 | "Almost ready" |
| Count | "Step {n} of 4 · {s} s" |
| TalkBack | "Step {n} of 4, {step}" |

Status line (rules section 6):

| Element | String |
|---------|--------|
| States | "Connected" · "Starting HQ · {s} s" · "Restarting HQ · {s} s" · "HQ has stopped" |
| Strip | "Saved on this phone at {time}" |

Connection sheet:

| Element | String |
|---------|--------|
| Starting, title | "HQ is starting" |
| Starting, body | "HQ runs on this phone. After a reboot it can take up to a minute." |
| Facts | "HQ on this phone" · "Step {n} of 4 · {s} s" · "Internet" · "Saved on this phone at" |
| Usual | "Usually about {N} s" (from the second start) |
| Slow | "Taking longer than usual. HQ is still starting." |
| Alive, past 60 s | "HQ is still opening your notes. Restarting starts over." · "Keep waiting" · "Restart" |
| Almost full | "The phone is almost full. Free up {size} and try again." · "Open Android settings" · "Android shows what you can remove." |
| Stopped, title | "HQ has stopped" |
| Stopped, body | "Android may have stopped it to save battery, or the phone ran low on memory. Nothing you wrote is lost." |
| Stopped, facts | "HQ on this phone" · "Stopped" · "Stopped at" · "{step}" · "Saved on this phone at" · "{time}" |
| Buttons | "Start HQ" · "Save a copy of my notes" · "Open Android settings" · "Then set Battery to Unrestricted." · "Copy details" |
| Technical details | "Health check timed out after {s} s." · "Last start {time}" · "exit code {code or none}" |
| Server owner | "Your server HQ answers your chats. HQ on this phone is starting after a reboot." · "Your server HQ" · "{address} · Connected" |

The running notice, channel HQ is running (Min):

| State | String |
|-------|--------|
| Active | "HQ is running" · "Answering your chats and running scheduled tasks." |
| Standby | "HQ is running" · "Only the app is on. Chats and scheduled tasks are paused." (no action, D-2) |
| Standby, server | "HQ is running" · "Only the app is on. Your server HQ answers your chats." (no action, D-2) |
| Starting | "HQ is starting" · "Step {n} of 4 · {step}" |
| Restarting | "HQ is restarting" · "A setting changed. It is back in a few seconds." |
| Stopped | "HQ has stopped" · "Tap to see why." |
| Retired | "HQ is not answering" · "Tap to see why and try again." · "HQ did not start" (as a cover) · "Try again" · "Battery settings" · the Standby action "Background work" |

Strings move to resources with the rules section 2 keys (`fail_start_phone_full` and the rest).

## 6. Behaviour rules

1. **The cover** shows only until HQ has answered once on this install (`FirstLaunch`'s `first_done`,
   `Startup.kt:9-14`). It opens the app at the first good answer, as today.
2. **One 4 dp bar,** rounded, blue, filled to the steps done. It never moves backwards; if hq restarts
   mid-way it goes back to step 2 with no animation. The current step pulses, still when the phone asks
   for less motion.
3. **The steps** are read once a second: step 1 when the hub service is in the foreground, step 2 when
   hq's process is alive, step 3 while its port on 127.0.0.1 refuses a connection, step 4 once it accepts,
   done at the first good health answer.
4. **Every later start:** no cover. The status line reads the state first and wraps, never cut. It is a 48
   dp button that opens the sheet.
5. **The saved copy** stays on screen with its strip; actions that need HQ wait in the outbox (FixOffline).
6. **The sheet** holds the four steps, the bar and the seconds, live while it is open.
7. **"Usually about"** is the median of the last five starts that reached a good answer, rounded to 5 s;
   it appears from the second start on.
8. **Slow** past twice the usual time, and never before 30 s: one amber line with a clock icon, no new
   buttons.
9. **Alive past 60 s** but not stuck: "HQ is still opening your notes. Restarting starts over." Keep
   waiting closes the sheet; Restart is second, never green.
10. **Almost full:** under 200 MB free for the app's files, the catalogue line and Open Android settings,
    in any start state.
11. **Stopped** only when hq's process is dead (`Process.isAlive` false and not restarting), or its step
    has not moved for max(60 s, 2 × usual). The status line turns amber with a stop icon.
12. **The stopped sheet opens by itself** at most once a day; after that the status line opens it.
13. **Technical details** is the only place for the health check, the last start and the exit code. Copy
    details includes it and "Stopped at".
14. **Save a copy of my notes** opens safe mode, which needs no HQ (FixSafeMode).
15. **Every notice's tap** opens the app where Android's launcher would; Stopped opens the sheet.
16. **No buttons on the running notice** in any state or version (`../../resolutions.md` D-2). A tap opens
    the app; Background work stays in Settings (`hq://settings/background`).
17. **Starting and Restarting** replace the running text while HQ has not answered; they change only when
    the step does, with no sound and no seconds. Restarting only after the app asked for it.
18. **Stopped** follows rule 11, also with the app closed. Back to "HQ is running" at the next good answer.
    Never more than one notice.
19. **Server owners:** the sheet lists Your server HQ, with its name, above HQ on this phone; the Standby
    notice says the server answers.
20. **Android 8 to 11:** the same states; the channel level is a device check on 8.0 and 8.1.
21. **360 dp at font scale 2.0:** the status line wraps onto two lines, the sheet scrolls, the step list
    moves below the fold.

## 7. Android versions

| Version | What this fix does there |
|---------|--------------------------|
| 8 and 9 | No Active apps panel. The hub's channel moves to Min under a new id; 8.0 and 8.1 may show a foreground notice at Low anyway (check on device). The slow start rule is tested on the 8.1 emulator. |
| 10 and 11 | As 8 and 9 for the notice. The sheet's Open Android settings opens the app's battery page where the phone has one. |
| 12 | Every PendingIntent on the notice is `FLAG_IMMUTABLE`. Starting the hub from the background is limited, but `BOOT_COMPLETED` and a tap in the app are allowed. |
| 13+ | Without notification permission the notice shows only in Android's Active apps panel; the status line and the sheet do not depend on it. Android 14+ lets the owner swipe the notice away; the next state change posts it again. |

Android constraints, as checked on the board:

- A foreground service's notice takes `setContentIntent` and actions like any other; from Android 12
  every PendingIntent needs `FLAG_IMMUTABLE` (or MUTABLE), and these are immutable.
- Updating the notice is `notify()` with the same id and `setOnlyAlertOnce(true)`. The channel is made at
  `HubService.java:173` as Low; Min needs a new channel id.
- Without notification permission on Android 13+, the notice shows only in Android's Active apps panel.
- Android 14+ lets owners swipe a foreground notice away; the service keeps running and the next state
  change posts it again.
- The port check is a local TCP connect with a 300 ms timeout, once a second; the hub already holds hq's
  Process (`HubService.java:49`) and can ask `isAlive()`.
- Free space: `StorageManager.getAllocatableBytes()` (API 26) on the app's files directory;
  `ACTION_MANAGE_STORAGE` opens Android's free-up-space screen.
- Reduced motion: the pulse follows Settings, Accessibility, Remove animations (the animator duration
  scale).

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `K/ui/startup/Startup.kt` | `decide()` (lines 24 to 28) returns the cover only while `first_done` is false, and never `DidNotStart` after that; `GIVE_UP_SECONDS` (line 22) gives way to the stuck rule; `failedLines()` (line 39) moves into Technical details; a `StartStep` enum, `usual()`, the slow rule. | yes |
| `K/ui/startup/StartupScreens.kt` | `StartingScreen` (line 41) drops `PROGRESS_FILL` (line 36) for the 4 dp bar and the checklist; `DidNotStartScreen` (line 68) is retired for later launches. | yes |
| `K/data/HqServices.kt` | `HqStatus.Starting(seconds)` (line 18) gains the step; new `Restarting` and `Stopped` from the stuck rule; records start durations (last five). | yes |
| `K/ui/connection/ConnectionSheet.kt` | Built here in wave 2 (`../../resolutions.md` D-5): the steps, usual, Stopped at, the almost-full line, Save a copy of my notes. Reads `data/Connection` and `ConnectionWords` from wave 0. Wave 4 adds saved copies and server rows. | NEW |
| `java/example/hq/spike/HubService.java` | Expose the step (service up, child alive, port probe); `notification()` (lines 193 to 197) gains the content intent and the state words, with no action (D-2), posted on changes only; the channel moves to Min under a new id (line 173). | yes |
| `res/values/strings.xml` | Strings with the rules section 2 keys (`fail_start_phone_full` and the rest). | yes |

## 9. Server, release or desktop changes

None required (the four steps are app-side). The `start_cover_removal` kill switch in the signed channel
file (`../wave-0/02-safe-mode.md`, rules section 10) can bring the cover back if `start_hq.while_running`
climbs (`../wave-0/04-measure.md`).

## 10. Acceptance checks (each fails on today's build)

1. Second launch with hq held from starting: no full-screen cover at any time, even past 60 s; the status
   line reads "Starting HQ · {s} s" and opens the sheet with the four steps.
2. 8.1 emulator, a vault fixture that takes 90 s to open, usual 40 s: at 60 s the sheet says "HQ is still
   opening your notes. Restarting starts over.", HQ finishes by itself, and Stopped never shows.
3. Kill hq's process: within 2 s the status line reads "HQ has stopped" in amber with the stop icon and
   the sheet opens once. Kill it again the same day: the sheet does not open by itself.
4. With the app closed: a reboot shows "HQ is starting" with the step; Restart HQ in Settings shows "HQ is
   restarting"; a killed hq shows "HQ has stopped", and a tap opens the sheet.
5. Fill the phone to under 200 MB free: the sheet shows "The phone is almost full. Free up {size} and try
   again."
6. Fresh install: the cover shows the four steps on one 4 dp blue bar that never goes back.
7. From the second start, the sheet shows "Usually about {N} s", the median of the last five.
8. Android's page for the app shows the "HQ is running" category as Min (Silent, folded away).
9. The running notice has no action button in Active, Standby, Starting, Restarting or Stopped, on
   Android 8.1, 10, 12 and 14; a tap opens the app (D-2).
9. The stopped sheet has no health check or exit code outside Technical details, and its buttons read
   Start HQ, Save a copy of my notes, Open Android settings.
10. 360 dp emulator at font scale 2.0: the status line wraps and the state is never cut; the sheet
    scrolls.
11. Signed in to a server HQ: the sheet lists Your server HQ with its name above HQ on this phone.
12. TalkBack reads each step once; with Remove animations on, the bar does not pulse.

## 11. Check on device

- 8.0 and 8.1: a Min channel on a foreground notice, shown as Min or raised to Low.
- The Galaxy A32, a Tecno, Infinix or itel phone and an Android Go phone: the usual start time with a
  2,860-note vault, to set the fixtures for check 2.
- The 200 MB free-space line: high enough that HQ can still open its database.

## 12. Open questions and decisions it depends on

- **Should HQ report its own start phases** (database migrations, vault check) for a finer bar?
  Recommendation: later. The four app-side steps need no Rust change. (Decided: D15, "Later; Stopped by
  progress now.")
- **New: when is the phone "almost full"?** Recommendation: under 200 MB free for the app's files
  (`getAllocatableBytes`), with {size} the amount up to 200 MB. Tune it with the "What HQ noticed"
  counters. (Decided: N15.)
- **Decided (rules section 6):** the stopped sheet opens by itself at most once a day.

Depends on: D15, N15, `../../resolutions.md` A7, D-2, D-5, rules sections 1, 2, 6; `../wave-4/01-offline.md` (the Connection sheet and status
line); `../wave-0/02-safe-mode.md` (Save a copy of my notes); `../wave-0/04-measure.md`
(`start_hq.while_running`).
