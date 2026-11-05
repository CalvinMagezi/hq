# Wave 1 · 02 · A truthful backup row (finding #9)

Board: **FixBackup** ("Fix 9: backup status"), revised after review, 5 Nov 2026.
Area: Settings (`ui/settings`: `BackupFacts`, `SettingsWords`, `SettingsScreens`, `SettingsViewModel`).
Follows rules sections 2, 3, 5, 6 and 7 (`../../rules-shared.md`); where this board differs, the rules
win. "Save a copy now" is the same code and screens as safe mode's "Save a copy of my notes"
(`../wave-0/02-safe-mode.md`); where the two differ, FixSafeMode wins.

## 1. Finding and the review items it closes

**#9 A false backup alarm.** The "Backup and about" row on Settings home always says "No backup yet" in
amber, even when the backup card one tap away says "Backup is on".

**Who it hurts, how often.** Every owner, every time Settings opens. An owner whose backups work is told
they do not, learns to ignore amber, and then misses the one day a backup really fails.

What changed in this revision (each closes a review item):

- **D1 revised:** amber only when the page offers something that clears it. Amber when backups are only
  on this phone, when the last one failed, and when one is overdue. The new action "Save a copy now"
  (Android's save screen) clears it.
- **Six states, not seven,** plus one fact: whether a copy was saved off this phone in the last 7 days.
- **Backups on but missing a folder or key file** no longer stay grey forever. HQ writes no status then
  (`backup.rs:47-49` and `record_failure`). The row turns amber after twice `every_hours`.
- **A backup folder outside the app** now reads "Could not check". Today `SettingsViewModel.kt:303`
  swaps it for `files/backups` without a word.
- **Server HQ:** owners on a server HQ with Background work paused see "Backups run on your server HQ" in
  grey (M8).
- **No "Checking…" rows:** rows from HQ keep saved values under "Saved on this phone at 14:32".
- **Settings home drawn as rules section 5,** with the card slot on top. "Set up backup" is not drawn
  until it works.
- **No green status:** "Backup is on" is ink with a grey check. Copy split to 15 words a sentence. "Copy
  details" replaces "Copy diagnostics".
- **Added:** Android versions card, a 360 dp frame at font scale 2.0, acceptance checks that fail on
  today's build.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/kotlin/example/hq/spike/`.

- `ui/settings/SettingsWords.kt:150` the home row is a constant: `HomeRow(SettingsPage.About, "arch",
  "Backup and about", NO_BACKUP, warn = true)`; `:160` `const val NO_BACKUP = "No backup yet"`; `:136`
  `home(...)` wraps everything in `withChecking(loaded, …)` (`:156`), which turns rows into "Checking…"
  while HQ has not answered.
- `ui/settings/SettingsViewModel.kt:300-307` `backupFacts()` reads the config's `backup:` block,
  `backup-status.json` and the newest `hq-backup-*.hqbak`. Line 302 `runCatching { … }.getOrNull()`
  treats an unreadable config as absent. Line 303 keeps `backup.dir` only if it is inside `filesDir`,
  else silently uses `files/backups`.
- `ui/settings/BackupFacts.kt:9` `BackupConfig(enabled, everyHours, keep, dir)` (no `key_file`); `:20`
  `Card(tone, title, body, offerSetUp)`; `:26` `OFF_BODY`; `:48-62` `card()` yields not set up, failed,
  "No backup has finished yet", old, on.
- `ui/settings/SettingsScreens.kt:1029` `BackupCard` draws the good title in `Mts.BrandGreen` and a
  disabled "Set up backup" pill; `:229` the home row status has `maxLines = 1` and `Ellipsis`.
- `crates/hq-cli/src/commands/start/daemon/tasks_periodic/backup.rs:47-49` `check()` bails with
  "backup.enabled needs backup.dir and backup.key_file"; `record_failure` (line 225) writes a status only
  into a folder that exists.
- `app/src/test/kotlin/example/hq/spike/ui/settings/BackgroundWorkTest.kt:96` asserts
  `"No backup yet"`.
- `ui/chat/NoteMenu.kt:201` already uses `ActivityResultContracts.CreateDocument`.

### Before frames

- **B1 · Settings home.** This phone: "Background work · On · answers chats, runs tasks", "Keep running ·
  Good · 1 thing to check", "Notifications · Needs you on · Updates on", "Capture and widgets · Sharing
  opens a new note", "Keys on this phone · Add, replace or remove". Your accounts: "AI service and keys ·
  GitHub Copilot · 3 services", "Messaging apps · Telegram on · Discord on", "Other devices · 1 signed
  in". How HQ behaves: "Safety and limits · Read only". Your data: "Spending limits · $2.10 of $5.00
  today", "Backup and about · No backup yet" (amber, warning sign; pin A).
- **B2 · One tap later: Backup and about.** Card "Backup is on" (green) / "Last backup 3 h ago, 412
  files. A new one is made every 24 hours and the newest 7 are kept." (pin B). About: "AgentHQ · hub ·
  build 1", "Run mode · Active", "HQ · 0.9.1". Updates: "Check for updates · Up to date". Support: "Copy
  diagnostics", "Restart HQ".

Why it happens: **A** the home row is a constant; **B** the page already knows (and draws its good title
in brand green); **C** backups on, but no folder or key file: no status, no file, so the card says "No
backup has finished yet" in green forever; **D** a folder outside the app is swapped silently. Two
smaller lies: an unreadable config is "not set up", and `withChecking()` turns this local row into
"Checking…" even though it needs nothing from HQ.

What the owner reads: Settings: "No backup yet", amber, a warning sign. Tap. "Backup is on", green, last
one 3 h ago. Back. Still amber. Either the app is wrong or the backup is. Most people stop trusting both.
And the backups that do work sit on the same phone the owner might lose.

## 3. Decision (D1 revised)

**The row and the card read the same facts through one function, and amber always comes with something
to tap.**

- **What:** one decision, `BackupWords.state(facts, now)`, picks one of six states: Could not check, Not
  set up, Failed, Waiting, Overdue, Recent. One more fact rides on it: whether a copy left this phone in
  the last 7 days. The card and the home row are both drawn from that, so they cannot disagree.
- **Amber means "you can fix this here":** failed, overdue, and backups only on this phone. Each amber
  card offers Save a copy now. Grey states have nothing to do, like every other home row.
- **Grey with a one-time explanation** for Not set up when nothing can be tapped (Save a copy switched
  off). "Set up backup" is not drawn until it works.
- **The row is a phone fact,** like the rows under This phone: it is read from files on the phone and
  never turns into "Checking…".
- **Server HQ:** with a server HQ and Background work paused, the phone makes no backups by design. The
  row says so in grey.

Rejected:

- **A green "Backed up" on home.** Rules section 3: a healthy row is never green.
- **Amber for Not set up with nothing to tap** (the old D1). It trains owners to ignore amber, and the
  review overturned it.
- **Treat a copy in Downloads as safe.** It is on the same phone the owner might lose.

### Save a copy now (shared with safe mode)

- **What it is:** a plain copy of the vault (`files/hqhome/.vault`, see `Restore.kt:126`) into one .zip,
  saved where the owner picks on Android's save screen (`ActivityResultContracts.CreateDocument`, already
  used in `NoteMenu.kt:201`). It needs no HQ, so the same code serves safe mode when HQ cannot start.
- **What is in it:** the notes, plus tasks, chats and what HQ remembers in one database file. Not keys
  (Android locks them to this phone) and not the config. The page says so in FixSafeMode's words: "The
  copy is not locked with a password. Anyone with the file can read your notes."
- **Off the phone or not:** the app records the time of a copy only when the place is not this phone's
  own storage (the document's provider is not external storage, Downloads or media). A copy saved on the
  phone says "This copy is on this phone too." and the row stays amber.
- **What clears amber:** a copy off the phone in the last 7 days (owner decision 1). The time is a saved
  value in `filesDir`, kept until replaced.
- **Busy:** "Saving a copy · 45% · 26 of 57 MB", one 4 dp blue bar, Cancel. It runs in the app, so
  leaving the page does not stop it.
- **Failed:** rules section 2 lines, amber with an icon. "Couldn't save a copy of your notes. The phone
  is almost full. Free up 120 MB and try again." A cancelled save screen changes nothing and says
  nothing.
- **The database while HQ runs:** tasks live in `vault.db`, which HQ writes in WAL mode. A raw file copy
  can be torn. As on FixSafeMode, HQ pauses for a moment while the database is copied, and the card says
  "HQ paused for a moment to copy the database. It starts again when the copy is done."

## 4. Screens and states (After)

Settings home follows rules section 5: one card slot on top, then This phone, Your accounts, How HQ
behaves, Your data. Home rows: icon, bold title, status line (ink-muted, or amber with the warning mark
when it needs you), chevron. Status lines wrap; they are never cut.

Backup and about cards (`.bk`): surface-100, 12 dp radius, 16 dp padding, a 1 dp line (grey) border;
title 16/24 bold ink with an ink-muted icon; body 14/20 ink; a smaller 13/18 ink-muted line ("said").
The amber variant has a `#FFB300` border and an amber title and icon. Grey cards use an outlined button;
amber cards use a green "Save a copy now" button.

- **A1 · Home, all fine (whole page).** No card in the slot. This phone: "Background work · On · answers
  chats, runs tasks", "Keep running · Good · 1 thing to check", "Notifications · Needs you on · Updates
  on · App updates on", "Phone assistant · Not chosen", "Capture and widgets · Sharing opens a new note",
  "Keys on this phone · Add, replace or remove". Your accounts: "AI service and keys · GitHub Copilot · 3
  services", "Messaging apps · Telegram on · Discord on", "Other devices · 1 signed in". How HQ behaves:
  "Spending limits · $2.10 of $5.00 today", "Safety and limits · Read only". Your data: "Backup and about ·
  Last backup 3 h ago" (grey, no icon; pin 1).
- **A2 · Home, backups only on this phone.** Card slot (amber, warning icon; pin 2): "Check your backups"
  / "Backups are only on this phone. If you lose it, you lose them." Rows as A1; Your data "Backup and
  about · Last backup 3 h ago · only on this phone" (amber).
- **A3 · Home, overdue, HQ can't be reached.** Card slot "Check your backups" / "The last backup is 3
  days old. A new one should be made every 24 hours." "Background work · Only when I open the app". Above
  Your accounts, the strip "Saved on this phone at 14:32" (pin 3); the HQ rows keep their saved values.
  Your data "Backup and about · Overdue · last backup 3 d ago" (amber).
- **A4 · Home, server HQ, Background work paused.** "Background work · Only when I open the app", "Keep
  running · Good". Your accounts adds "Server HQ · Connected · hq.example.ts.net". Your data "Backup and
  about · Backups run on your server HQ" (grey; pin 4).
- **A5 · Page, on, recent, copy saved off the phone.** Grey card: "Backup is on" / "Last backup 3 h ago,
  412 files. A new one is made every 24 hours. The newest 7 are kept." / small "Copy saved off this phone
  2 d ago." / outlined [Save a copy now] (pin 5). About: "AgentHQ · Build 1", "Background work · On", "HQ
  version · 0.9.1". App updates: "Check for updates · Up to date". Support: "Copy details", "Restart HQ".
- **A6 · Page, on, only on this phone.** Amber card: "Backups are only on this phone" / "If you lose it,
  you lose them. Last backup 3 h ago, 412 files." / "Save a copy to a computer or a cloud drive." / green
  [Save a copy now] / small "The copy is not locked with a password. Anyone with the file can read your
  notes. Your keys are not in it." (pin 6).
- **A7 · Save a copy now, Android's save screen** (generic, pin 7). "Save to" [Save]; "File name · HQ
  notes 2026-11-05 1432.zip"; Places: "Downloads · This phone", "Cloud drive · you@example.com", "USB
  drive · Plugged in". Caption: "Android's own screen. HQ only names the file. The list shows what this
  phone offers."
- **A8 · Saving, then done, then on the phone too** ("Three moments of one card, drawn together", pin
  8). Grey "Saving a copy" / "45% · 26 of 57 MB" / 4 dp blue bar at 45% (read as "45 percent") /
  outlined [Cancel]. Grey "Copy saved" / "Your notes are off this phone now. Saved at 14:32." Amber "This
  copy is on this phone too" / "Move it to a computer or a cloud drive. Or save it again somewhere else."
  / green [Save a copy now].
- **A9 · Page, the last one failed.** Amber card: "The last backup did not finish" / "Your newest changes
  are not in a backup. The newest good one is from 26 h ago." / small "HQ said: not enough space left on
  the phone." / [Save a copy now] / [Copy details] (pin 9). Home row: "Backup and about · Last backup did
  not finish".
- **A10 · Page, on, but none made (no folder or key).** Amber card: "No backup has been made" / "Backups
  are on, but none was made in 2 days. The backup settings have no folder or no key file." / "Copy details
  for whoever set up HQ." / [Save a copy now] / [Copy details] (pin 10). Home row "Overdue · none made
  yet".
- **A11 · Page, overdue, while paused.** Amber card: "The last backup is old" / "Last backup 3 d ago. A
  new one should be made every 24 hours." / "Backups run only while Background work is on. It is
  paused." / link [Open Background work] / [Save a copy now] (pin 11). About shows "Background work · Only
  when I open the app". Note: "With Background work on, the second sentence is "Something is stopping it.
  Restart HQ, or copy details for whoever set up HQ." and there is no link."
- **A12 · Page, not set up, only on this phone.** Amber card: "Backup is not set up" / "Your notes are
  only on this phone. If you lose it, you lose them." / "The app cannot set up backups yet. Save a copy
  somewhere else to keep your notes safe." / [Save a copy now] (pin 12). Home row "Not set up · only on
  this phone".
- **A13 · Page, not set up, nothing to tap.** Grey card: "Backup is not set up" / "HQ can make backups by
  itself. The app cannot turn that on yet." / "When it can, this page will offer it." / outlined [Got it]
  (pin 13). Home row "Not set up" (grey). Note: "Only when Save a copy is switched off by the signed
  channel file. The card shows once; after Got it the page keeps the title and the first sentence."
- **A14 · Page, could not check.** Grey card: "Backup could not be checked" / "Backups go to a folder this
  app cannot open. It cannot say if they work." / [Copy details] (pin 14). Home row "Could not check".
  Unreadable settings instead: "The app could not read the backup settings. It cannot say if backups are
  on. Try again in a minute. If this stays, restart HQ."
- **A15 · Page, server HQ.** Grey card: "Backups run on your server HQ" / "Your notes live on your server
  HQ, hq.example.ts.net. This phone cannot see its backups yet." (pin 15). About: "Background work · Only
  when I open the app", "Server HQ version · 0.9.1". App updates: "Check for updates · Up to date". Note:
  "Never amber, never "Overdue": a paused phone makes no backups, and that is expected here."
- **A16 · 360 dp, font scale 2.0, only on this phone.** Amber card "Backups are only on this phone" / "If
  you lose it, you lose them. Last backup 3 h ago, 412 files." / [Save a copy now] full width (pin 16).
  Home row "Backup and about · Last backup 3 h ago · only on this phone" wraps to a second line.

## 5. Exact copy

| State | Home row | Card title | Card body |
|-------|----------|------------|-----------|
| Recent, copy off | "Last backup 3 h ago" (`../../resolutions.md` B10) | "Backup is on" | "Last backup 3 h ago, 412 files. A new one is made every 24 hours. The newest 7 are kept." Then, smaller: "Copy saved off this phone 2 d ago." |
| Recent, only here | "Last backup 3 h ago · only on this phone" | "Backups are only on this phone" | "If you lose it, you lose them. Last backup 3 h ago, 412 files. Save a copy to a computer or a cloud drive." |
| Waiting | "On · first one not made yet" | "Backup is on" | "No backup has finished yet. A new one is made every 24 hours." |
| Failed | "Last backup did not finish" | "The last backup did not finish" | "Your newest changes are not in a backup. The newest good one is from 26 h ago." (None: "No backup has finished yet.") Then, smaller: "HQ said: …", 160 characters at most. |
| Overdue | "Overdue · last backup 3 d ago" | "The last backup is old" | "Last backup 3 d ago. A new one should be made every 24 hours." Paused: "Backups run only while Background work is on. It is paused." plus "Open Background work". On: "Something is stopping it. Restart HQ, or copy details for whoever set up HQ." |
| Overdue, none | "Overdue · none made yet" | "No backup has been made" | "Backups are on, but none was made in 2 days." With no `dir` or `key_file` in the block: "The backup settings have no folder or no key file." Then "Copy details for whoever set up HQ." |
| Not set up | "Not set up · only on this phone" | "Backup is not set up" | "Your notes are only on this phone. If you lose it, you lose them. The app cannot set up backups yet. Save a copy somewhere else to keep your notes safe." |
| Not set up, copy off | "Not set up · copy saved 3 d ago" | "Backup is not set up" | "The app cannot set up backups yet. Your newest copy off this phone is from 3 d ago." |
| Not set up, no action | "Not set up" | "Backup is not set up" | "HQ can make backups by itself. The app cannot turn that on yet. When it can, this page will offer it." (Once, with "Got it".) |
| Could not check | "Could not check" | "Backup could not be checked" | Folder outside the app: "Backups go to a folder this app cannot open. It cannot say if they work." Unreadable: "The app could not read the backup settings. It cannot say if backups are on. Try again in a minute. If this stays, restart HQ." |
| Server HQ | "Backups run on your server HQ" | "Backups run on your server HQ" | "Your notes live on your server HQ, {name}. This phone cannot see its backups yet." |

| Element | String |
|---------|--------|
| Card slot | "Check your backups" (amber, warning icon), then the card body's first two sentences. Shown only when no Update ready or Notifications off card claims the slot. |
| Button | "Save a copy now" |
| File | "HQ notes {yyyy-MM-dd HHmm}.zip", FixSafeMode's pattern (`../../resolutions.md` D-4) |
| Busy | "Saving a copy · {pct}% · {done} of {total} MB" · "Cancel" |
| Done | "Copy saved" / "Your notes are off this phone now. Saved at {time}." |
| On the phone | "This copy is on this phone too" / "Move it to a computer or a cloud drive. Or save it again somewhere else." |
| Failed | rules section 2, first sentence "Couldn't save a copy of your notes." (for example "Couldn't save a copy of your notes. The phone is almost full. Free up 120 MB and try again.") |
| Lock line (amber only-here card) | "The copy is not locked with a password. Anyone with the file can read your notes. Your keys are not in it." |
| Database pause | "HQ paused for a moment to copy the database. It starts again when the copy is done." |
| Strip | "Saved on this phone at {time}" |
| Support | "Copy details" (replaces "Copy diagnostics"), "Restart HQ" |
| Server page | "Server HQ version" |

Ages use `ago()`: "just now", "5 h ago", "3 d ago". Times use the phone's format and its 12 or 24-hour
setting. TalkBack reads the row as title, status, then "Check this" when amber.

## 6. Behaviour rules (numbers match the After pins)

1. **Recent:** `enabled: true`, status missing or ok, newest `.hqbak` younger than 2 × `every_hours`,
   and a copy off the phone in the last 7 days. Grey, no icon.
2. **Card slot:** one card on top of Settings home, in rules order: Update ready, else Notifications off,
   else "Check your backups" when the backup row is amber.
3. **HQ can't be reached:** rows that come from HQ keep their saved values under one "Saved on this phone
   at {time}" strip. The backup row is local and keeps its live state. No row says "Checking…".
4. **Server HQ:** `ServerHq.address(ctx)` is set and Background work is paused. The row reads "Backups
   run on your server HQ" in grey, whatever the local files say. With Background work on, the phone
   states apply.
5. **Order of checks,** first match wins: server HQ, could not check, not set up, failed, waiting,
   overdue, recent. Then the off-phone fact can turn Recent or Not set up amber.
6. **Only on this phone:** no copy off the phone in the last 7 days. Amber, with Save a copy now. Waiting
   and Could not check never take it.
7. **Save screen:** `CreateDocument("application/zip")` with the suggested name. Cancel changes nothing.
8. **Copy result:** the saved time is recorded only for a place that is not this phone. The row changes
   on the next refresh.
9. **Failed:** `backup-status.json` says `ok: false`. Amber even when an older good backup exists. The
   card names that one's age.
10. **On, but none made:** no backup file and no status for 2 × `every_hours` after the phone first saw
    `enabled: true` (a saved value). HQ writes no status when `dir` or `key_file` is missing
    (`backup.rs:47-49`, `record_failure` needs the folder). The card names that cause when the block
    lacks either key.
11. **Overdue:** newest file at or past 2 × `every_hours` (48 h for 24). In Standby the daemon's backup
    task does not run, so the card says so and links to Background work.
12. **Not set up:** no `backup:` block, or `enabled` is not true, or no config file. Amber while no copy
    left the phone in 7 days.
13. **Nothing to tap:** when the signed channel file sets `switches.save_copy` to false
    (`../../resolutions.md` D-3), Not set up is grey. The
    explanation shows once, then stays dismissed (a saved value).
14. **Could not check:** the config exists but cannot be read (any error other than file not found), or
    `backup.dir` is outside `filesDir`. Grey, never "Not set up", never "first one not made yet".
15. **Server variant page:** no local states, no Save a copy (the notes are not on the phone). Its version
    row comes from the server HQ.
16. **Large text:** the status wraps, never "…". Buttons go full width. The state is never cut off.

**Refresh:** recomputed on every Settings refresh and on return from the page, so the row changes as soon
as a backup lands or a copy is saved.

## 7. Android versions

| Version | What differs for this fix |
|---------|---------------------------|
| 8 and 9 (API 26 to 28) | Android's save screen works the same (it exists since API 19). Cloud places appear only when their app offers one. |
| 10 and 11 | Scoped storage changes nothing here: the save screen hands the app one document to write. Backups in `filesDir` need no storage permission. |
| 12 | `allowBackup="false"` no longer stops phone-to-phone transfer. The backups folder and the saved copy time follow the data-extraction rules on FixDataTransfer (wave 0). |
| 13+ | "Backup finished." posts on the Updates channel only with notification permission. Without it, nothing posts and the row still tells the truth. The row never depends on a notification. |

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `ui/settings/BackupFacts.kt` | `BackupConfig` (line 9) gains `keyFile: String?` (`parseConfig` reads `key_file`, a path, still nothing else). `BackupFacts` (line 15) gains `readable`, `dirOutside`, `firstSeenOnMs`, `copyOffPhoneMs`, `server`. Add `enum class BackupState { CouldNotCheck, Off, Failed, Waiting, Overdue, Recent }`, `state(f, now)`, `onlyHere(f, now)` and `homeLine(f, now)`; rebuild `card()` (line 48) on them. `OFF_BODY` (line 26) goes. `offerSetUp` becomes `offerCopy`. | yes |
| `ui/settings/SettingsViewModel.kt` | `backupFacts()` (lines 300-306): a `FileNotFoundException` means absent, any other read error sets `readable = false` (today `runCatching { … }.getOrNull()`, line 302). A `dir` outside `filesDir` sets `dirOutside` instead of the silent swap (line 303). | yes |
| `ui/settings/SettingsWords.kt` | `home()`: take the backup row as a parameter (built from `homeLine`) and delete `NO_BACKUP` (line 160). `withChecking()` (line 156) is replaced by saved values and the strip. | yes |
| `ui/settings/SettingsScreens.kt` | `BackupCard` (line 1029) drops `Mts.BrandGreen` for the good title and the disabled "Set up backup" pill, adds Save a copy now. Home row status (line 229) loses `maxLines = 1` and `Ellipsis`. | yes |
| `safe/NotesCopy.kt` | The one copy implementation, shared with safe mode (`../../resolutions.md` D-4): the plan, the zip writer, the provider check, progress, and the saved time. Loads without Compose, so the `:safe` process uses it too. The file name is FixSafeMode's "HQ notes {yyyy-MM-dd HHmm}.zip". Built in `../wave-0/02-safe-mode.md`; this board calls it. | NEW (wave 0) |
| `app/src/test/.../ui/settings/BackgroundWorkTest.kt` | Line 96 asserts "No backup yet". Change it. | yes |
| `app/src/test/.../ui/settings/BackupFactsTest.kt` | One case per state, the 2 × boundary, the missing-key case, the outside folder, the off-phone fact, and "row warns exactly when the card warns" over every fixture. | yes |

## 9. Server, release or desktop changes

- None required. Open question 4 proposes a read-only `/api/backup/status` behind the web token later
  (no HQ route reports backup status today; none in `crates/hq-web/src`).
- The signed channel file's `switches.save_copy` (a boolean, `../../resolutions.md` D-3, added to the
  file in `../wave-0/02-safe-mode.md`) turns Save a copy off (rule 13).

## 10. Acceptance checks (each fails on today's build)

1. Fixture: on, newest 3 h ago, copy off the phone 2 d ago. Home reads "Last backup 3 h ago" in grey
   with no icon. Today it reads "No backup yet" in amber.
2. For every fixture, the home row is amber if and only if the card is amber. Today the row is amber for
   all of them.
3. `enabled: true`, no `dir`, no status, first seen 49 h ago, every 24 h: row amber "Overdue · none made
   yet", card names the missing folder or key. Today the card says "No backup has finished yet" forever.
4. `dir: /sdcard/hq-backups` with a fresh file in `files/backups`: "Could not check". Today it reports the
   file in `files/backups`.
5. A config that throws an `IOException` other than not found reads "Could not check", never "Not set
   up". Today it reads "Not set up".
6. With HQ stopped (`loaded = false`), no Settings row reads "Checking…" and the backup row keeps its
   state. Today every non-phone row reads "Checking…".
7. Save a copy now to a cloud provider turns the row grey on the next refresh. Saved to Downloads, it
   stays amber and the card says "This copy is on this phone too". Today there is no action.
8. Server HQ signed in and Background work paused: row grey "Backups run on your server HQ", never
   Overdue.
9. Every 24 h, a backup 47 h 59 min old is Recent and one 48 h old is Overdue, on the row as well as the
   card.
10. At 360 dp and font scale 2.0, "Last backup 3 h ago · only on this phone" wraps to a second line and is never cut.
    Today line 229 cuts it with "…". Amber on Surface 100 is 9.1:1, grey 10.7:1.
11. The good card title is not `Mts.BrandGreen` (a screenshot test with a colour probe). Today it is.

## 11. Check on device

- Provider names for "off the phone": One UI My Files, Files by Google, and an SD card (FixPlan: "Where a
  copy was saved").
- Android 8 and 9: an Android Go phone with Files by Google, and the 8.1 emulator (the save screen and
  its cloud places).

## 12. Open questions and decisions it depends on

Open questions for the owner, with the board's recommendation:

1. **How long does a copy off the phone keep the row grey?** Recommendation: 7 days, the same span the
   default keep of 7 daily backups covers. Longer lets the copy go stale. Shorter nags. (Decided: N16, "7
   days; a copy in Downloads does not count.")
2. **Should Save a copy now lock the .zip with the backup key?** Recommendation: no, for now. Safe mode
   must work with no HQ and no key, and an owner without a laptop cannot open an .hqbak. The sheet says
   plainly that anyone who opens it can read it. (Matches N6.)
3. **Does a copy in Downloads clear amber?** FixSafeMode starts the picker in Downloads, which is right
   there: Downloads survives an uninstall. Here the risk is a lost phone. Recommendation: no. Downloads,
   internal storage and media count as "on this phone too". A memory card counts the same until it is
   checked on device. (Decided: N16.)
4. **Show a server HQ's backups on the phone?** No HQ route reports backup status today. Recommendation:
   later, with a read-only `/api/backup/status` behind the web token. Until then the grey line is honest.

Depends on: D1, N6, N16; rules sections 2, 3, 5, 6, 7; `../wave-0/02-safe-mode.md` (the copy code and
screens, the copy time); `../wave-0/03-data-transfer.md` (what moves).
