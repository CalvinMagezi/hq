# Wave 0 · 04 · What HQ noticed (M16)

Board: **FixMeasure** ("Fix M16: what HQ noticed"), wave 0, added after the review, 5 Nov 2026.
Area: Settings › Backup and about › Technical details › What HQ noticed.
Shared rules: `../../rules-shared.md` section 10 (counters and roll-back) wins.

## 1. Finding and the review items it closes

**The problem.** Nothing on the phone counts anything. When a wave ships, nobody can tell whether more
owners allow notifications, whether Undo is pressed because the action was a mistake, or whether people
tap Start HQ while HQ is already running. "Copy diagnostics" gives a build number, the run mode and HQ's
raw health answer, nothing more.

**Who it hurts, how often.** Every owner, after every release, because a fix that makes things worse
stays in. The answer must not be telemetry: HQ is local-first and private. So the counts stay on the
phone, hold no content, are never sent, and the owner sees exactly what is counted. They leave the phone
only when the owner copies them.

Closes:

- **M16:** on-phone counters with no content, never sent, included in Copy details.
- **Rules section 10:** the counter list and the roll-back rules, each tied to a counter and a switch in
  the signed channel file.
- **FixPlainWords:** "Copy diagnostics" becomes "Copy details", the same words as the Connection sheet.

## 2. Before (as built, verified at `c04a100`)

- `ui/settings/SettingsScreens.kt:992` the Support row "Copy diagnostics".
- `ui/settings/SettingsWords.kt:417-418` `diagnostics(build, active, health, port)` builds the clipboard
  text. It is copied without being shown.
- No counter code exists anywhere in `apps/android`.

### Before frames

- **B1 · Backup and about, Support.** "About": "AgentHQ · hub · build 1", "Run mode · Active", "HQ ·
  0.9.1". "Support": "Copy diagnostics", "Restart HQ". Pin A.
- **B2 · What "Copy diagnostics" copies:**
  ```text
  AgentHQ hub build 1
  Run mode: Active: relays and scheduler on.
  Serving this device on 127.0.0.1:5678
  Health: {"status":"ok","version":"0.9.1","uptime_s":5123, ...}
  ```
  "From `SettingsWords.diagnostics()`, line 418. Pasted without a look at what it holds." Pin B.

What goes wrong: **A** nothing is counted (no record of grants, Undo taps, locked presses, data spent;
each fix's rollout has no way to say "this made it worse"). **B** what is copied is not shown (run mode,
a port and a raw health answer; nothing about whether the app works). Telemetry is not the answer: HQ
sends nothing about its owner anywhere.

### What every counter must be

- **A number,** from a fixed list compiled into the app. No ids, titles, names, paths, addresses or
  times of day.
- **Kept on this phone** in `noBackupFilesDir`, so no copy or transfer takes it (`03-data-transfer.md`).
- **Never sent.** No code path posts it. It leaves the phone only when the owner taps Copy details and
  pastes it somewhere.
- **Shown,** each with one line saying what it counts.

## 3. Decision

**A fixed list of counters, kept on the phone, shown on one page, and copied only with Copy details
after the owner reads it.** Each counter is a number tied to one fix and one roll-back rule. Two columns,
this version and the last, make a release's effect visible without a server.

Rejected:

- **Send anonymous counts to a server.** Even with no content it is telemetry, needs a server, and
  breaks "HQ sends nothing about you".
- **An event log with times.** Times of day show habits. Counts per version are enough to compare
  releases.
- **Counters inside HQ's database (`vault.db`).** They would move to a new phone and into backups, and
  need HQ to run. The phone app owns these.
- **Hide the page from owners.** A count the owner cannot see is a count they cannot trust.

## 4. Screens and states (After)

**One page under Technical details. This version and the last one, side by side.** Counter rows are a
label on the left and two right-aligned number columns.

1. **Backup and about.** "About": "AgentHQ · Build 1", "Background work · On", "HQ version · 0.9.1".
   "App updates": "Up to date · AgentHQ 0.9.1". "Support": row "Technical details · Versions, what HQ
   noticed, Copy details" (pin 1), row "Restart HQ".
2. **Technical details.** Key/value: "HQ version · 0.9.1", "Build · 1", "Android · 14", "HQ ·
   Connected · started at 08:41", "Your server HQ · 0.9.3 · Connected". Row "What HQ noticed · Counted
   on this phone. Never sent." (pin 2). Row "Copy details · Shows you the text first" (pin 3). Note:
   "Send these to whoever helps you with HQ. Nothing here holds your notes, chats or keys."
3. **What HQ noticed, top.** Intro (pin 4): "HQ counts a few things on this phone, so you can tell
   whether a change helped. **Only numbers. Nothing is sent.**" Column header (pin 5): "Since 3 Nov ·
   0.9.1 · 0.9.0".
   - Notifications: "Allowed at first run 1 0", "Allowed later 0 0", "Not allowed 0 1".
   - Phone assistant: "Chosen as assistant Yes No", "Locked, then unlocked 4 0", "Locked, then cancelled
     1 0".
   - "HQ answered" notices: "Posted 12 9", "Tapped 7 2", "From your server HQ, tapped 3 of 5 · 0 of 2".
4. **What HQ noticed, further down.**
   - "Data used · mobile · Wi-Fi, MB": "App updates 0 · 41 / 0 · 38", "Speech model 0 · 57 / 0 · 0",
     "Server HQ checks 6 · 2 / 9 · 3", "Everything else, about 18 · 210 / 22 · 180" (pin 6).
   - "Undo · taps of actions": "Task discarded 2 of 31 / 1 of 20", "Note deleted 0 of 4 / 0 of 2",
     "Helper stopped 1 of 3 / 0 of 1" (pin 7).
   - "Starting": "Start HQ tapped while it ran 0 / 3", "HQ closed while opening 0 / 0", "Copies of notes
     saved 1 / 0" (pin 8).
   - "Saved copy, age when shown · Under 1 hour · up to 1 day · older": "12 · 3 · 1 / 8 · 0 · 0".
5. **What HQ noticed, the end.** The saved-copy line again; row "Count on this phone · On. Numbers stay
   here." with a switch; row "Clear these numbers · Counting starts again from now" (pin 9). Foot: "Tap
   any line to see what it counts. When the app updates, this version's numbers move to the right-hand
   column."
6. **Tap a line: what it counts.** A bottom sheet over the page from the row "Task discarded 2 of 31 / 1
   of 20": title "Task discarded", "How many times you tapped Undo after discarding a task, out of all
   the tasks you discarded." and "Only the two numbers are kept. Not which task, not when." [Done] (pin
   10).
7. **Copy details, shown first.** Sheet "Copy details / This is what goes on the clipboard. Read it
   before you send it." A monospace block (pin 11):
   ```text
   AgentHQ 0.9.1 (build 1) on Android 14
   Background work: On
   HQ: Connected, started at 08:41
   Your server HQ: 0.9.3, Connected
   What HQ noticed (0.9.1 since 3 Nov | 0.9.0)
   notifications.allowed_first_run 1 | 0
   notifications.allowed_later 0 | 0
   notifications.not_allowed 0 | 1
   assistant.chosen yes | no
   locked_press.unlocked 4 | 0
   locked_press.cancelled 1 | 0
   answer_notice.posted 12 | 9
   answer_notice.tapped 7 | 2
   answer_notice.server_tapped 3/5 | 0/2
   data_mb.app_updates 0/41 | 0/38
   data_mb.speech_model 0/57 | 0/0
   data_mb.server_checks 6/2 | 9/3
   data_mb.other 18/210 | 22/180
   undo.task_discarded 2/31 | 1/20
   undo.note_deleted 0/4 | 0/2
   undo.helper_stopped 1/3 | 0/1
   start_hq.while_running 0 | 3
   start.closed_while_opening 0 | 0
   safe_copy.saved 1 | 0
   saved_copy.age 12/3/1 | 8/0/0
   ```
   Buttons [Copy] (green) [Not now].
8. **Clear these numbers.** Dialog "Clear these numbers? / Both columns go back to zero. Counting starts
   again from now." [Keep them] (left) [Clear numbers] (right, red, pin 12).
9. **Counting off.** Top note (pin 13): "Counting is off. These numbers stopped on 5 Nov. Copy details
   now says "counting is off"." The rows stay; "Count on this phone · Off".
10. **360 dp, font scale 2.0** (pin 14). "Only numbers. Nothing is sent.", group "Notifications ·
    0.9.1", "Allowed at first run 1", "Allowed later 0" with the last version's "0" moved under the
    line. Caption: "0.9.0 moves under each line."

### Roll-back rules (rules section 10): each signal, its counter, and what is pulled

| Signal | Counter | Threshold | What is pulled |
|--------|---------|-----------|----------------|
| Crash on start after a release | `start.closed_while_opening`, and "HQ did not start" from the Connection state | Any device-check phone with 2 in a row, or one owner's Copy details, within 7 days of a release, where the last version had none | The version: added to `withdrawn`, rollout 0 (`02-safe-mode.md`) |
| Start HQ tapped while HQ is alive | `start_hq.while_running` | More than 2 a week on any test phone | Switch `start_cover_removal` to false: the start cover comes back while the sheet is fixed |
| Undo used often for one kind | `undo.{kind}` | More than about 20% of at least 10 actions of that kind | That kind gets its confirm dialog back in the next release. The release that removed it goes to rollout 0 until then |
| Locked presses cancelled | `locked_press.cancelled` against `.unlocked` | More cancelled than unlocked over 10 presses | Switch `lock_screen_assistant` to false |
| Data on mobile | `data_mb.server_checks`, `.other` | Server checks over 3 MB a day on mobile data | Slow the server poll on mobile data (M10), no switch needed |
| Notifications refused | `notifications.*` | "Not allowed" above "Allowed at first run" across the five test owners | Nothing pulled. Step 4's words are rewritten and tested again |

Counts never leave a phone by themselves. They reach the people making HQ only through the device-check
phones (the A32, a Tecno, Infinix or itel phone, an Android Go phone, the 8.1 and 10 emulators) and the
five test owners (M13), who paste Copy details by their own choice.

## 5. Exact copy

| Where | String |
|-------|--------|
| Row | "Technical details" · "Versions, what HQ noticed, Copy details" |
| Row | "What HQ noticed" · "Counted on this phone. Never sent." |
| Intro | "HQ counts a few things on this phone, so you can tell whether a change helped. Only numbers. Nothing is sent." |
| Columns | "Since {date}" · "{this version}" · "{last version}" |
| Switch | "Count on this phone" · "On. Numbers stay here." / "Off" |
| Clear | "Clear these numbers" · "Counting starts again from now" |
| Dialog | "Clear these numbers?" · "Both columns go back to zero. Counting starts again from now." · "Keep them" · "Clear numbers" |
| Off note | "Counting is off. These numbers stopped on {date}. Copy details now says "counting is off"." |
| Foot | "Tap any line to see what it counts. When the app updates, this version's numbers move to the right-hand column." |
| Copy details sheet | "Copy details" · "This is what goes on the clipboard. Read it before you send it." · "Copy" · "Not now" |
| Page note | "Send these to whoever helps you with HQ. Nothing here holds your notes, chats or keys." |
| Copy details row | "Copy details" · "Shows you the text first" |
| Line sheet | "Only the two numbers are kept. Not which task, not when." · "Done" |
| Block | Keys and numbers as frame 7, one per line, English keys so anyone can read them. |
| Android 8 to 12 | "Not asked on this Android" (the notification lines, no permission before 13) |

Counters (key · label · what it counts, shown on tap):

| Key | Label | What it counts |
|-----|-------|----------------|
| `notifications.allowed_first_run` | Allowed at first run | Times you tapped Allow on Android's notification question during first run. |
| `notifications.allowed_later` | Allowed later | Times you allowed notifications after first run, from a Turn on button. |
| `notifications.not_allowed` | Not allowed | Times you tapped Don't allow. Closing the question without an answer is not counted. |
| `assistant.chosen` | Chosen as assistant | Whether HQ is your phone's assistant now. |
| `locked_press.unlocked` | Locked, then unlocked | Times you held Home on a locked phone and then unlocked to see HQ. |
| `locked_press.cancelled` | Locked, then cancelled | Times you held Home on a locked phone and then closed it without unlocking. |
| `answer_notice.posted`, `.tapped` | Posted · Tapped | "HQ answered" notices posted, and how many you tapped. |
| `answer_notice.server_tapped` | From your server HQ, tapped | Server notices tapped, out of all posted. |
| `data_mb.{purpose}` (`app_updates`, `speech_model`, `server_checks`, `other`) | App updates · Speech model · Server HQ checks · Everything else, about | Megabytes on mobile data and on Wi-Fi. |
| `undo.{kind}` (`task_discarded`, `note_deleted`, `helper_stopped`) | Task discarded · Note deleted · Helper stopped | Times you tapped Undo, out of all those actions. |
| `start_hq.while_running` | Start HQ tapped while it ran | Times you tapped Start HQ while HQ was in fact running. |
| `start.closed_while_opening` | HQ closed while opening | Times the app closed during its start (`02-safe-mode.md`). |
| `safe_copy.saved` | Copies of notes saved | Copies saved from safe mode. |
| `saved_copy.age` | Under 1 hour · up to 1 day · older | How old the copy saved on this phone was each time it was shown. |

The task-discarded line sheet text (frame 6): "How many times you tapped Undo after discarding a task, out
of all the tasks you discarded."

## 6. Behaviour rules

1. **Technical details** replaces Copy diagnostics in Support. Restart HQ stays.
2. **What HQ noticed** reads only the counter file. It works with HQ stopped.
3. **Copy details** always opens the sheet first (frame 7). The Connection sheet's Copy details uses the
   same sheet.
4. **The intro** stays at the top. It is the promise, so it is never hidden.
5. **Two columns:** this version since its install date, and the last version. A new version moves the
   left column right and starts again.
6. **Data used** counts bytes the app itself reads for updates, the speech model and server checks, by
   network (metered or not). "Everything else" is the app's whole use, from Android's per-app count,
   minus those three, so it says "about".
7. **Undo** counts two numbers per kind: shown and tapped. Only kinds that act at once with Undo
   (FixDestructive).
8. **Start HQ while running** is counted when the tap comes while the hub's child process is alive.
9. **Count on this phone** off stops every counter at once. Numbers stay until cleared.
10. **Every line** opens a sheet with what it counts, in the words of the table.
11. **Copy** puts the shown text on the clipboard. It is not marked sensitive, so on Android 13+
    Android's own preview shows it.
12. **Clear** asks first. Back and a tap outside keep the numbers.
13. **Counting off** says so at the top and in Copy details.
14. **At 360 dp and font scale 2.0** one column shows, and the last version's number moves under each
    line.

## 7. Android versions

| Version | Behaviour |
|---------|-----------|
| 8 and 9 | Counting works the same. The notification lines read "Not asked on this Android" (no permission before 13). Undo shown counts use the 10 s window. |
| 10 and 11 | The same. Locked presses count once the locked path exists (wave 3). |
| 12 | The same. |
| 13+ | The three notification counters apply. The clipboard shows Android's own preview after Copy. |

Android constraints, as checked on the board:

- `noBackupFilesDir` is never backed up or moved, on every version from API 21.
- The app's own data use comes from `TrafficStats` for its UID, which includes the hq child and the
  WebView. Per purpose only for bytes the app reads itself.
- Writes go to a temporary file and are renamed into place, so a killed process never leaves a half
  file. If the file is unreadable, it starts again at zero.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `kotlin/.../measure/Counters.kt` | `enum class Counter` (the keys), `bump()`, `addBytes(purpose, metered, n)`, two version columns, atomic write to `noBackupFilesDir/counters.json`, 8 KB cap. Imports nothing from `java.net` or OkHttp. | NEW (package `measure/` is new) |
| `kotlin/.../ui/firstrun/FirstRunViewModel.kt` | Grants at first run. | yes |
| `NotifAccess` | Grants after first run. | NEW: no `NotifAccess` exists today; it is the helper FixNotifications introduces (`wave-1/01-notifications.md`) |
| `kotlin/.../voice/Assistant.kt`, `voice/VoiceActivity.kt` | Locked presses. | yes |
| `kotlin/.../AlertPoster.kt` | Posted. | yes |
| `kotlin/.../ShellActivity.kt` | Tapped, from the app's own PendingIntent. | yes |
| `kotlin/.../update/Updater.kt` | `download()` (line 168): bytes. | yes |
| `kotlin/.../voice/VoiceModels.kt` | Bytes. | yes |
| `java/example/hq/spike/Alerts.java` | `pollServer()` (line 127): bytes. | yes |
| Undo snackbars, the Connection sheet's Start HQ, the saved-copy strip, safe mode | Call sites. | partly NEW (sheet, strip, safe mode come from other boards) |
| `kotlin/.../ui/settings/SettingsScreens.kt` | Technical details and What HQ noticed pages; line 992 "Copy diagnostics" goes. | yes |
| `kotlin/.../ui/settings/SettingsWords.kt` | `diagnostics()` (line 418) becomes the Copy details builder with the block. | yes |
| `app/src/test/kotlin/.../measure/CountersTest.kt` | Keys only from the enum, values whole numbers, the file holds no other strings than version and date; the Copy details text over a fixture. | NEW |

## 9. Server, release or desktop changes

None. The switches the roll-back rules flip (`start_cover_removal`, `lock_screen_assistant`) and
`withdrawn` / `rollout` live in the signed channel file (`02-safe-mode.md`).

## 10. Acceptance checks (each fails on today's build)

1. Settings › Backup and about › Technical details › What HQ noticed opens with HQ stopped and lists
   exactly the counters on this board.
2. Fresh install on Android 13, Allow at step 4: "Allowed at first run" reads 1. Another install,
   dismiss the question: nothing is counted. Tap Don't allow: "Not allowed" reads 1.
3. Discard a task, tap Undo: "Task discarded" reads "1 of 1". Discard another, let Undo time out: "1 of
   2".
4. Download the speech model on Wi-Fi: "Speech model" reads 0 · 57, give or take 1.
5. A fixture vault with distinctive note titles, task names, a helper session name, a server address and
   key names: Copy details holds none of them (string search over the copied text).
6. A strict network policy test: no counter code makes a network call. A source check: nothing under
   `data/`, `update/` or `bridge/` imports `Counters` for reading.
7. A real phone-to-phone copy (FixDataTransfer check 1): `counters.json` is not on the new phone.
8. Update from 0.9.1 to 0.9.2: the 0.9.1 numbers are in the right column, the left starts at zero.
9. Count on this phone off: numbers stop, Copy details says "counting is off". Clear numbers: both
   columns read zero.
10. Tap Start HQ in the Connection sheet while HQ is alive: "Start HQ tapped while it ran" reads 1.

## 11. Check on device

- None named on the board beyond the shared device matrix. The counters themselves are how the other
  boards' device checks are read (paste Copy details from each device-check phone).

## 12. Open questions and decisions it depends on

Open questions for the owner, with the board's recommendation:

- **Counting on by default?** Recommendation: on. Numbers only, never sent, one switch to stop it.
  (Decided: N8, "On by default, one previous version kept.")
- **How many past versions to keep?** Recommendation: one. Two columns fit a phone; more needs a table.
  (Decided: N8.)
- **Should test owners be asked to paste Copy details each week?** Recommendation: yes, the five test
  owners only, by message from a person. The app never asks.

Depends on: N8, rules section 10; FixSafeMode (`withdrawn`, rollout, switches); FixDestructive (the Undo
kinds); FixNotifications (`NotifAccess`, the step 4 grant).
