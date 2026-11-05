# Wave 0 · 03 · Moving phones (M1, D17)

Board: **FixDataTransfer** ("Fix M1: moving phones"), wave 0, added after the review, 5 Nov 2026.
Area: `AndroidManifest.xml`, a new `data_extraction_rules.xml`, first run on a new phone.
Shared rules: `../../rules-shared.md` (sections 5 and 6 win).

## 1. Finding and the review items it closes

**The problem.** The manifest says `android:allowBackup="false"` and has no `dataExtractionRules`. The
app targets Android 15 (targetSdk 35). For such an app on Android 12 and newer, allowBackup stops cloud
backup but not a phone-to-phone copy. So the copy Android offers when a new phone is set up can take the
whole app folder: notes and database in plain text, the phone's bridge key, the "active" run mode, and
keys that cannot be opened on the new phone.

**Who it hurts, how often.** Every owner who moves to a new phone from Android 12 or newer, once per
phone. They either get a half-working HQ (keys listed but dead, two phones answering the same chats) or,
if the copy is simply blocked, no way to move their notes without a computer.

Closes:

- **M1:** data-extraction rules for cloud backup and for phone-to-phone copy, tested with a real copy.
- **D17 revised:** the saved copy (FixOffline) is excluded from every copy, so "never in backups" is
  true.
- **Spoken ids and the outbox** stay on the old phone.
- **A way to move phones without a laptop:** the vault moves, with "Your keys did not come with you. Add
  them again."

## 2. Before (as built, verified at `c04a100`)

- `apps/android/app/src/main/AndroidManifest.xml:16-17`:
  `<application android:label="AgentHQ" android:icon="@mipmap/ic_launcher" android:allowBackup="false"
  android:networkSecurityConfig="@xml/network_security_config">`. No `android:dataExtractionRules`, no
  `android:fullBackupContent`.
- `apps/android/app/build.gradle.kts:83-84` `minSdk = 26`, `targetSdk = 35`.
- `res/xml/` holds no rules file.
- `noBackupFilesDir` is used only by the speech model (`voice/VoiceModels.kt:29`).
- `java/example/hq/spike/SecretStore.java:104` `names()` lists names from the `secrets` preferences
  whether or not the value opens.
- `ui/firstrun/FirstRun.kt:6` `enum class Step { Welcome, Model, Chat, Start, Restore }`; no Moved step.

### Before frames

- **B1 · The manifest** (as above), "targetSdk = 35 minSdk = 26", "No android:dataExtractionRules. No
  android:fullBackupContent. No noBackupFilesDir use." Pin A.
- **B2 · New phone setup: copy apps and data** (Android's setup, generic): "Copy apps and data / From
  your old phone, with a cable or over Wi-Fi." "Apps · 38 apps and their data", "AgentHQ · 1.2 GB" (pin
  B), "Messages, photos, contacts · Copied as well".
- **B3 · The new phone, first open.** "Keys on this phone / Names only. Values never leave the
  Keystore." listing `OPENROUTER_API_KEY`, `TELEGRAM_BOT_TOKEN`, `DISCORD_BOT_TOKEN` (pin C).
  "Meanwhile: Background work · On · answers chats, runs tasks" (pin D). Caption: "First run was
  skipped: its "done" flag came along. HQ started in Active mode, the same as the old phone, which is
  still running."

What goes wrong:

- **A** allowBackup no longer stops the copy. For an app that targets Android 12 or newer,
  `allowBackup="false"` turns off cloud backup only. A phone-to-phone copy follows
  `dataExtractionRules`, and there are none, so everything outside `noBackupFilesDir` and the caches
  may go.
- **B** Plain notes, the bridge key and the run mode travel: `files/hqhome/.vault` (notes and
  `vault.db`), `files/hqhome/.hq/bridge/bridge.key` (the phone's identity for paired devices),
  `files/mode` ("active"), `files/outbox.json`, the logs and every preference file.
- **C** Keys look present but are dead. The sealed values come along in the `secrets` preferences, but
  the Keystore key that opens them stays on the old phone. `SecretStore.names()` lists names from the
  preferences, so the page shows three keys that cannot be opened.
- **D** Two phones answer the same chats. The `firstrun` preferences say done and `mode` says active, so
  the new phone skips first run and its one-machine warning, and starts the relays while the old phone
  still runs them.
- **Unverified:** whether a sideloaded AgentHQ is moved at all depends on the tool. Google's copy
  reinstalls apps from Play; Samsung Smart Switch also moves other apps. Both are device checks.

### Files in the app folder today

| Path under the app | Kind | What it holds |
|--------------------|------|---------------|
| `files/hqhome/.vault/` | Notes | Every note, picture and file, and `vault.db` with tasks, chats, memory, sign-ins and paired devices |
| `files/hqhome/config.yaml` | Settings | AI service settings. May hold keys an owner typed in by hand |
| `files/hqhome/.hq/` | Identity | `bridge.key`, the phone's private key for paired devices |
| `files/mode`, `outbox.json` | State | Run mode, and notes waiting to send |
| `shared_prefs/secrets`, internal | Sealed | Keys and tokens, sealed by a Keystore key that never leaves the phone |
| `shared_prefs/firstrun`, `server_hq`, `alerts`, `update`, `bridge`, … | State | First run done, the server's address, seen ids, update channel |

## 3. Decision

**Cloud backup takes nothing. A phone-to-phone copy takes the vault folder and nothing else.** The new
phone sees a vault with no first-run record, says what came and what did not, asks for the keys again,
starts in Standby, and shows the one-machine warning before Start HQ. An owner with no computer can now
move phones in the setup flow they already use.

Why the vault may move: the copy runs only during a new phone's setup, needs the old phone unlocked and
both phones in the owner's hands. The notes are no less safe on the new phone than on the old one. What
could do harm on another phone (keys, tokens, the bridge identity, the run mode) stays behind.

Rejected:

- **Exclude everything.** Safe, but an owner without a laptop loses every note when the old phone goes.
  The .zip from safe mode is the only way, and few find it.
- **Copy everything but the sealed keys.** Brings the bridge key (two phones with one identity),
  "active" mode and a stale first-run record.
- **Allow cloud backup of the vault.** 25 MB per app, then it stops silently, and plain notes would sit
  in the owner's cloud account.

### `res/xml/data_extraction_rules.xml` (new)

```xml
<data-extraction-rules>
  <cloud-backup>
    <exclude domain="root" path="."/>
    <exclude domain="file" path="."/>
    <exclude domain="database" path="."/>
    <exclude domain="sharedpref" path="."/>
    <exclude domain="external" path="."/>
  </cloud-backup>
  <device-transfer>
    <include domain="file" path="hqhome/.vault"/>
  </device-transfer>
</data-extraction-rules>
```

- Manifest: `android:dataExtractionRules="@xml/data_extraction_rules"` beside `allowBackup="false"`,
  which still covers Android 11 and older.
- An include list means only what is listed moves. Everything else in the table stays, with no exclude
  to forget when a new file is added.
- `noBackupFilesDir` gets the install marker, the copy's partial-file note and the counters
  (`04-measure.md`). Android never copies it.

### What moves, file by file

| Path | Moves | Why |
|------|-------|-----|
| `hqhome/.vault/` | Yes | The owner's notes, tasks, chats, memory |
| `hqhome/config.yaml` | No | May hold a typed-in key. Written again by the key step |
| `hqhome/.hq/` | No | `bridge.key`: two phones must never share one identity |
| `mode` | No | The new phone starts in Standby |
| `outbox.json` | No | The old phone sends them. Moving them would send twice |
| saved copy (FixOffline) | No | D17: may hold what came from a server HQ |
| spoken ids (voice) | No | D7 moved the spoken mark into HQ |
| `shared_prefs/*` | No | Sealed keys cannot open there. Server address, first run, seen ids start fresh |
| `hq.log`, caddy, web, Caddyfile | No | Logs and files the app rebuilds |
| `noBackupFilesDir` | Never | Android leaves it out of every copy |

## 4. Screens and states (After)

**Only the vault moves. The new phone knows it was moved and walks the owner through what did not come.**

1. **What the copy now takes** (Android's setup, generic): "Copy apps and data / From your old phone,
   with a cable or over Wi-Fi." Row "AgentHQ · 340 MB · notes only" (pin 1). "Moves:
   files/hqhome/.vault: notes, pictures, files and the database". "Stays on the old phone: Keys and
   tokens, settings, the bridge key, the run mode, notes waiting to send, the saved copy, logs, every
   preference".
2. **First open on the new phone** (pin 2). Title "Your notes came with you"; "2,860 notes and 140
   tasks moved from your old phone." Keys card: "Your keys did not come with you. Add them again." /
   "Android locks keys to the phone that made them." List (pin 3) "Also starting fresh": "Sign-ins from
   other devices and paired devices", "Notes waiting to send. They stayed on the old phone.", "Settings
   on this phone". Buttons "Add keys" (green, primary), "Start fresh instead".
3. **Start fresh instead.** Over frame 2, dialog "Start fresh?" / "The notes that came with you are
   deleted from this phone. Your old phone keeps its own." / [Keep my notes] (left) [Delete moved notes]
   (right, red `#FF6B6B`, black text; pin 4).
4. **AI key, on a moved phone.** The first-run "Your AI model" step: "HQ needs a key from an AI service
   to think. It is kept sealed on this phone and is never written to a file." plus the moved note (pin
   5) "Your old phone used OpenRouter. Paste that key again, or a new one." Fields Service, Key; buttons
   Next, Back, Skip.
5. **Ready to start, moved.** Top to bottom (rules section 5): card "Notifications are off / Questions
   will wait in the Inbox." [Turn on]; option "Use HQ as your phone's assistant / Optional. Hold Home to
   ask HQ."; amber warning (pin 6) "Is HQ still running on your old phone? / Stop it there first. Two
   copies would answer the same chats twice."; "Start HQ" (green); "Back".
6. **Other devices, after the move** (pin 7). "No other devices yet / Sign-ins did not move with your
   notes. Sign in again from each computer or browser." Group "Paired devices": "None paired / Pair each
   device again with a new code." Group "Server HQ": "Not signed in / Sign in again with a new code from
   your server HQ."
7. **The database did not copy cleanly** (pin 8). Amber line "Your notes came with you, but not your
   tasks and chats." / "The database file was being written while it was copied. Your note files are
   fine." / "Restore a backup or a notes copy to bring tasks and chats back. Or start HQ with notes
   only." Buttons "Restore a backup or a copy", "Start with notes only".
8. **Moving from Android 11 or older.** Welcome: "Welcome to HQ / HQ is your own AI assistant that
   lives on this phone and keeps its notes, tasks and memory here." Buttons "Set up a new HQ", "Restore
   from a backup or a copy" (pin 9). Line "Moving from another phone? On the old one, open Settings ›
   Backup and about › Save a copy of my notes."
9. **Restore takes a notes copy.** "Restore a backup or a copy / Pick the file. A notes copy ends in
   .zip and needs nothing else. A backup ends in .hqbak and needs its passphrase or key file." Chosen
   file row "HQ notes 2026-11-05 1432.zip · 338 MB · a notes copy" (pin 10). "Your notes, tasks and
   chats come back as they were. Keys are added again after this." Buttons "Restore", "Choose another
   file".
10. **Server HQ owner, new phone** (pin 11). "Sign in to your server HQ again / Your notes are on your
    server HQ, so nothing needed to move. This phone's sign-in stayed on the old phone." "On your
    server HQ, open Settings › Devices and make a new code. Then scan it here." Buttons "Scan a code",
    "Not now".
11. **Old phone: before you move.** Backup and about, card "Moving to a new phone / Android 12 and newer
    can copy your notes when you set up the new phone. Your keys stay here and must be added again. /
    From an older phone, or to be safe, save a copy and restore it on the new phone." [Save a copy of my
    notes] (pin 12). Note "After the move, stop HQ here before you start it there." Group "App updates":
    "Up to date · AgentHQ 0.9.3".
12. **360 dp, font scale 2.0** (pin 13). "Your notes came with you", "2,860 notes and 140 tasks moved.",
    the keys card "Your keys did not come with you. Add them again." wraps and stays above "Add keys".

## 5. Exact copy

| Where | String |
|-------|--------|
| Title | "Your notes came with you" |
| Count | "{N} notes and {M} tasks moved from your old phone." (plural resources for both) |
| Keys card | "Your keys did not come with you. Add them again." · "Android locks keys to the phone that made them." |
| Fresh list | "Also starting fresh" · "Sign-ins from other devices and paired devices" · "Notes waiting to send. They stayed on the old phone." · "Settings on this phone" |
| Buttons | "Add keys" · "Start fresh instead" |
| Dialog | "Start fresh?" · "The notes that came with you are deleted from this phone. Your old phone keeps its own." · "Keep my notes" · "Delete moved notes" |
| AI key note | "Your old phone used {service}. Paste that key again, or a new one." |
| Warning | "Is HQ still running on your old phone?" · "Stop it there first. Two copies would answer the same chats twice." |
| Other devices | "No other devices yet" · "Sign-ins did not move with your notes. Sign in again from each computer or browser." · "None paired" · "Pair each device again with a new code." |
| Server row (frame 6) | "Not signed in" · "Sign in again with a new code from your server HQ." |
| Not clean | "Your notes came with you, but not your tasks and chats." · "The database file was being written while it was copied. Your note files are fine." · "Restore a backup or a notes copy to bring tasks and chats back. Or start HQ with notes only." · "Restore a backup or a copy" · "Start with notes only" |
| Welcome | "Restore from a backup or a copy" (was: "Restore from a backup") · "Moving from another phone? On the old one, open Settings › Backup and about › Save a copy of my notes." |
| Restore | "Restore a backup or a copy" · "Pick the file. A notes copy ends in .zip and needs nothing else. A backup ends in .hqbak and needs its passphrase or key file." · "Keys are added again after this." |
| Restore file row | "{name} · {size} · a notes copy" |
| Server HQ, new phone | "Sign in to your server HQ again" · "Your notes are on your server HQ, so nothing needed to move. This phone's sign-in stayed on the old phone." · "On your server HQ, open Settings › Devices and make a new code. Then scan it here." · "Scan a code" · "Not now" |
| Old phone card | "Moving to a new phone" · "Android 12 and newer can copy your notes when you set up the new phone. Your keys stay here and must be added again." · "From an older phone, or to be safe, save a copy and restore it on the new phone." · "Save a copy of my notes" |
| Old phone note | "After the move, stop HQ here before you start it there." |

## 6. Behaviour rules

1. **Only the vault folder** is in the device-transfer list. Cloud backup lists nothing.
2. **Moved is detected** when `hqhome/.vault` exists and there is no first-run record and no install
   marker in `noBackupFilesDir`. An upgrader has the record, so is never taken for moved.
3. **Before HQ starts** on a moved phone, the app runs a one-off `hq vault moved` (new): it signs out
   every other device, forgets paired devices, edge and connector grants, and push subscriptions, all
   held in `vault.db`.
4. **Start fresh** deletes the moved vault after the dialog, then runs normal first run.
5. **The AI key step** names the service from the moved database's backend name only. Never a key,
   never a base URL with a token in it.
6. **Ready to start** follows rules section 5. On a moved phone the one-machine warning is amber and
   asks about the old phone. Mode stays Standby until Start HQ.
7. **Other devices** and paired devices start empty, with one line on why.
8. **A database check** (SQLite's quick check) runs before the first start. If it fails, frame 7. Notes
   only keeps the files and lets HQ make a new database.
9. **From Android 11 or older,** nothing moves. Welcome offers the notes copy and says how to make one.
10. **Restore** accepts the safe-mode .zip as well as .hqbak, with the same swap and the same moved flow
    after.
11. **Server HQ:** nothing of the server moves, the saved copy included. The new phone asks for a new
    sign-in code.
12. **The old phone** explains the move in Backup and about, with the safe-mode button.
13. **At 360 dp and font scale 2.0** the keys card wraps and stays above Add keys.

## 7. Android versions

| Version | Behaviour |
|---------|-----------|
| 8 and 9 | `allowBackup="false"` stops every copy from these phones. Moving needs the notes copy (frame 8). |
| 10 and 11 | The same. The rules file is ignored below Android 12. |
| 12 | The rules file applies when the old phone runs 12 or newer: the vault moves, nothing else. |
| 13+ | The same. The notification permission does not move, so the moved flow asks again (`wave-1/01-notifications.md`). |

Android constraints, as checked on the board:

- Android 12+ with targetSdk 31+: `allowBackup="false"` turns off cloud backup only. Phone-to-phone copy
  follows the device-transfer part of the rules file.
- Excluded by Android from any copy: `noBackupFilesDir`, the caches and the code cache. Keystore keys
  never leave the phone.
- The hq child keeps running while Android copies files, so `vault.db` may be copied mid-write. The
  folder include brings `-wal` along, and rule 8 catches the rest.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/android/app/src/main/AndroidManifest.xml` | Add `android:dataExtractionRules`; keep `allowBackup="false"`. | yes |
| `apps/android/app/src/main/res/xml/data_extraction_rules.xml` | The rules above. | NEW |
| `kotlin/.../ui/firstrun/FirstRun.kt` | A `Moved` step before `Model` in `enum class Step` (line 6). | yes |
| `kotlin/.../ui/firstrun/FirstRunViewModel.kt` | Detects Moved (rule 2), runs the database check and `hq vault moved`. | yes |
| `kotlin/.../ui/firstrun/FirstRunWords.kt`, `FirstRunScreens.kt` | Frames 2 to 9 (words into resources). | yes |
| `kotlin/.../ui/firstrun/Restore.kt` | The .zip path (shared with `02-safe-mode.md`). | yes |
| `java/example/hq/spike/SecretStore.java` | `names()` (line 104): list only names whose value opens, so dead keys never show. | yes |
| `crates/hq-cli` | New `hq vault moved`, clearing the device, bridge, connector and push tables through `hq-db`. | NEW command |
| `noBackupFilesDir` install marker | Written at first run and at every start. | NEW |
| Backup and about (`ui/settings/SettingsScreens.kt`) | The old-phone card (frame 11). | yes |

## 9. Server, release or desktop changes

- **`hq vault moved`** (FixPlan: "clears sign-ins, paired devices, connector tokens and push
  subscriptions on a moved phone"). It runs on the phone's own hq before the first start; nothing on a
  server changes.

## 10. Acceptance checks (each fails on today's build)

1. A real copy from an Android 12+ phone to a freshly reset one, with Google's setup over a cable: on
   the new phone, `run-as` lists `files/hqhome/.vault` and nothing else from the app. No `bridge.key`,
   no `mode`, no `shared_prefs`.
2. The same copy with Samsung Smart Switch from the A32. If the app is not moved at all, write that
   down: it changes the decision text.
3. After the copy: frame 2 shows, Keys on this phone lists no key, Background work is off until Start
   HQ, and the warning asks about the old phone.
4. A browser signed in to the old phone's HQ cannot sign in to the new one with its old token (the
   device table was cleared).
5. Copy while a chat writes to the database: either the notes and tasks open, or frame 7 shows. HQ
   never starts on a broken database.
6. `adb shell bmgr backupnow` on Android 12+: the cloud backup set for AgentHQ is empty.
7. An upgrade from today's build on the same phone never shows frame 2.
8. From an Android 10 emulator, a notes .zip restores on a new phone and the moved flow follows.

## 11. Check on device

- Whether a sideloaded AgentHQ is copied by Google's setup (cable and Wi-Fi) on a Pixel image.
- Whether Samsung Smart Switch on the A32 moves it (FixPlan wave 0: "Smart Switch and Google copy with a
  sideloaded app").
- Whether the restore runs when the app is installed later.

## 12. Open questions and decisions it depends on

Open questions for the owner, with the board's recommendation:

- **Include the vault in a phone-to-phone copy?** Recommendation: yes, with the moved flow on this
  board. It is the only way to move for owners with no computer. If no: the same file with an empty
  device-transfer list, and frame 8 becomes the only path. (Decided: N4, "Yes, with the moved-phone
  flow; keys, config and the bridge key stay behind.")
- **Bring `config.yaml`?** Recommendation: no. It may hold a key typed by hand. The AI key step writes
  it again.
- **Stop HQ on the old phone by itself** when it sees a sign-in from the new one? Recommendation: not
  in wave 0. The warning on Ready to start is enough for now.

Depends on: N4, D7 (spoken mark in HQ), D17 (saved copy), rules sections 5 and 6; `02-safe-mode.md` (the
.zip and Restore); `04-measure.md` (counters in `noBackupFilesDir`).
