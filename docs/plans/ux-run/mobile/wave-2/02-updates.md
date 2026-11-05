# Wave 2 · 02 · Updates that reach the owner (finding #4, with #7 applied)

Board: **FixUpdates** ("Fix updates"), finding #4 with #7 applied, revised after review, 5 Nov 2026.
Area: updates; how one reaches the owner and installs with no retraced taps.
Shared rules: `../../rules-shared.md` sections 1 (App updates channel), 4 (D8), 5 (card slot), 8 (links)
and 10 (D11, rollout, withdrawn) win. The idle gate, withdrawn and rollout also appear on
`../wave-0/02-safe-mode.md`; the download parts are `01-downloads.md`.

Headline: **An update tells the owner once, shows its signed size, survives a restart, and installs only
when HQ is not busy.**

## 1. Finding and the review items it closes

**Problem.** An available update shows only in Settings, Backup and about, with no notification; "needs
Android 29" is an API level; after allowing "Install unknown apps" the owner must tap Check, then Install
again; the download has no size, percentage or Cancel and starts on any network.

**Who and how often.** Every owner, at every release; with no Play Store this is the only way the app is
fixed. The first install from the app also hits the permission detour. Owners on metered data pay for
each download, up to 200 MB. A bad update cannot be undone on the phone (R3), so this board also carries
the idle gate, withdrawn versions and staged rollout from rules section 10.

What changed in this revision:

- **#4, found update lost:** the found channel file and its signature are saved to disk, so the dot and
  the card survive process death (frames 2, 11), with an acceptance check.
- **#4, notes not re-signed:** size and notes are written by `android-channel.sh` and signed at release.
  Withdrawn and the rollout share change later, so promote re-signs the Android channel file (today it
  copies the release's file as it is, `promote.yml:84-100`).
- **#4, install confirm claim:** corrected. An app updating itself qualifies for `setRequireUserAction`,
  and it needs `UPDATE_PACKAGES_WITHOUT_USER_ACTION`, which the manifest lacks. D11 stays off.
- **R3:** install only when HQ is idle (frame 8), withdrawn versions (frame 12), a staged rollout bucket
  kept on the phone, and an urgent flag for daily reminders.
- **M8:** the server HQ's version line (frame 13).
- **M11, M7:** Samsung Auto Blocker and developer verification lines; Android 8 to 11 (frame 14).
- **M12, M14:** FixPlainWords' About words (Build 47, Background work, HQ version), "App updates" for the
  group, no HTTP codes, no "Later", D8 buttons, the Settings home drawn as rules section 5, a blue dot
  (green is never a status), the 4 dp bar.
- **Links (M2):** `hq://settings/update` added to `SETTINGS_SCREENS` and `LinksTest`. It opens the page and
  starts nothing.
- **M15:** a 360 dp frame at font scale 2.0 (frame 15).
- **Acceptance checks** rewritten so each fails on today's build.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/kotlin/example/hq/spike/` unless noted.

- `update/Updater.kt:41` `private val flow = MutableStateFlow<UpdateState>(UpdateState.Idle)`: a found
  update lives in memory only.
- `update/Updater.kt:169` the APK goes to `cacheDir`; `:200` `commit()` commits at once; `:209` the result
  `PendingIntent` is `FLAG_MUTABLE`.
- `update/UpdateLogic.kt:8` `ChannelManifest(version, versionCode, url, sha256, minSdk)`; `:40`
  `RELEASES_URL` hardcodes a real GitHub `<owner>/<repo>` (FixPlan: move it behind config); `:52`
  `STOPPED` ("The install was stopped before it finished. Tap Check for updates, then Install update, and
  confirm each prompt Android shows, including Play Protect's "Install anyway"."); `:53` `BLOCKED`.
- `ui/shell/Links.kt:28` `SETTINGS_SCREENS` has no `"update"`; `app/src/test/.../ui/shell/LinksTest.kt:46`
  `settingsScreensAreAnAllowedSet`.
- `ui/settings/SettingsScreens.kt:982-983` `FactRow("Run mode", "Active"/"Standby")` and `FactRow("HQ", …
  ?: "Not answering")`; `:1019` "Follow every build of main".
- `ui/settings/SettingsViewModel.kt:265` builds "{versionName or "hub"} · build {n}".
- `scripts/release/android-channel.sh:32-34` the jq object `{version, versionCode, url, sha256, minSdk}`;
  `.github/workflows/release.yml:539` signs it with `sign.sh`; `.github/workflows/promote.yml:84-100`
  verifies and uploads it unchanged (the job has `MINISIGN_KEY` at line 70 and signs
  `channel-stable.json` at line 76).
- `AndroidManifest.xml:12` `REQUEST_INSTALL_PACKAGES`; no `UPDATE_PACKAGES_WITHOUT_USER_ACTION`.
- `crates/hq-web/src/api.rs:30` `/health` returns `"version"`.

### Before frames

- **B1 · Settings, Backup and about.** About: "AgentHQ · 1.4.0 · build 42", "Run mode · Active". Updates:
  "Install update · Version 1.5.0 is ready" (pin A), "Follow every build of main · Only builds the owner
  promoted · Off". Support: "Copy diagnostics", "Restart HQ".
- **B2 · Lines the Updates row shows.** "Check for updates · A newer version needs Android 29" (B);
  "Check for updates · Allow AgentHQ to install apps, then try again." (C); "Check for updates ·
  Downloading…" (D); "Check for updates · The install was stopped before it finished. Tap Check for
  updates, then Install update, and confirm each prompt Android shows, including Play Protect's "Install
  anyway"." (E); "Install update · Version 1.5.0 is ready"; "After the app's process is killed: Check for
  updates · Not checked yet" (F).

Pins: **A** the only place an update shows (no notification, no badge, no size). **B** "Android 29" is
an API level. **C** the detour resets the row (the owner retraces two taps). **D** a spinner for up to
200 MB, on any network, with no percentage and no Cancel. **E** a stopped install starts from zero. **F**
a found update lives in memory only (`Updater.kt:41`).

## 3. Decision

**Tell the owner once, in three quiet places; give the update its own page; never make the owner retrace
a step.** A found update is saved to disk, posts one silent notification on App updates, puts a dot on
the gear and a card in the Settings home slot. Every entry opens one Update page with the version, the
signed size and what is new. Download and install runs the shared downloads rule, then, only the first
time, a short sheet before Android's install switch. The pending install is saved, so coming back from
Android starts the download with no tap, and a stopped install keeps its checked file.

**Safe to roll out (R3, rules section 10):** the channel file carries a signed rollout share and a
withdrawn flag; each phone keeps one random bucket; the install waits until HQ is idle; Android's confirm
stays on (D11 off) until safe mode and the idle gate exist, and never for test versions.

Rejected:

- **Download on Wi-Fi without asking.** A 36 to 200 MB file would land on phones short of space before
  the owner chose it, and a paid Wi-Fi Android calls free would be charged.
- **A loud, heads-up update notice.** An update is not urgent; Needs you stays the only channel that makes
  a sound.
- **Asking for the install switch in first run.** It reads as a security warning to a new owner with no
  update in sight.
- **Showing the API level with a gloss** ("API 29, Android 10"). The level means nothing to the owner;
  the table maps it.
- **Skipping Android's confirm now (old D11).** With no downgrade and the notes in app storage, a bad build
  installed unseen could cost the owner their notes.
- **Install the moment it is checked.** The restart would cut a chat answer, a backup or an outbox send in
  half.

## 4. Screens and states (After)

Version numbers and sizes are examples. Progress, the data ask and Cancel are the shared Downloads
pattern.

1. **How it reaches the owner: once** (Android's shade, generic).
   - "Found by the daily check" (pin 1): "AgentHQ · App updates · now" / "A new version of AgentHQ is
     ready." / "AgentHQ 1.5.0 · 36 MB download. Tap to see what is new." (pin 2, no buttons).
   - "Downloaded while the app was closed" (pin 3): "AgentHQ 1.5.0 is downloaded." / "Checked and ready.
     Android will ask you to confirm." / [Install].
   - "After the app restarts" (pin 4): "Updated to 1.5.0" / "AgentHQ is running the new version."
   - "Progress, waiting and paused": the Downloads channel's notice, with Pause and Cancel.
2. **In the app: a dot, then the card slot.** Chat top bar "Chat · Connected · Copilot 912 left" with a
   blue dot on the gear (pin 5; TalkBack "Settings, update ready"). Settings home, card slot (pin 6):
   "Update ready / AgentHQ 1.5.0 · 36 MB download / [See update]". Then This phone: "Background work ·
   On", "Keep running · Good", "Notifications · Needs you on · Updates on · App updates on", "Phone
   assistant · Not set up", "Capture and widgets", "Keys on this phone"; Your accounts: "AI service and
   keys · OpenRouter", "Messaging apps · Telegram", "Other devices · 1 signed in", "Server HQ ·
   hq.example.ts.net"; How HQ behaves: "Spending limits", "Safety and limits"; Your data: "Backup and
   about". Note: "Speech model shows under This phone only once it is downloaded."
3. **The update page (new).** Title "Update AgentHQ". Facts: "On this phone · 1.4.0 · Build 42", "New
   version · 1.5.0 · Build 47". "What is new" (pin 7): three lines "[First line of the release notes]",
   "[Second line …]", "[Third line …]". Card (pin 8): "AgentHQ 1.5.0 · 36 MB download". "You are on
   Wi-Fi." [Download and install] (green, full width, pin 9). [Not now] (text). Footnote: "Only a build
   signed with the AgentHQ release key installs. Android checks the app's signature again before it
   replaces this one."
4. **On mobile data: the shared ask (D8).** Over frame 3: "Download on mobile data?" / "AgentHQ 1.5.0 is
   36 MB. Your data plan may charge for it." / [Wait for Wi-Fi] [Download · 36 MB] (equal outlined, pin
   10).
5. **First time only: before Android's page** (pin 11). Sheet "Let AgentHQ install its updates" /
   "Android asks once. The update goes on by itself when you come back." Numbered: "1 Tap Open Android
   settings", "2 Turn on Allow from this source", "3 Go back to AgentHQ". Buttons [Not now] [Open Android
   settings].
6. **Android's page, switched on** (generic, pin 12). "Install unknown apps · AgentHQ · Allow from this
   source" (on) / "Your phone and personal data are more vulnerable to attack by unknown apps."
7. **Back in the app: already downloading** (pin 13). Update page with the row "AgentHQ 1.5.0 · Over Wi-Fi
   · 45% · 16 of 36 MB · Pause · Cancel". Line: "You can leave this page. The download goes on, and
   Android asks you to confirm when it is ready." Snackbar "Installs allowed. Downloading the update."
8. **Checked: waits for HQ, then Android asks.** Row "AgentHQ 1.5.0 · 36 MB · Installs when HQ is free ·
   Waiting for 1 chat to finish" (blue, pin 14). "Then, once HQ is idle": "AgentHQ 1.5.0 · 36 MB ·
   Checked. Confirm the install in Android's window." with Android's dialog (generic) "AgentHQ / Do you
   want to update this app? / Cancel · Update".
9. **Updated: Backup and about, in plain words** (pin 15). About: "AgentHQ · Build 47", "Background work
   · On", "HQ version · 1.5.0". App updates: "Check for updates · Up to date · 1.5.0 installed today",
   "Get test versions · You get versions marked ready · Off". Snackbar "Updated to 1.5.0 · What is new".
10. **The phone's Android is too old** (pin 16). Card "This update needs a newer Android" / "Version 1.5.0
    needs Android 10 or newer. This phone has Android 9, so HQ stays on 1.4.0 and keeps working." Facts
    "On this phone · 1.4.0 · Build 42", "Android on this phone · 9". Note "If the phone can get a newer
    Android, its own Software update page has it." No button, no notification.
11. **Every line the Install update row can show:**
    - "Check for updates · Up to date · checked today 09:14"
    - "Install update · Version 1.5.0 is ready · 36 MB" (pin 17)
    - "Install update · Version 1.5.0 · waiting for Wi-Fi"
    - "Install update · Downloading 45% · 16 of 36 MB"
    - "Install update · Version 1.5.0 is downloaded · tap to install"
    - "Install update · Installs when HQ is free · Waiting for a backup to finish"
    - "Install update · Waiting for you to confirm the install"
    - "Install update · The install was stopped. Tap to try again." (pin 18)
    - "Install update · Android blocked the update. Tap to try again, then choose Install anyway if Play
      Protect asks."
    - "Install update · Samsung Auto Blocker stopped the update. Open Android settings and turn off Auto
      Blocker." (pin 19)
    - "Install update · Android could not check who made this version. It cannot install yet."
    - "Install update · AgentHQ is not allowed to install apps yet. Tap to allow it."
    - "Check for updates · Couldn't check for updates. Couldn't reach the download server. Check your
      internet and try again."
    - "Check for updates · Version 1.5.0 needs Android 10 or newer"
12. **A version is withdrawn.** The words are FixSafeMode's (`../wave-0/02-safe-mode.md` frame 15), one
    wording on both boards (recommended in `../README.md` "Still open"). "Found, not installed yet":
    "Check for updates · Up to date · 1.5.0 was withdrawn. Nothing was installed." (pin 20). "The dot,
    the card and the notice go. A downloaded file is deleted." "Already installed: the card slot on
    Settings home": "AgentHQ 1.5.0 has a problem / A fix is coming. Save a copy of your notes now, to be
    safe. / [Save a copy of my notes]" (pin 21). "And once, on App updates": "AgentHQ · App updates /
    AgentHQ 1.5.0 has a problem / Open HQ to see what to do."
13. **Backup and about: server owner, test versions on.** About: "AgentHQ · Build 47", "Background work ·
    Only when I open the app", "HQ version · 1.5.0". "Your server HQ · hq.example.ts.net · HQ version 1.5.0
    · up to date" (grey, pin 22). "A week behind:" "hq.example.ts.net · Still on 1.4.0, a week after 1.5.0
    came out. On the server, run hq update." (amber with the mark). App updates: "Check for updates · Up to
    date · checked today 09:14", "Get test versions · New versions before they are marked ready. Some may
    have problems. You can go back only when a regular version is newer. · On" (pin 23).
14. **Android 8 to 11** (pin 24). "Every Android version: the same notice path": "AgentHQ · App updates /
    AgentHQ 1.5.0 is downloaded. / Checked and ready. Android will ask you to confirm. / Install". What
    differs: "Android 8 and 9 would let a background app open Android's confirm. HQ still waits for the
    app to be in front, so every version behaves the same." "Android 10 and 11 forbid that, so the notice
    with Install is the only way. It opens the update page, which shows the confirm." "Below Android 12
    there is no setRequireUserAction: every update confirms, with or without D11." ""Install unknown apps"
    is a switch for each app from Android 8.0 (API 26), the minSdk, so the first-time sheet is the same on
    all of them." "Undo after Cancel lasts 10 s on Android 8 and 9."
15. **360 dp wide, font scale 2.0** (pin 25). "Update AgentHQ", facts stacked ("On this phone · 1.4.0 ·
    Build 42", "New version · 1.5.0 · Build 47"), "36 MB download", [Download and install] full width, [Not
    now], "What is new ↓" below the fold. The page scrolls.

### Taps, end to end (on Wi-Fi, installs already allowed: two taps)

| Step | What happens | Taps |
|------|--------------|------|
| 1 | Daily check finds 1.5.0 in the signed channel file, inside this phone's rollout share. Saved to disk. | no tap |
| 2 | One notification, App updates, silent. Dot on the gear, card on Settings home. | no tap |
| 3 | Tap the notice, or See update: the update page. Download and install. | tap 1 |
| 4 | On a paid connection: Wait for Wi-Fi, or Download · 36 MB. | +1 if paid |
| 5 | First time only: Open Android settings, turn on the switch, Back. | +3, once ever |
| 6 | Downloads and checks in the hub service, with percentage, Pause and Cancel. | no tap |
| 7 | Waits until HQ is idle: no chat turn, backup, outbox send or other download. | no tap |
| 8 | Android's confirm: Update. It stays on, since D11 is off. | tap 2 |
| 9 | Play Protect on some phones: More details, Install anyway. | +2 sometimes |
| 10 | The app restarts, the hub comes back, "Updated to 1.5.0". | no tap |

Today the same path is Check for updates, Install update, Android's Update, plus Check and Install again
after the first-time switch.

### API level to the Android a person knows

| API | Android | API | Android | API | Android |
|-----|---------|-----|---------|-----|---------|
| 26 | 8.0 | 30 | 11 | 34 | 14 |
| 27 | 8.1 | 31 | 12 | 35 | 15 |
| 28 | 9 | 32 | 12L | 36 | 16 |
| 29 | 10 | 33 | 13 | 37+ | a newer Android |

The phone's own version comes from `Build.VERSION.RELEASE`, as Android's About phone page shows it.

## 5. Exact copy

Notifications, channel App updates:

| Kind | String |
|------|--------|
| Found | "A new version of AgentHQ is ready." · "AgentHQ {version} · {size} download. Tap to see what is new." (no buttons; the tap opens `hq://settings/update`) |
| Reminder, day 7 | "A new version of AgentHQ is ready." · "AgentHQ {version} is still waiting. Tap to see what is new." |
| Downloaded | "AgentHQ {version} is downloaded." · "Checked and ready. Android will ask you to confirm." · "Install" |
| Installed | "Updated to {version}" · "AgentHQ is running the new version." |
| Withdrawn, installed | "AgentHQ {version} has a problem" · "Open HQ to see what to do." (FixSafeMode's words) |

In the app:

| Element | String |
|---------|--------|
| Gear, for TalkBack | "Settings, update ready" |
| Home card | "Update ready" · "AgentHQ {version} · {size} download" · "See update" |
| Home card, withdrawn | "AgentHQ {version} has a problem" · "A fix is coming. Save a copy of your notes now, to be safe." · "Save a copy of my notes" (FixSafeMode's words) |
| Page title | "Update AgentHQ" |
| Facts | "On this phone" · "New version" · "Android on this phone" · "{version} · Build {n}" |
| Notes heading | "What is new" |
| Network line | "You are on Wi-Fi." / "You are on mobile data. HQ asks before it uses any." |
| Buttons | "Download and install" · "Not now" |
| Footnote | "Only a build signed with the AgentHQ release key installs. Android checks the app's signature again before it replaces this one." |
| Sheet | "Let AgentHQ install its updates" · "Android asks once. The update goes on by itself when you come back." · "Tap Open Android settings" · "Turn on Allow from this source" · "Go back to AgentHQ" · "Not now" · "Open Android settings" |
| Back, allowed | "Installs allowed. Downloading the update." |
| Leaving the page | "You can leave this page. The download goes on, and Android asks you to confirm when it is ready." |
| Idle gate (FixSafeMode) | Row: "Installs when HQ is free · Waiting for 1 chat to finish". Install tapped while busy: "Install now?" · "HQ is answering a chat. Installing now stops it." · "Wait until it's done" · "Stop the chat and install" |
| Checked | "Checked. Confirm the install in Android's window." |
| Snackbar after | "Updated to {version}" · "What is new" |
| Too old, card | "This update needs a newer Android" · "Version {version} needs Android {name} or newer. This phone has Android {release}, so HQ stays on {installed} and keeps working." |
| Too old, note | "If the phone can get a newer Android, its own Software update page has it." |

Backup and about (FixPlainWords):

| Element | String |
|---------|--------|
| About | "AgentHQ · Build {n}" · "Background work · On / Only when I open the app" · "HQ version · {version}" |
| Group | "App updates" |
| Up to date | "Up to date · checked today {time}" / "Up to date · {version} installed today" |
| Ready | "Version {version} is ready · {size}" |
| Waiting | "Version {version} · waiting for Wi-Fi" |
| Downloading | "Downloading {pct}% · {done} of {size}" |
| Downloaded | "Version {version} is downloaded · tap to install" |
| Stopped | "The install was stopped. Tap to try again." |
| Blocked | "Android blocked the update. Tap to try again, then choose Install anyway if Play Protect asks." |
| Auto Blocker | "Samsung Auto Blocker stopped the update. Open Android settings and turn off Auto Blocker." |
| Not verified | "Android could not check who made this version. It cannot install yet." |
| Not allowed yet | "AgentHQ is not allowed to install apps yet. Tap to allow it." |
| Check failed | "Couldn't check for updates. Couldn't reach the download server. Check your internet and try again." |
| Withdrawn, found | "Up to date" · "{version} was withdrawn. Nothing was installed." (FixSafeMode's words) |
| Too old | "Version {version} needs Android {name} or newer" |
| Test versions | "Get test versions" · off: "You get versions marked ready" · on: "New versions before they are marked ready. Some may have problems. You can go back only when a regular version is newer." |
| Server HQ | "Your server HQ" · "{address}" · "HQ version {version} · up to date" / "Still on {old}, a week after {new} came out. On the server, run hq update." |
| Cancel a download | Acts at once, then "Download cancelled · Undo" for the accessibility timeout (rules section 4) |
| Kept as shipped | "Not checked yet" · "Waiting for you to confirm the install" · the signature, hash, size and format failures, reworded to the rules section 2 form |

The withdrawn wording is `../wave-0/02-safe-mode.md`'s, so both boards say the same thing; the owner may
still choose the other wording (`../README.md` "Still open").

## 6. Behaviour rules

1. **Checking** stays once a day while the hub runs, first 10 minutes after it starts; also when the
   update page or Backup and about opens and the last check is over an hour old. The channel file and its
   signature are under 1 KB. A version shows only when the phone's bucket (0 to 99, drawn once and kept on
   the phone) is below the signed rollout share.
2. **One notice per version** (same id, a newer one replaces it), low and silent on App updates. The tap
   opens `hq://settings/update` and starts nothing. One reminder after 7 days; daily only when the channel
   file marks the version urgent.
3. **Finished in the background:** the "is downloaded" notice. Install opens the update page, which shows
   Android's confirm once HQ is idle, since Android 10 and newer do not let a background app open it.
4. **After the install** Android restarts the app and the hub (`MY_PACKAGE_REPLACED`, as today); the app
   posts "Updated to {version}" and shows the snackbar on the first open.
5. **The gear's dot** (blue) shows while an update is ready and its page has not been opened.
6. **The card** sits in the one Settings home slot and stays until it is installed; Not now on the page
   hides card and dot for 7 days.
7. **What is new** is the channel file's notes field, a map from language to up to three plain lines of
   120 characters (`{"en": ["…", "…"]}`, `../../resolutions.md` D-3, D12), written at release and signed
   with the rest. The phone shows its own language, else `en`. Left out when missing.
8. **The size** is the channel file's size field, in bytes, signed. The download must match it as well as
   the SHA-256. A channel file with no size shows no size and no notice; the row says "Version {version}
   is ready".
9. **Download and install** runs the Downloads rule: space check (twice the size + 10 MB), then the ask on
   a paid connection.
10. **The ask** is the D8 dialog: two equal buttons, Wait for Wi-Fi first.
11. **First time only,** when `canRequestPackageInstalls()` is false: this sheet, then Android's page. Not
    now leaves the update ready.
12. **Back from Android** returns to the update page. The install request (version code, any data
    consent) is saved to disk before leaving, because some phones restart the app when this switch
    changes.
13. **On return** the page re-checks the switch. On: the download starts by itself with the snackbar.
    Off: "not allowed yet", and a tap opens Android's page again.
14. **Idle gate:** the checked file goes to Android's installer only when HQ has no chat turn running, no
    backup, no outbox send and no other download. Otherwise the row says what it waits for and it goes on
    by itself; Install tapped while busy shows FixSafeMode's "Install now?" dialog, Wait on the left.
15. **Backup and about** uses FixPlainWords' words: AgentHQ · Build {n}, Background work, HQ version. The
    group is "App updates".
16. **Too old:** Android's version name from the table, the phone's own version, no button and no
    notification.
17. **Saved found update:** the channel file and its signature are written to the app's files when found,
    checked again on load, and drive the dot, card and row after a restart. Deleted after the install, or
    when withdrawn.
18. **Stopped or blocked:** the checked file is kept, so a tap goes back to Android's confirm with no new
    download. It is deleted after a good install or when a newer version is found.
19. **Blocked by the phone:** Samsung Auto Blocker and developer verification get their own lines, picked
    from the installer's status and message. The exact values: check on device.
20. **Withdrawn before install:** the version is never offered again; the dot, card and notice go, and a
    downloaded file is deleted.
21. **Withdrawn after install:** the Settings card with Save a copy now (the Android file picker, shared
    with FixSafeMode and FixBackup), and one notice. It clears when a newer version installs.
22. **Server HQ version:** read from the server's `/health` "version" (`crates/hq-web/src/api.rs:30`),
    compared with the signed stable channel. Grey while current or under a week behind; amber with the
    mark after that.
23. **Get test versions** follows the main channel. Android cannot install an older version, so turning it
    off waits until a regular version is newer; the line says so.
24. **Android 8 to 11:** the same notice path everywhere; every update confirms.
25. **360 dp at font scale 2.0:** the facts stack, the page scrolls, Download and install stays full
    width. Nothing is cut.

(Rule 14 says "Wait on the left"; FixSafeMode frame 13 draws that dialog stacked because "Stop the chat
and install" is over 18 characters. Rules section 4 wins: stacked, safe on top.)

## 7. Android versions

| Version | What this fix does there |
|---------|--------------------------|
| 8 and 9 | "Install unknown apps" per app (API 26). The confirm always comes from the update page with the app in front, though Android would allow more. Every update confirms. Undo lasts 10 s. |
| 10 and 11 | A background app may not start the confirm (`STATUS_PENDING_USER_ACTION`), so a download finished in the background posts "is downloaded" with Install. Undo follows the accessibility timeout. |
| 12 | `setRequireUserAction` exists (API 31), but D11 stays off (rules section 10). The installer's result intent stays `FLAG_MUTABLE`, as `Updater.kt:209` has it, since the installer adds its status. |
| 13+ | Without notification permission, the dot, the card and the row still say it. Where Android's developer verification applies (from 2026, country by country), an unverified build is blocked: the release must be signed by a verified developer. |

Android constraints, as checked on the board:

- `canRequestPackageInstalls()` (API 26) needs `REQUEST_INSTALL_PACKAGES`, in the manifest (line 12).
  `ACTION_MANAGE_UNKNOWN_APP_SOURCES` with a `package:` URI opens this app's switch. Android returns no
  answer, so the app reads it again on return.
- **PackageInstaller:** `STATUS_PENDING_USER_ACTION` carries Android's confirm Intent. From Android 10 a
  background app may not start it, so the receiver starts it only while the app is visible, else posts
  the "is downloaded" notice.
- `setRequireUserAction(USER_ACTION_NOT_REQUIRED)` (API 31) can skip the confirm when the installer is the
  app's installer of record or the app is updating itself, as here. It also needs the
  `UPDATE_PACKAGES_WITHOUT_USER_ACTION` permission, which the manifest does not have, and newer Androids
  add target SDK conditions. Check on device before D11 is reconsidered.
- **Developer verification:** Google is rolling out a rule that certified Android phones install only
  apps from verified developers, sideloaded ones included, starting in some countries in 2026. AgentHQ's
  release key must be registered before it reaches owners' countries.
- **Samsung Auto Blocker** (One UI 6 and newer) blocks installs from unknown sources while on. The A32's
  last One UI may not have it.
- **Session progress** covers only staging and commit, near instant for a local file; the percentage
  comes from the download.
- **Metered and space:** as on the Downloads board. An update needs twice its size plus 10 MB: the file,
  and Android's copy in the session.
- **Some phones restart the app** when the install switch changes; the pending request on disk covers it.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `scripts/release/android-channel.sh` (lines 32 to 34) | Add `size` (the APK's bytes), `notes` (a map from language to up to three lines), `withdrawn` (a list of versionCodes, empty at release), `rollout` and `urgent` to the jq object, plus FixSafeMode's `seq`, `issued_at` and `switches` (with `save_copy` and `lock_screen_assist_off_models`), all as `../../resolutions.md` D-3 shapes them. `release.yml:539` signs it as today. | yes |
| `.github/workflows/promote.yml` (lines 84 to 100) | Today it verifies and copies `android-channel.json` unchanged. It must set the rollout share, re-sign with `sign.sh` (the job already has `MINISIGN_KEY`, line 70, and signs `channel-stable.json` at line 76), and upload both. A withdraw input adds the version's versionCode to the `withdrawn` list and re-signs (D-3). | yes |
| `kotlin/.../update/UpdateLogic.kt` | `ChannelManifest` (line 8) gains `size`, `notes` (a language map), `withdrawn` (a list of versionCodes), `rollout`, `urgent`, all optional for older files (D-3); `androidName(sdk)`; `words()` rewritten; `STOPPED` and `BLOCKED` (lines 52, 53) shortened; failures in the rules section 2 form. Also move `RELEASES_URL` (line 40) behind config. | yes |
| `kotlin/.../update/Updater.kt` | The state (line 41) is rebuilt from the saved channel file at start; the bucket is drawn once into prefs; the idle gate before `commit()` (line 200); the checked APK moves from `cacheDir` (line 169) to `noBackupFilesDir` and is kept after an abort. | yes |
| Idle gate | HQ's busy state (chat turn, backup, outbox send) from a signed-in route, plus `LargeDownload`'s own state. Shared with FixSafeMode. | NEW route |
| `kotlin/.../update/InstallResultReceiver.kt` | Start the confirm only while visible, else notify; map Auto Blocker and verification. | yes |
| `kotlin/.../update/UpdateNotices.kt` | The App updates notices. | NEW |
| `kotlin/.../ui/shell/Links.kt:28` | `SETTINGS_SCREENS` gains `"update"`. | yes |
| `app/src/test/.../ui/shell/LinksTest.kt:46` | `settingsScreensAreAnAllowedSet` asserts `hq://settings/update` opens "update" and that a link carries no action. | yes |
| `kotlin/.../ui/settings/SettingsScreens.kt:982-983` | "Run mode" becomes Background work, "HQ" becomes HQ version, and "Not answering" becomes the saved value. Line 1019: "Follow every build of main" becomes Get test versions. The Update page, the home card, the server line. | yes |
| `kotlin/.../ui/settings/SettingsViewModel.kt:265` | "Build {n}" without "hub". | yes |

## 9. Server, release or desktop changes

- **The channel file** gains `size`, `notes` (a map from language to up to three lines), `withdrawn` (a
  list of versionCodes), `rollout`, `urgent` (and, per FixSafeMode, `seq`, `issued_at`, `switches` with
  `save_copy` and `lock_screen_assist_off_models`), as `../../resolutions.md` D-3 shapes them;
  **promote re-signs instead of copying**; **a withdraw workflow**.
- **A signed-in busy route** in `crates/hq-web` for the idle gate.
- **`UpdateLogic.kt` hardcodes a real GitHub owner** (`RELEASES_URL`): move it behind config (FixPlan).
- The server HQ version reads `/health`'s `version`, which exists today.

## 10. Acceptance checks (each fails on today's build)

1. An update found, then `adb shell am kill` on the app, then reopen with no network: the dot and the
   Settings card are still there, with the size.
2. Promote a test tag: `channel-stable`'s `android-channel.json` carries size, notes, withdrawn and
   rollout, and `minisign -V` passes over those bytes. Change one byte of the notes: the app shows no
   update.
3. LinksTest: `parse("hq://settings/update").screen` is "update". Opening it on the phone shows the page
   and starts no download.
4. Start a long chat turn, then tap Install: "Install now?" with Wait until it's done. Wait: the row reads
   "Installs when HQ is free · Waiting for 1 chat to finish", and Android's confirm shows only after the
   turn ends.
5. A test channel marks the found version withdrawn: the dot, card and notice go, the downloaded file is
   deleted, the row reads "Up to date · 1.5.0 was withdrawn. Nothing was installed."
6. Rollout share 0: no notice, no dot. Share 100: the notice. The bucket is the same after an app update.
7. Without urgent: one reminder after 7 days, then nothing. With urgent: one a day.
8. Backup and about reads AgentHQ · Build {n}, Background work, HQ version; no "Run mode", no "hub", no
   "Not answering".
9. Get test versions on: the line ends "You can go back only when a regular version is newer."
10. Wi-Fi, installs not yet allowed: Download and install, the sheet, Open Android settings, switch on,
    Back: the download is running with no tap.
11. Android 9 emulator, a channel file with minSdk 29: "needs Android 10 or newer"; no notification.
12. Signed in to a server HQ: Backup and about shows its HQ version, read from that server.
13. Cancel at Android's confirm: "The install was stopped. Tap to try again."; tap: the confirm is back,
    and the app's data usage has not grown.
14. 360 dp emulator at font scale 2.0: the update page scrolls, nothing is cut, no sideways scroll.

## 11. Check on device

- Samsung Auto Blocker on a newer Galaxy (One UI 6+); the exact installer status and message for Auto
  Blocker and for developer verification (rule 19).
- What Android shows when developer verification blocks an install (13+).
- `setRequireUserAction` conditions before D11 is reconsidered.
- Whether the A32 restarts the app when the install switch changes.
- FixPlan "Still to prove" for #4: Auto Blocker, verification.

## 12. Open questions and decisions it depends on

- **Decided (rules section 1):** one notice per version, one reminder at 7 days, daily only when the
  channel file marks it urgent (D10).
- **New: the rollout steps.** Recommendation: 10% for two days, 50% for two days, then 100%, set at
  promote. A crash-on-start report sets the share back to 0. (Decided: N14.)
- **New: a withdrawn version that is already installed** needs the Settings card slot, which rules section
  5 lists as Update ready, Notifications off, Backup. Recommendation: it goes first, above Update ready.
  (Decided: N12, "Takes the top Settings card.")
- **New: the gear's dot colour.** Green is never a status and amber means "needs you now".
  Recommendation: blue. (Decided: N13.)

Depends on: D4, D8, D10, D11, D12, N12, N13, N14, `../../resolutions.md` D-3, rules sections 1, 4, 5, 8, 10;
`../wave-0/02-safe-mode.md` (idle gate, withdrawn, seq, switches); `01-downloads.md`;
`../wave-1/04-plain-words.md` (About words, "Get test versions").
