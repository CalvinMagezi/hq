# Wave 0 · 02 · Safe mode, the idle gate and a channel file that can take a version back (R3)

Board: **FixSafeMode** ("Fix R3: safe mode"), wave 0, added after the review, 5 Nov 2026.
Shared rules: `../../rules-shared.md` (sections 2, 4, 6, 10 win).

## 1. Finding and the review items it closes

**The problem.** If an update stops HQ from starting, the owner has no way out that keeps their notes.
Android refuses to go back to an older version. The vault lives in the app's private folder
(`files/hqhome/.vault`, `Restore.kt:71`), which uninstalling deletes. The app cannot set up backups
yet. So the only advice that works, "uninstall and install again", deletes everything.

**Who it hurts, how often.** Rarely, but totally. Any owner without a backup on a computer, after any
release that breaks the start (or the app's first screen). Skipping Android's install confirm (D11) and
test versions would make that release reach phones faster. The signed channel file has no way to pull a
bad version back, and an update can install in the middle of a chat or a backup.

Closes:

- **R3** (blocker): a screen that needs no HQ, "Save a copy of my notes", through Android's file picker.
- **The idle gate:** an update installs only when no chat, backup, outbox send or download is running.
- **Withdrawn and staged rollout** as signed fields in the channel file, signed again at promote.
- **D11** stays off, with the conditions that turn it on (rules section 10).
- **FixBackup D1:** "Save a copy now" is this same screen (`wave-1/02-backup.md`).

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/kotlin/example/hq/spike/` unless noted.

- `ui/firstrun/Restore.kt:71` `val home = File(app.filesDir, "hqhome")`; the vault is
  `files/hqhome/.vault` (the swap at `Restore.kt:124-136`).
- `ui/startup/Startup.kt:34` `FAILED_TITLE = "HQ did not start"`; line 41 the detail lines "Health
  check timed out after $GIVE_UP_SECONDS s." and "Last start: $start · exit code …";
  `ui/startup/StartupScreens.kt:82,85,88` the only buttons: "Try again", "Battery settings", "Copy
  details".
- `update/Updater.kt:89` `install()` calls `commit(context, apk)` at line 99; `commit()` at line 200
  commits the session at once (`session.commit`, line 210). Nothing checks whether HQ is busy.
- `update/UpdateLogic.kt:8` `data class ChannelManifest(version, versionCode, url, sha256, minSdk)`;
  `verdict()` at line 92 knows no withdrawn list or rollout.
- `scripts/release/android-channel.sh` writes `{ version, versionCode, url, sha256, minSdk }`;
  `scripts/release/sign.sh` signs.
- `.github/workflows/promote.yml:88-100` verifies the release's `android-channel.json` and uploads it
  to `channel-stable` byte for byte (`gh release upload … --clobber`), so nothing can be added at
  promote.
- `java/example/hq/spike/HubService.java:142` `holdHq()` and `:161` `releaseHq()` (the board cites
  lines 142 to 162).
- `res/xml/shortcuts.xml` has three static shortcuts: `capture`, `new_task`, `ask_hq`.
- `AndroidManifest.xml:16` `android:allowBackup="false"`, no `hasFragileUserData`, no Application class.
- `capture/Outbox.kt:76` the outbox is `filesDir/outbox.json`.

### Before frames

- **B1 · HQ did not start.** Title "HQ did not start"; body "Android may have stopped it to save
  battery, or the phone ran low on memory. Nothing you captured is lost."; detail "Health check timed
  out after 60 s." and "Last start: 09:14 · exit code 101"; buttons Try again (green), Battery
  settings, Copy details. Pin A.
- **B2 · Android's App info, the only other way.** "AgentHQ", Uninstall, Force stop (pin B), "Storage
  and cache · 1.2 GB used in internal storage", "Notifications · About 3 notifications per day".
- **B3 · Install update, any time.** Backup and about › Updates › "Install update · Version 0.9.2 is
  ready" (pin C). Caption: "Tapped while HQ answers a chat or writes a backup, the install starts at
  once. Android ends HQ to replace the app."
- **B4 · `android-channel.json` today** (`scripts/release/android-channel.sh`):
  ```json
  { "version": "0.9.2", "versionCode": 92,
    "url": "https://github.com/<owner>/<repo>/releases/download/v0.9.2/agent-hq-android.apk",
    "sha256": "<64 hex>", "minSdk": 26 }
  ```
  Pin D.

What goes wrong: **A** no way to save anything (the notes are on the phone, readable by the app, and
nothing copies them out). **B** the way out deletes the notes (uninstall removes `files/hqhome`;
Android will not install an older version over a newer one). **C** installs cut work off
(`Updater.install()` commits at once; a chat turn, a backup or an outbox send dies mid-way). **D** a bad
version cannot be pulled (no withdrawn list, no rollout share, no sequence number, no switches; promote
copies the signed file).

## 3. Decision

**A plain Kotlin screen in its own process that writes one .zip through Android's file picker.** It
reads only files on the phone and calls no HQ route, so it works when HQ is dead, when the main app
crashes, and with no network. The owner reaches it from the stopped Connection sheet, a launcher
shortcut, Backup and about, the copy-first card before an update, and by itself after two crashed
starts.

Why one .zip: one picker, one file to move, fast on a slow phone. It opens on any computer, and Restore
reads it back.

Rejected:

- **A locked .hqbak through `hq backup`.** Needs the hq binary to run, which is what may be broken, and
  a key the owner has not made.
- **A folder of loose files (`ACTION_OPEN_DOCUMENT_TREE`).** Every file is a separate call to the
  provider: thousands of notes take many minutes and stop half way.
- **A screen inside the main app process.** If the app itself crashes at start, the screen dies with it.
- **Rely on Google's cloud backup.** Off by design (`03-data-transfer.md`), capped at 25 MB per app, and
  plain notes leave the phone.

## 4. Screens and states (After)

### Two ways in that need no HQ

1. **Connection sheet: HQ has stopped.** Title "HQ has stopped"; body "Android may have stopped it to
   save battery, or the phone ran low on memory. Nothing you wrote is lost." Rows: "HQ on this phone ·
   Stopped" (amber dot with stop icon, rules section 3), "Stopped at · Opening your notes and tasks",
   "Saved on this phone at · 14:32". Buttons stacked: "Start HQ" (green, primary), "Save a copy of my
   notes" (outlined, pin 1), "Copy details" (text, blue).
2. **Long press on the app icon.** Launcher menu (generic): "Save notes" (pin 2), "New note", "New
   task", "Ask HQ". App label "AgentHQ".

### Safe mode screens (plain Android views, process `:safe`)

3. **What is in the copy.** Title "Save a copy of my notes" (pin 3). Body "This works even when HQ
   cannot start. Your notes go into one file, in a place you choose." Group "In the copy" (pin 4):
   "Your notes, with their pictures and files", "Tasks, chats and what HQ remembers, in one database
   file", "Notes and tasks waiting to send". Group "Not in the copy": "Your keys. Android locks them to
   this phone, so they cannot be copied.", "Settings, sign-ins and paired devices". Size line (pin 5):
   "About **340 MB** · 2,860 files". Lock line: "The copy is not locked with a password. Anyone with the
   file can read your notes." Buttons: "Choose where to save" (green, primary), "Not now".
4. **Android's file picker** (generic): "Save to Downloads", existing "Receipts" (Folder), "HQ notes
   2026-10-01 0915.zip · 301 MB · 1 Oct", "ticket.pdf · 212 kB · 28 Oct"; the file name field "HQ notes
   2026-11-05 1432.zip"; [Save] (pin 6).
5. **Saving.** Title "Saving your notes"; "1,240 of 2,860 files done" (pin 7); a 4 dp blue progress bar;
   "45% · 153 of 340 MB"; "About 2 min left". Lines: "Keep this screen open until it is done." and "HQ
   paused for a moment to copy the database. It starts again when the copy is done." Button "Stop
   copying" (outlined).
6. **Done** (pin 8). Title "Your notes are saved". Key/value rows: "File · HQ notes 2026-11-05
   1432.zip", "Size · 338 MB · 2,860 files", "Saved · In Downloads at 14:36". Lines: "Move it off this
   phone, to a computer or a USB stick. A copy on this phone is lost with the phone." and "Restore on
   this phone or a new one opens this file too." Button "Done".

### Low space, a place that goes away, a cancelled picker, Stop, server HQ, narrow phone

7. **Not enough space, before writing** (pin 9). Amber failure line with icon: "Couldn't save a copy of
   your notes." / "The phone is almost full. Free up 230 MB and try again." Note: "The copy needs about
   340 MB. Downloads has 120 MB free. A USB stick or memory card works too." Buttons "Choose another
   place", "Not now".
8. **The place stopped answering** (pin 10). "Couldn't save a copy of your notes." / "The place you
   chose stopped answering. Choose another place and try again." Note: "The unfinished file was
   removed. Nothing on the phone changed." Buttons "Choose another place", "Not now".
9. **Back from the picker.** Frame 3 with the line "Nothing was saved." (pin 11) under the body. Variant
   "Or, after the app was closed during a copy:" "The last copy did not finish. The unfinished file was
   removed." Buttons "Choose where to save", "Not now".
10. **Stop copying.** Over frame 5, dialog "Stop copying?" / "The part saved so far is removed." /
    [Keep copying] (left) [Stop copying] (right, red `#FF6B6B`, black text; pin 12).
11. **Server HQ owner** (pin 13). Title "Your notes are on your server HQ"; body "This phone keeps no
    notes of its own, so there is nothing here to copy. Backups run on your server HQ." Group "On this
    phone": "2 notes waiting to send" / "They go to your server HQ when it can be reached. You can save
    them now." Buttons "Save 2 waiting notes", "Not now".
12. **360 dp, font scale 2.0** (pin 14). "Save a copy of my notes", "This works even when HQ cannot
    start.", the list wraps ("Your notes, with their pictures and files", "Tasks, chats and what HQ
    remembers"), the page scrolls, "Choose where to save" stays at the bottom.

### Updates wait for HQ to be free, offer a copy first, and a bad version can be pulled

13. **Install tapped while HQ is busy.** Backup and about › App updates › "Install update · AgentHQ
    0.9.3 is ready · 41 MB". Dialog "Install now?" / "HQ is answering a chat. Installing now stops it."
    / [Wait until it's done] (left) [Stop the chat and install] (right, red; pin 15). The label is over
    18 characters, so per rules section 4 these stack, safe on top.
14. **Waiting, with a copy first.** Backup and about: a surface-100 card with a grey (line) border, 12
    dp radius, 16 dp padding, white 16/22 bold title "Save a copy of your notes first", body "Backups are
    not set up. If this update goes wrong, a copy keeps your notes safe.", a green pill [Save a copy]
    (pin 16). Group "App updates": row "Installs when HQ is free", secondary line in blue "Waiting for 1
    chat to finish" (pin 17), under it a 4 dp blue progress bar; a text button "Not now". Note: "Android
    then asks you to confirm the install, as it always does."
15. **Settings home: this version was withdrawn.** The top card slot, the same card with an amber
    `#FFB300` border and amber title: "AgentHQ 0.9.2 has a problem" / "A fix is coming. Save a copy of
    your notes now, to be safe." / green pill [Save a copy of my notes] (pin 18). Then "This phone": "Background work · On · answers chats, runs tasks", "Keep
    running · Good · 1 thing to check", "Notifications · Needs you on · Updates on · App updates on",
    "Phone assistant · Not chosen", "Capture and widgets · Sharing opens a new note", "Keys on this
    phone · Add, replace or remove"; "Your accounts": "AI service and keys · GitHub Copilot · 3
    services".
16. **App updates: the other states.**
    - "Withdrawn, a fix is out": "AgentHQ 0.9.2 has a problem" / "Update to 0.9.3. It fixes it." /
      [Install update] (pin 19).
    - "Found, then withdrawn": "Up to date" / "0.9.2 was withdrawn. Nothing was installed." (pin 20).
    - "Checked by hand during a rollout": [Install update] "AgentHQ 0.9.3 is ready · 41 MB" (pin 21) /
      "It is going out to a few phones at a time. You can install it now."
    - "Server HQ": "Your server HQ" / "Its own updates. This phone does not install them."

### The channel file, after (signed by the release, signed again at promote and at withdraw)

```json
{
  "version": "0.9.3",
  "versionCode": 93,
  "url": "https://github.com/<owner>/<repo>/releases/download/v0.9.3/agent-hq-android.apk",
  "sha256": "<64 hex>",
  "minSdk": 26,
  "size": 42991616,
  "notes": "Fixes a crash when opening a large vault.",
  "seq": 412003,
  "issued_at": "2026-11-05T09:12:00Z",
  "rollout": 20,
  "urgent": false,
  "withdrawn": [92],
  "switches": {
    "install_without_confirm": false,
    "lock_screen_assistant": false,
    "task_outbox": true,
    "start_cover_removal": true
  }
}
```

- New fields (from `size` down) are all inside the signed bytes. A file without them still parses:
  rollout 100, nothing withdrawn, every switch as built, install without confirm off.
- `seq` must rise. The phone keeps the highest it has seen and refuses an older file, so a replayed file
  cannot hide a withdrawal.
- **Promote** builds a new file from the release's verified one, sets rollout and seq, and signs it with
  `sign.sh`, instead of copying it.
- **Withdraw** is a new manual workflow: it adds a version code to `withdrawn`, can set rollout to 0, and
  signs.

### How this unblocks D11 (install without Android's confirm)

D11 stays off. It turns on for a phone only when all of these are true, and the app checks each one
before every install:

1. Safe mode has shipped in a stable release at least 30 days old, and opened on every device-check
   phone.
2. The idle gate is built and HQ is free right now.
3. The phone honours seq, withdrawn, rollout and switches, and the version is not withdrawn.
4. The signed file says `install_without_confirm: true`, and the phone follows the stable channel, never
   test versions.
5. A copy or backup of the notes is less than 7 days old. Otherwise Android's confirm shows, with the
   copy-first card.
6. Android 12+ only (`setRequireUserAction`), with `UPDATE_PACKAGES_WITHOUT_USER_ACTION` added to the
   manifest, and only once the app is the installer of record.

## 5. Exact copy

| Where | String |
|-------|--------|
| Sheet button | "Save a copy of my notes" |
| Shortcut | "Save notes" (long label: "Save a copy of my notes") |
| After 2 crashes | "HQ closed twice while opening. You can save your notes here first." · button "Open HQ again" |
| Title | "Save a copy of my notes" |
| Body | "This works even when HQ cannot start. Your notes go into one file, in a place you choose." |
| In the copy | "Your notes, with their pictures and files" · "Tasks, chats and what HQ remembers, in one database file" · "Notes and tasks waiting to send" |
| Not in it | "Your keys. Android locks them to this phone, so they cannot be copied." · "Settings, sign-ins and paired devices" |
| Size | "About {size} · {N} files" · while counting: "Counting your files · {N} so far" |
| Lock line | "The copy is not locked with a password. Anyone with the file can read your notes." |
| Buttons | "Choose where to save" · "Not now" |
| File name | "HQ notes {yyyy-MM-dd HHmm}.zip" |
| Saving title | "Saving your notes" · "{N} of {M} files done" · "{p}% · {a} of {b} MB" · "About {t} left" |
| Saving lines | "Keep this screen open until it is done." · "HQ paused for a moment to copy the database. It starts again when the copy is done." |
| No pause | "HQ did not pause, so the last minute of changes may be missing." |
| Stop | "Stop copying?" · "The part saved so far is removed." · "Keep copying" · "Stop copying" |
| Done | "Your notes are saved" · "File" · "Size" · "Saved: In {place} at {time}" · "Move it off this phone, to a computer or a USB stick. A copy on this phone is lost with the phone." · "Restore on this phone or a new one opens this file too." · "Done" |
| Left out | "{N} files could not be read and were left out. Their names are in left-out.txt in the copy." |

Failure lines (new keys in the rules section 2 catalogue):

| Reason | String |
|--------|--------|
| Phone full | "Couldn't save a copy of your notes. The phone is almost full. Free up {size} and try again." |
| Place full | "Couldn't save a copy of your notes. That place is full. Choose another place, or free up {size}." |
| Place gone | "Couldn't save a copy of your notes. The place you chose stopped answering. Choose another place and try again." |
| Under it | "The copy needs about {size}. {Place} has {free} free. A USB stick or memory card works too." · "The unfinished file was removed. Nothing on the phone changed." |
| Cancelled | "Nothing was saved." |
| Interrupted | "The last copy did not finish. The unfinished file was removed." |

Server HQ:

| Where | String |
|-------|--------|
| Title, body | "Your notes are on your server HQ" · "This phone keeps no notes of its own, so there is nothing here to copy. Backups run on your server HQ." |
| Waiting | "{N} notes waiting to send" · "They go to your server HQ when it can be reached. You can save them now." · "Save {N} waiting notes" |

Updates:

| Where | String |
|-------|--------|
| Busy dialog | "Install now?" · one of "HQ is answering a chat." / "HQ is making a backup." / "HQ is sending notes saved while it was stopped." / "The speech model is downloading." · "Installing now stops it." · "Wait until it's done" · "Stop the chat and install" (or: "Stop it and install") |
| Waiting row | "Installs when HQ is free" · "Waiting for 1 chat to finish" · "Not now" · "Android then asks you to confirm the install, as it always does." |
| Copy first | "Save a copy of your notes first" · "Backups are not set up. If this update goes wrong, a copy keeps your notes safe." · "Save a copy" |
| Withdrawn | "AgentHQ {version} has a problem" · "Update to {new}. It fixes it." · or: "A fix is coming. Save a copy of your notes now, to be safe." |
| Found, pulled | "Up to date" · "{version} was withdrawn. Nothing was installed." |
| Rollout | "It is going out to a few phones at a time. You can install it now." |
| Notice (App updates channel) | "AgentHQ {version} has a problem" · "Open HQ to see what to do." (daily while it applies, as the urgent flag) |

## 6. Behaviour rules

Safe mode:

1. **The sheet button** shows whenever HQ has stopped or not started, and in Backup and about as "Save a
   copy now".
2. **The shortcut** is static (`shortcuts.xml`), so it works when the main screen crashes. It opens
   `SafeModeActivity` in process `:safe`, built with plain Android views, no Compose screens from the
   app, no HQ route, no network.
3. **Two crashed starts** in a row (a marker written first thing in `ShellActivity.onCreate`, cleared
   after the first frame) open safe mode on the next launch, with the line and Open HQ again.
4. **What is copied:** everything under `files/hqhome/.vault`, except logs, HQ's lock files, `.hqbak`
   files and `.hq-restore-` folders; the database as `vault.db`, `-wal` and `-shm` together;
   `outbox.json` as `waiting-to-send.md`. Never `config.yaml`, `hqhome/.hq` (the bridge key),
   preferences, sealed keys or logs.
5. **Counting** walks the folder off the main thread and shows the count as it goes. The size shown is
   the files' size, the most the .zip can need.
6. **The picker** is `ACTION_CREATE_DOCUMENT`, type `application/zip`, with the file name and Downloads
   as the start.
7. **Writing** streams a .zip into the chosen file: text compressed fast, pictures and PDFs stored as
   they are, Zip64 past 4 GB. Progress every 200 ms. The screen stays on only while copying. If HQ
   runs, the hub holds it (`holdHq`, at most 25 s) for the database files only, then releases it.
8. **Done** names the file, size, place and time. It records the copy's time for FixBackup and the
   copy-first rule.
9. **Space** is checked before writing when the provider reports free space for its root: need = size +
   10 MB. Otherwise a full disk is caught while writing (rule 10).
10. **Any write error** deletes the document and shows the matching line. Nothing in the vault is
    touched.
11. **Cancel and interruption:** Back from the picker shows "Nothing was saved." The document's address
    is noted before writing, so after a killed process the next open deletes the partial file and says
    so.
12. **Stop copying** asks first. Back and a tap outside keep copying.
13. **Server HQ:** with no vault on the phone and a server signed in, the screen says so and offers only
    the waiting notes.
14. **At 360 dp and font scale 2.0** the lists wrap, the page scrolls, and Choose where to save stays at
    the bottom.

Updates:

15. **The idle gate:** before `commit()` the app checks that no chat turn or backup runs (a signed-in HQ
    route), and that no outbox send or other download runs (the app knows these). HQ stopped counts as
    free. Busy shows the dialog.
16. **Copy first** shows when no backup or copy is under 7 days old.
17. **Waiting** installs as soon as HQ is free, checked every 10 s. Android's confirm then shows if the
    app is open, else the App updates notice "AgentHQ {version} is downloaded." with Install.
18. **Installed version withdrawn, no fix yet:** the Settings card slot and a daily App updates notice
    point to safe mode.
19. **Installed version withdrawn, fix out:** the card offers the fix at once, whatever the rollout
    share.
20. **A found update that is then withdrawn** disappears, and its partial download is deleted.
21. **Rollout:** the phone keeps a random number from 0 to 99, made once. An automatic offer needs
    number below rollout. A manual check shows the update anyway. Test versions are always 100.

## 7. Android versions

| Version | Behaviour |
|---------|-----------|
| 8 and 9 | The picker, static shortcuts and a separate process all work (minSdk 26). Free space comes only from providers that report it. Zip64 writing to check on the 8.1 emulator with a 5 GB fixture. |
| 10 and 11 | Same. Scoped storage does not matter: the picker grants the one file. With `hasFragileUserData`, the uninstall dialog offers to keep the data. |
| 12 | As 11. `setRequireUserAction` exists, but D11 stays off until its checklist holds. |
| 13+ | Safe mode posts no notification, so it needs no permission. Android 14+ blocks starting the install confirm from the background, hence rule 17. |

Android constraints, as checked on the board:

- **No downgrade:** the installer refuses a lower version code over a newer one. Recovery is a newer,
  fixed version, hence withdrawn.
- **Uninstall removes `filesDir`,** unless the owner ticks "keep app data" (Android 10+, with
  `hasFragileUserData`).
- **A process of its own** gets its own runtime. The manifest has no Application class, so nothing of
  the app runs before the screen.
- **Cloud providers** (Drive and others) may stage the whole file on the phone first.
- **A screen turned off** lets Android stop the process later. Keep-screen-on while copying, rule 11 for
  the rest.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `kotlin/.../safe/SafeModeActivity.kt` | Views only, process `:safe`, ignores extras. | NEW (the `safe/` package is new) |
| `kotlin/.../safe/NotesCopy.kt` | Pure: plan, exclusions, sizes, entry names (`plan()` is unit-tested). | NEW |
| `safe/CopyWords` | In string resources, not a Kotlin object. | NEW |
| `AndroidManifest.xml` | The activity with `android:process=":safe"`, extras ignored; `android:hasFragileUserData="true"` if chosen. | yes |
| `res/xml/shortcuts.xml` | Static shortcut `save_notes` ("Save notes"). | yes |
| `kotlin/.../ShellActivity.kt` | Crash markers in `onCreate` (cleared after the first frame); the sheet button. | yes |
| `java/example/hq/spike/HubService.java` | A hold for the copy over `holdHq()` and `releaseHq()` (lines 142 to 162), at most 25 s. | yes |
| `kotlin/.../ui/firstrun/Restore.kt` | Accept a notes .zip as well as .hqbak, same swap (lines 124 to 136). | yes |
| `kotlin/.../update/UpdateLogic.kt` | `ChannelManifest` (line 8) gains the new fields with safe defaults; `verdict()` (line 92) takes bucket and withdrawn. | yes |
| `kotlin/.../update/Updater.kt` | The idle gate before `commit()` (line 200); seq and bucket in prefs; withdrawn handling. | yes |
| `crates/hq-web` | A signed-in busy route (turns, backup). | NEW route |
| `scripts/release/android-channel.sh` | `--seq --rollout --notes --size --withdrawn --urgent`. | yes |
| `.github/workflows/promote.yml` (88 to 100) | Build and sign, never copy. | yes |
| `.github/workflows/android-withdraw.yml` | The manual withdraw workflow. | NEW |

## 9. Server, release or desktop changes

- HQ: a **signed-in busy route** for the idle gate (is a chat turn or a backup running). FixPlan lists
  it as "A signed-in busy route for the idle gate before an install."
- Release: the channel file gains `seq`, `issued_at`, `rollout`, `urgent`, `withdrawn`, `switches`,
  `size`, `notes`; promote re-signs instead of copying; a withdraw workflow.
- FixPlan also names: `UpdateLogic.kt` hardcodes a real GitHub owner (`RELEASES_URL` at
  `UpdateLogic.kt:40`); move it behind config (see `wave-2/02-updates.md`).

## 10. Acceptance checks (each fails on today's build)

1. A test build whose hq exits at start: the Connection sheet shows "Save a copy of my notes". A
   2,860-note fixture vault saves, and the .zip opens on a computer with every note.
2. A test build whose `ShellActivity` throws in `onCreate`: the "Save notes" shortcut opens safe mode
   and the copy works. After two crashed launches, the next launch opens safe mode by itself.
3. Airplane mode with HQ stopped: the copy finishes. No network call is made (checked with a strict
   network policy in the test).
4. Emulator storage filled to 120 MB free: frame 7 shows before anything is written, and no file is
   left.
5. A USB stick pulled mid-copy: frame 8, and the partial file is gone. Kill the process mid-copy: the
   next open says the unfinished file was removed.
6. Back from the picker: "Nothing was saved." and no error.
7. Unit test on `NotesCopy.plan()` over a fixture `filesDir`: no `config.yaml`, no `bridge.key`, no
   preferences; `vault.db` with its `-wal` and `-shm`; `waiting-to-send.md` when the outbox has items.
8. A 1 GB vault on the Android Go phone: progress moves at least every 2 s. The time is recorded for the
   "About {t} left" estimate.
9. Fresh install, Restore, choose the .zip: the vault comes back and the notes open.
10. Install tapped during a chat turn: the dialog shows. Wait: the install starts within 10 s of the
    turn ending.
11. A test channel file withdrawing the installed code: the card shows after one check. Serve an older
    seq after it: refused, the card stays.
12. Rollout 0: no automatic offer on any of 20 phone profiles. A manual check shows the update.

## 11. Check on device

- A 4 GB zip on 8.1 (FixPlan "Still to prove" for R3; the board says: Zip64 writing on the 8.1 emulator
  with a 5 GB fixture).
- On the A32: a 340 MB copy straight to Drive with 200 MB free (cloud providers may stage the whole file
  first).
- Safe mode opened on every device-check phone (a D11 precondition).

## 12. Open questions and decisions it depends on

Open questions for the owner, with the board's recommendation:

- **Plain .zip, or locked with a password?** Recommendation: plain. It must open with no HQ and no key.
  The screen says so in one line. (Decided: N6, "A plain zip, no password, for now.")
- **Show the copy-first card before every update?** Recommendation: only when no backup or copy is under
  7 days old. (Decided: N7.)
- **Add `android:hasFragileUserData="true"`?** On Android 10+ the uninstall dialog then offers to keep
  the app's data. Recommendation: yes, it is one line and saves notes from a hasty uninstall. (Not
  listed among the decisions on FixPlan; still open.)
- **Manual check during a rollout.** Recommendation: show the update, as frame 16 does. Rollout slows
  automatic offers only.

Depends on: D11, N6, N7, N12 (a withdrawn version takes the top Settings card), N14 (rollout 10% for 2
days, 50% for 2 days, then all; a crash report sets 0), rules section 10 (kill switches). Feeds
`wave-1/02-backup.md` (Save a copy now, the copy time) and `wave-2/02-updates.md` (idle gate, rollout,
withdrawn).
