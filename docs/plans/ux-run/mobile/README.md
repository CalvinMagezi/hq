# UX run, mobile half: the Android fixes

This folder is the handover for the Android app's UX fixes. It carries everything the "HQ Mobile Design"
canvas boards say (page 2: FixPlan, FixReview, FixRules and twenty fix boards; the as-built Now boards and
the user journey), written so a coding agent with only this repository can build each fix.

How to read it:

1. `../rules-shared.md` first. It is the single rules reference for phone and web, and it wins over every
   spec here. (Web additions live in `rules-web.md`.)
2. Then the wave you are building, in order. Each spec has the same sections: finding and review items,
   Before (with file:line at `c04a100`), Decision, Screens and states, Exact copy, Behaviour rules,
   Android versions, Code touchpoints, server changes, Acceptance checks, Check on device, open
   questions.
3. Where two boards disagree, the rules win, then the revised board. The ones still open are listed under
   "Open conflicts" at the end of this file. Settle each one before you build the screen it touches.
4. `device-checks.md` lists every item that must be tried on a real phone, by wave, with how.

Writing rules for any string you add come from `CLAUDE.md` and rules section 7: "HQ" not "hq", no em
dashes, at most 15 words a sentence, no semicolons, strings in resources.

---

## 1. The app as built (5 Nov 2026)

### How it runs on the phone

- **One Compose activity is the launcher:** `ShellActivity` (label "AgentHQ", app id
  `online.agenthq.app`, Kotlin package `example.hq.spike`). It opens on Chat. First run sits in front of
  everything on a clean install.
- **A foreground service keeps the phone's own hq running:** `HubService` (Java) runs `hq` (packaged as
  `libhq.so`, `HQ_PROFILE=mobile`) as a child process, restarted with backoff. Standby runs the web app
  only; Active (`files/mode` = `active`) adds the daemon, relays and scheduler. Caddy (`libcaddy.so`) runs
  only when the owner supplies a Caddyfile.
- **Every screen talks to that hq over loopback,** `http://127.0.0.1:5678`: REST under `/api` and the chat
  WebSocket. The web token is sealed in the Keystore and sent as a Bearer header, never through a carrier
  proxy.
- **Four web pages are still shown inside the app,** each in `WebHostScreen` (a WebView in the app's own
  screen): Home (`/vault`), Usage (`/usage`), Board and Timeline (`/tasks?view=board|timeline`), each with
  `host=android`, which hides the web app's own bars. Any other link opens in the browser.
- **`MainActivity`** (the old WebView app) is retired; still in the APK, opens only from adb.
- **Notifications:** every 20 seconds `Alerts` asks the local hq for unread items (once a minute for a
  server HQ, with an ETag) and posts them on "Needs you" or "Updates". `AnswerReceiver` sends Allow once /
  Decline answers from the shade.
- **Server HQ:** Settings › Notifications › "Your HQ on a server" signs the phone in to a remote HQ
  (`ServerHq`, device token sealed in the Keystore). Its questions arrive with the same buttons.

### The shell

`HqShell` draws one top bar and one bottom bar on every main screen. The top bar: the place's name, a
status line under it (today "Live" in green, "Starting HQ · {s} s" or "Saved · {age}" in amber; not a
button), the balance chip ("Copilot 912 left"), Search (magnifier) and the gear (Settings). The bottom bar:
five places, Notes, Tasks, Chat (the raised centre button), Helpers, Inbox (badge). Back from any place
returns to Chat, then out. Each place keeps where you were. Pages and sheets cover both bars. While HQ has
never answered on this phone, or has not answered for 60 s, a full screen covers the shell ("Starting HQ",
then "HQ did not start").

### Places, pages and sheets

| Place | Main screen | Pages pushed over it | Sheets and dialogs | Built with |
|-------|-------------|----------------------|--------------------|-----------|
| Chat (opens first) | Name row, conversation or empty start, composer | Select text, Chart, Table | Chats, Add to this chat, Pick a note or task, message actions, note sheet, note menu, Export as | Native |
| Notes | Search pill, folders, Pinned, Recent | Folder, Search, Note, Write | Note menu, Export as, Save to, Save your changes? | Native |
| Tasks | Today, Upcoming, All | Task, Briefing | New task, Filter, Set status, Set priority, Mark N tasks done?, What should change? | Native; Board, Timeline: Web |
| Helpers | Filter chips, helper cards, Finished | Helper, Needs your OK, Technical details | Start a helper, menu, Stop this helper?, Rename | Native |
| Inbox | Needs you, Updates, Done | Details | Undo line | Native |
| Search (top bar) | Notes, tasks and chats | Opens the result in its place | | Native |
| Settings (gear) | Home with four groups | 16 pages, from Background work to Backup and about | Add a key, mode and restart questions | Native; Usage: Web |
| Home | The web app's `/vault`, reached only through `hq://web/home`. No button opens it. | | | Web |

### The user journey (Main board)

1. **Get it** (once per phone): website "Download for Android" (signed APK from the release page) →
   Android "Install unknown apps", then the installer → launcher "Open AgentHQ". Gap: no notification
   prompt; Android 13+ posts nothing until the owner finds Settings, Notifications.
2. **First run** (clean install only): Welcome (Start fresh, or restore a backup) → AI key (checked with
   the service, kept in the Keystore) → Chat apps (Telegram or Discord, optional) → Ready to start (shown
   after Restore too) → Starting HQ (or "HQ did not start", with a retry) → lands on Chat; every later
   launch starts there.
3. **Every day:** the five places. Notes (folders, its own search with Ask HQ, reader, editor with
   Markdown tint), Tasks (Today, Upcoming, All; Filter, pick several, task page, Briefing, queue for the
   edge worker), Chat (composer, slash commands, files and notes; rich reply blocks; Chats sheet with
   swipe to archive), Helpers (coding agents and edge-worker runs; Allow once inline; Needs your OK page;
   Stop), Inbox (Needs you, Updates, Done; answer in place; badge). Top bar on every place: Search (notes,
   tasks, chats), balance chip, gear. Web host pages inside places: Home, Usage, Board, Timeline. Gap: two
   searches.
4. **Ways in from outside:** Hold Home (the assistant sheet, only once HQ is the phone's Digital assistant
   app; listen, answer as text, follow up); a notification (answer from the shade; tap opens Inbox); home
   screen widgets (Capture, Needs you, Tasks); long press on the icon and the share sheet (shortcuts
   Capture, New task, Ask HQ; Capture sheet; offline outbox); `hq://` links (chat, inbox, tasks, helpers,
   notes, settings, search, web). Gaps: the assistant is not discoverable and a locked phone opens nothing;
   closing the sheet loses the answer.
5. **Looking after it** (from the gear): Background work (keep running, Samsung steps), AI service and
   keys (add or replace a key, balance), Spending limits and Messaging apps, Other devices (pairing hub: QR
   code, six digits, paired list, remove), Backup and about (Updates: the only place an update shows, with
   no notification).

### Entry points in the manifest

| Component | Role |
|-----------|------|
| `ShellActivity` | Launcher, the native app. `adb shell am start -n online.agenthq.app/.ShellActivity --es place chat --ei badge 3`; debug fixtures with `--es fixture <name>` (for example `a-alerts`; prefixes `n-`, `i-`, `t-`, `h-` pick the place). |
| `LinkActivity` | A see-through hop for `hq://` links, no history; a link only navigates. |
| `capture.CaptureActivity`, `CaptureWidget`, `NeedsYouWidget`, `TasksWidget` | Share target, widgets, shortcuts (`res/xml/shortcuts.xml`: `capture`, `new_task`, `ask_hq`). |
| `voice.AssistantService`, `AssistantSessionService`, `OnDeviceRecognitionService`, `VoiceActivity` | The phone assistant (Hold Home) and on-device dictation. |
| `ui.pairing.PairingActivity`, `PairLinkActivity` | Pairing a device (`hqpair://`). |
| `HubService`, `BootReceiver`, `AnswerReceiver`, `update.InstallResultReceiver` | The hub, start on boot, answers from the shade, install results. |
| `MainActivity` | Retired WebView app (adb only, `android:permission="android.permission.DUMP"`). |

### Where the code lives

`apps/android/app/src/main/kotlin/example/hq/spike/`:

| Path | What |
|------|------|
| `ShellActivity.kt`, `LinkActivity.kt` | The activity, startup cover choice, fixtures, Search and Settings hosts, links in. |
| `AlertPoster.kt`, `AnswerReceiver.kt`, `ServerHq.kt`, `KeyVault.kt` | Notification text and buttons, shade answers, server HQ sign-in, keys. |
| `data/` | `HqClient.kt` (REST), `ChatConnection.kt`, `ChatEvent.kt` (socket), `HqServices.kt` (`HqStatus`, polling), `Models.kt`. |
| `ui/shell/` | `HqShell.kt` (bars, status line), `Links.kt` (`hq://` parser, `SETTINGS_SCREENS`), `Icons.kt`, `Touch.kt`. |
| `ui/theme/HqTheme.kt` | `Mts` colour tokens and type styles. |
| `ui/chat/`, `ui/chat/blocks/` | Chat screen, composer, Chats sheet, reply blocks, note sheet and export. |
| `ui/notes/`, `ui/tasks/`, `ui/helpers/`, `ui/inbox/`, `ui/search/` | The places, each with `*Screen`, `*ViewModel`, `*Words` and fixtures. |
| `ui/settings/` | `SettingsScreens.kt` (all pages), `SettingsViewModel.kt`, `SettingsWords.kt`, `PhoneWords.kt` (`PhoneFix`), `PhoneIntents.kt`, `BackupFacts.kt`, `ModeSwitch.kt`. |
| `ui/firstrun/` | `FirstRun.kt` (`Step`, `FirstRunStore`), screens, view model, words, `KeyCheck.kt`, `Restore.kt`, `StoredKeysPage.kt`. |
| `ui/startup/` | `Startup.kt` (`FirstLaunch`, `StartupWords.decide`), `StartupScreens.kt`. |
| `ui/webhost/` | `WebHost.kt` (`WebPage`, `allows()`, `LocalOpenWeb`), `WebHostScreen.kt`. |
| `ui/pairing/`, `bridge/` | Pairing screens and the bridge supervisor. |
| `ui/usage/` | Usage words and fixtures. |
| `capture/` | Capture sheet, `Outbox.kt` (`files/outbox.json`), widgets. |
| `voice/` | `Assistant.kt`, `VoiceActivity.kt`, `VoiceSession.kt`, `VoiceTurn.kt`, `Ears.kt`, `VoiceEngines.kt`, `VoiceModels.kt`, `Invocation.kt`. |
| `update/` | `UpdateLogic.kt` (`ChannelManifest`, `verdict()`), `Updater.kt`, `InstallResultReceiver.kt`. |

`apps/android/app/src/main/java/example/hq/spike/`: `HubService.java`, `Alerts.java`, `HubConfig.java`,
`SecretStore.java`, `Loopback.java`, `BootReceiver.java`, `MainActivity.java`, `WebAssets.java`.

Resources: `app/src/main/res/values/strings.xml` (6 strings today), `res/xml/` (shortcuts, voice
interaction service, widgets, file paths, network security). Tests: `app/src/test/kotlin/...` and
`app/src/test/java/...` (JUnit 4 and `org.json` only).

---

## 2. Build, install and test

From `apps/android/README.md`, `docs/HQ_MOBILE.md` and `.github/workflows/android-app.yml`.

**Prerequisites:** a JDK 17 or newer (CI uses 21), the Android SDK with platform `android-36`. Gradle
wraps itself (8.14.3).

```bash
export JAVA_HOME=<a JDK 17 or newer> ANDROID_HOME=<the Android SDK>
cd apps/android
```

**What CI runs** (no hq inside the APK; this is the gate for every PR that touches `apps/android/**`):

```bash
./gradlew :app:assembleDebug :app:testDebugUnitTest :app:lintDebug --no-daemon --console=plain
```

- `:app:testDebugUnitTest` runs the JUnit tests under `app/src/test`.
- `:app:lintDebug` runs Android Lint's `NewApi` check against `minSdk` 26: any call to an API newer than
  Android 8 that is not behind a version check fails the build. Every fix here has Android 8 paths, so run
  it.
- One test class: `./gradlew :app:testDebugUnitTest --tests 'example.hq.spike.ui.shell.LinksTest'`.

**An APK for a phone** (with hq, Caddy and the web app inside). All four `-P` paths must be absolute and
exist:

```bash
# 1. hq for Android (the CI recipe in .github/workflows/android.yml; needs the NDK, API 26 clang wrappers)
cargo build -p hq-cli --profile release-small --target aarch64-linux-android --features tunnel,bridge --locked
# 2. the web app
(cd apps/hq-web && bun install && bun run build)          # gives apps/hq-web/dist/client
# 3. the APK
cd apps/android
./gradlew :app:assembleDebug \
  -Phq.bin=<target/aarch64-linux-android/release-small/hq> -Phq.caddy=<caddy linux arm64> \
  -Phq.web=<apps/hq-web/dist/client> -Phq.keystore=<the signing key>
```

The APK is `app/build/outputs/apk/debug/app-debug.apk`. It is a debug build on purpose (`run-as` works).
A debug build only updates a phone that runs a debug build signed with the same `-Phq.keystore`.

**A copy beside the real app** for phone tests with its own data (never a release id):

```bash
./gradlew :app:assembleDebug -Phq.appId=online.agenthq.<name> <the -P paths above>
adb install -r -g app/build/outputs/apk/debug/app-debug.apk
```

**Install and drive:**

```bash
adb install -r app/build/outputs/apk/debug/app-debug.apk
adb shell am start -n online.agenthq.app/.ShellActivity --es place chat     # no -S: it force-stops hq
adb logcat -s HQHUB            # hub: names imported and the run mode, never a value
adb logcat -s HqVoice          # assistant phases with a clock, never the words
adb shell cmd role add-role-holder android.app.role.ASSISTANT online.agenthq.app 0   # make HQ the assistant (10+)
adb shell input keyevent 219   # the assist key
```

**Release builds:** do not build one locally. CI's `.github/workflows/release.yml` builds and signs the
release APK and `android-channel.json`; `.github/workflows/promote.yml` moves it to `channel-stable`. Owners
install `agent-hq-android.apk` from the GitHub Releases page, then the app updates itself.

**Test infrastructure the specs need that does not exist yet** (add it in wave 0):

- Compose UI tests (FixDestructive, FixSearch, FixPlainWords' word check), screenshot tests with a colour
  probe (FixBackup, FixOffline's `x-` fixtures at 360 dp and font scale 2.0): `app/build.gradle.kts` has
  only `junit:junit` and `org.json`. A JVM route (for example Robolectric with Compose UI test) keeps them
  inside `testDebugUnitTest` so CI runs them.
- Instrumentation tests (FixLockScreen check 2, a test `NotificationListenerService`): there is no
  `app/src/androidTest`, and CI runs no emulator. Either add an emulator job or mark these as device checks.
- The custom lint detector `TextLiteralDetector` (FixPlainWords) needs a lint-checks module; none exists.

---

## 3. The plan (FixPlan, revised after the review and the correction pass, 5 Nov 2026)

"14 findings and 6 review blockers, designed to close. Five waves, safety first." All 14 findings and all
6 blockers are closed in design; what is left is checked on real phones. Sizes: S a day or two, M about a
week, L more. Device checks run before each wave (`device-checks.md`).

| Wave | Goal | Items | Specs |
|------|------|-------|-------|
| 0 | Safety and shared parts | R1 · R3 · M1 · M16 · R5 · M12 | Nothing private in notifications by default; safe mode with "Save a copy of my notes"; data-extraction rules and the moved-phone flow; the "What HQ noticed" counters; the Connection state and the failure catalogue (no UI yet); all five channels; strings moved to resources; kill switches and signed fields in the channel file. |
| 1 | Reach every owner, cheaply | #1 · #9 · #10 · #11 · #13 | Notifications step and the upgrader sheet; a truthful backup row with Save a copy now; the shared dialogs and Undo with the danger colour; plain words with the rendered-text check; one way to count progress. |
| 2 | Stay current without burning data | Downloads · #4 · #7 · Startup | One download engine in the hub (size first, paid-Wi-Fi ask, resume with a hash check, Undo on cancel); updates saved to disk, installed only when HQ is idle, staged and withdrawable; the speech model on the same engine; start steps inside the Connection sheet. |
| 3 | An assistant people find and trust | #2 · #3 · #12 · #8 | Setup card and page on every Android version; a fresh empty sheet on a locked press; the microphone path; answers that survive closing, OK requests that say so, no keep-screen-on beyond 30 s. **Starts only after the locked Home hold is logged on the A32 and a Pixel image.** |
| 4 | One app, one way | #14 · #5 · #6 | The connection UI in every place with saved copies and server HQ rows; one Search with a folder filter on HQ and a Helpers scope; the web Home retired and While you were away made native. |

Wave 0 shared parts with no board of their own (build them from `../rules-shared.md`): the five channels,
created once at the hub's start (section 1); the failure-line catalogue as string resources (section 2);
the `Connection` state type, no UI yet (section 6, shaped in `wave-4/01-offline.md`); strings moved to
resources with the literal lint and its 172-entry baseline reaching zero (section 7, N20,
`wave-1/04-plain-words.md`); the kill switches and new signed fields in the channel file (section 10,
`wave-0/02-safe-mode.md`).

### Each finding and review blocker, its board and where it stands

| # | Problem | Fix, as revised | Board | Size | Still to prove |
|---|---------|-----------------|-------|------|----------------|
| R1 | Lock screen shows text | General words, Open only; "Show details in notifications" off; details only while unlocked | FixLockScreen | M | Details swap on unlock |
| R3 | A bad update can cost the notes | Safe mode copy, idle gate, withdrawn and rollout fields, D11 off | FixSafeMode, FixUpdates | L | A 4 GB zip on 8.1 |
| M1 | Phone-to-phone copy in plain text | Cloud backup takes nothing; transfer takes the vault only, then the moved-phone flow | FixDataTransfer | M | Smart Switch and Google copy |
| 1 | No notification prompt | Step 4 plus the upgrader sheet; a dismissal is not a refusal | FixNotifications | M | Running notice after a grant |
| 9 | False backup alarm | Amber only when it can be cleared; Save a copy now; two more false alarms fixed | FixBackup | S | Where a copy was saved |
| 10 | Stop is green | Side by side, safe left, red right; losing approvals; Undo timeout and outbox | FixDestructive | M | Undo across an HQ stop |
| 11 | Config words | Resources with whole sentences and plurals; check over rendered text; 172 literals to zero | FixPlainWords | M | Five-owner reading test |
| 13 | Progress worded three ways | "{N} of {M} done", unit-tested | FixProgress | S | None |
| 7 | Big downloads | Shared engine, paid-Wi-Fi ask, hash check, Undo | FixDownloads, FixAssistantModel | L | Data Saver; unpacked model size |
| 4 | Updates hidden | Saved to disk, notice, dot, card, Update page, staged rollout | FixUpdates | L | Auto Blocker, verification |
| R4 | Startup vs Offline | Cover on first launch only; steps in the sheet; Stopped by progress, not time | FixStartup, FixOffline | S | None |
| 2 | Assistant not findable | `isActiveService` on all versions; card, page, Android 8 and 9 words | FixAssistantSetup | M | 8 and 9 page labels |
| R2 | Old sheet over the lock screen | A fresh empty sheet; unlock bar only | FixAssistantLocked | S | Screen-record check |
| R6 | Locked hold unverified | Pass, Fail A, Fail B defined; Fail B ships the unlocked assistant only | FixAssistantLocked | M | A32 and Pixel log |
| 12 | Mic refused | Three steps; Type instead after two misses; 8 to 10 dialogs | FixAssistantMic | S | Two apps recording on 8 and 9 |
| 8 | Closing loses the answer | Watcher saved to disk; "HQ needs your OK to finish"; You said; 3 s cancel | FixAssistantHandoff | M | Child process after a kill |
| 14 | Offline is uneven | One Connection state, saved copies, server HQ rows, slow-start rule | FixOffline | L | Saved-copy size cap |
| 5 | Two searches | One Search, folder filter on HQ, Helpers scope | FixSearch | M | None |
| 6 | Web Home has no button | Retired on the phone; While you were away native; `/vault` blocked | FixWebPages | M | None |

R5 (the shared failure words, broken on nine boards) is closed by rules section 2 and lands in wave 0.

### Server and release changes the designs need

- **`GET /api/status/ai`** (signed in): last good and failed call per AI service, no prompt text.
  `/health` stays as it is. (`wave-4/01-offline.md`)
- **`/api/search`** takes a folder, filtered before the limit; rejects "..", a leading "/" and
  backslashes. (`wave-4/02-search.md`)
- **Chat messages** carry a runtime-set source ("voice") and client id in their meta; `turn_end` gains
  `waiting_for_owner`; voice approvals quote "You said". (`wave-3/04-assistant-handoff.md`)
- **Approvals** carry a `loses` flag set by the runtime from the tool's category, never the model.
  (`wave-1/03-destructive.md`)
- **A restore route** for a cleared update, so Undo is shared with the web. (`wave-1/03-destructive.md`)
- **A signed-in busy route** for the idle gate before an install. (`wave-0/02-safe-mode.md`,
  `wave-2/02-updates.md`)
- **`hq vault moved`** clears sign-ins, paired devices, connector tokens and push subscriptions on a moved
  phone. (`wave-0/03-data-transfer.md`)
- **The channel file** gains seq, withdrawn, rollout, switches, size and notes; promote re-signs instead of
  copying; a withdraw workflow. (`wave-0/02-safe-mode.md`, `wave-2/02-updates.md`)
- **`UpdateLogic.kt` hardcodes a real GitHub owner** (`RELEASES_URL`, line 40); move it behind config.

### Decisions (green is the recommendation; "Revised" marks the seven changed after review; N are new from the correction pass)

| Id | Decision | Chosen |
|----|----------|--------|
| D1 | Backup row colour | Amber only when it can be cleared (Save a copy now); grey otherwise. Revised |
| D2 | Dialog buttons | Side by side, safe left, red action right; stack for long labels or big text. Revised |
| D3 | Sign out everywhere | One red dialog. |
| D4 | Early builds switch | "Get test versions", with the can-go-back line. |
| D5 | Pairing relay line | Yes, in plain words. |
| D6 | Telling upgraders about the assistant | Settings row and release notes. |
| D7 | Spoken mark | A runtime-set source field in HQ, shown on the web too. Revised |
| D8 | Paid-data question | Two equal buttons, Wait for Wi-Fi first. Revised |
| D9 | Answers while locked | No. |
| D10 | Update notices | Once per version, a reminder at 7 days, daily when urgent. |
| D11 | Install without confirm | Off until safe mode, the idle gate and a kill switch; never on test versions. Revised |
| D12 | Release notes | Three short lines, signed, by language. |
| D13 | Inbox notifications banner | Not now hides it 7 days. |
| D14 | Paid data | No "don't ask again"; "Used about {N} MB of mobile data". |
| D15 | Start steps from hq | Later; Stopped by progress now. |
| D16 | AI service status | A signed-in route, never `/health`. Revised |
| D17 | Saved copy lifetime | Until replaced, age shown; server data cleared on sign-out. Revised |
| D18 | Stop a task while it finishes | Next round. |
| D19 | Folders by last edited | Once the listing has times. |
| D20 | Search finds helpers | Yes; Inbox stays out. |
| D21 | Native Timeline | Later. |
| N1 | Messages sent offline | Held in the chat up to 24 h, in order, removable, never sent twice. |
| N2 | Saved copy size | 256 KB a place, 2 MB in all, measured first. |
| N3 | Search a server HQ's notes | Wave 4, its own group. |
| N4 | The vault in phone-to-phone copy | Yes, with the moved-phone flow; keys, config and the bridge key stay behind. |
| N5 | Upgraders and details | Details off, a one-time note. |
| N6 | Safe-mode copy | A plain zip, no password, for now. |
| N7 | "Save a copy first" before updates | Only with no backup or copy under 7 days old. |
| N8 | Counters | On by default, one previous version kept. |
| N9 | Knowing a Wi-Fi network | A fingerprint; never ask for location. |
| N10 | "Ask before big downloads" | Spending limits, Mobile data. |
| N11 | Upgrader sheet | Android 13+ only. |
| N12 | A withdrawn version installed | Takes the top Settings card. |
| N13 | Gear dot for an update | Blue. |
| N14 | Rollout | 10% for 2 days, 50% for 2 days, then all; a crash report sets 0. |
| N15 | Almost full | Under 200 MB, tuned by the counters. |
| N16 | A copy off the phone keeps the backup row grey | 7 days; a copy in Downloads does not count. |
| N17 | Who marks an approval as losing | HQ's runtime, from the tool. |
| N18 | Restore a cleared update on HQ | Yes. |
| N19 | Strings | One catalogue that builds both phone and web. |
| N20 | The literal check | Fails on new ones; today's 172 reach zero in wave 0. |
| N21 | If the locked hold fails outright | Ship the unlocked assistant; lock-screen switch off on that model. |
| N22 | Keep the screen on | 30 s while thinking only. |

### The adversarial review in one table (FixReview)

Three reviewers checked each fix against the code at main, the boards against each other, and what nobody
considered. Verdicts before the correction pass: 5 closed, 8 partly, 1 blocked, 6 new blockers. Every item
is now closed in design; the specs carry the corrections.

| # | Verdict | What was open (now addressed in the spec) |
|---|---------|------------------------------------------|
| 1 Notifications | Partly | Upgraders never saw step 4; a dismissed dialog counted as a refusal; wrong claim about where channels are made. |
| 2 Assistant setup | Partly | RoleManager needs Android 10, the app runs on 8. Use `isActiveService`. |
| 3 Locked phone | Blocked | Rested on an unverified locked Home hold; Android 8.0 needs the old window flag. |
| 4 Updates | Partly | A found update lived only in memory; notes not re-signed on promote; the install-confirm claim was wrong. |
| 5 One search | Partly | A folder search filtered only the top 30 hits. |
| 6 Web pages | Closed | "Sent back and fixed" was false; `/vault` still reachable from Board. |
| 7 Big downloads | Partly | Notification Cancel worked two ways; unpacked size a placeholder; Data Saver probably wrong; Wi-Fi paid by the MB treated as free. |
| 8 Never lose an answer | Closed | Closing before the thread id lost it; the watcher died with the process; a turn waiting for an OK reported as "HQ answered". |
| 9 Backup status | Partly | On but missing a folder or key stayed grey; an outside folder silently swapped; six states, not seven. |
| 10 Destructive actions | Closed | Losing approvals still got a green Allow; Delete speech model used the old pattern. |
| 11 Plain words | Partly | The word test read only Words files; inline literals escaped it. |
| 12 Mic refused | Closed | Silence detection missing below Android 10. |
| 13 Progress words | Closed | The acceptance grep could not fail (no-break space). |
| 14 Offline | Partly | Startup kept the 60 s cover; nine boards broke the shared failure words; the backup exclusion not real on 12+. |

Blockers R1 to R6 and majors M1 to M16 are named in the specs where each is closed. Claims the boards got
wrong, now corrected: channels are made in `Alerts.java` and `HubService.java`, not `ShellActivity` line
367 (a debug fixture); RoleManager is not on the minimum Android; the first in-app update can skip the
confirm only with a permission the app lacks; "sent back and fixed" tasks were reopened, not fixed; a
dismissed notification dialog is not a refusal; "Startup and Offline agree" was not true; the speech
model's size and resume already existed, so #7 is mainly paid data and Cancel.

---

## 4. Index of fix specs

| Wave | File | Board | Closes |
|------|------|-------|--------|
| 0 | `wave-0/01-lock-screen.md` | FixLockScreen | R1, M3 (notifications) |
| 0 | `wave-0/02-safe-mode.md` | FixSafeMode | R3, idle gate, withdrawn and rollout, D11 |
| 0 | `wave-0/03-data-transfer.md` | FixDataTransfer | M1, D17 |
| 0 | `wave-0/04-measure.md` | FixMeasure | M16 |
| 1 | `wave-1/01-notifications.md` | FixNotifications | #1, M6, M7 |
| 1 | `wave-1/02-backup.md` | FixBackup | #9, D1 |
| 1 | `wave-1/03-destructive.md` | FixDestructive | #10, D2, N17, N18 |
| 1 | `wave-1/04-plain-words.md` | FixPlainWords | #11, M12, M13 |
| 1 | `wave-1/05-progress.md` | FixProgress | #13 |
| 2 | `wave-2/01-downloads.md` | FixDownloads | #7 (shared rule), D8, M10 |
| 2 | `wave-2/02-updates.md` | FixUpdates | #4, R3 (updates side) |
| 2 | `wave-2/03-assistant-model.md` | FixAssistantModel | #7 (speech model) |
| 2 | `wave-2/04-startup.md` | FixStartup | R4, M9 |
| 3 | `wave-3/01-assistant-setup.md` | FixAssistantSetup | #2 |
| 3 | `wave-3/02-assistant-locked.md` | FixAssistantLocked | #3, R2, R6 |
| 3 | `wave-3/03-assistant-mic.md` | FixAssistantMic | #12 |
| 3 | `wave-3/04-assistant-handoff.md` | FixAssistantHandoff | #8, M4, M5, D7 |
| 4 | `wave-4/01-offline.md` | FixOffline | #14, R4, M8, D16, D17 |
| 4 | `wave-4/02-search.md` | FixSearch | #5, D20, N3 |
| 4 | `wave-4/03-web-pages.md` | FixWebPages | #6 |
| all | `device-checks.md` | FixPlan, every board | Every "Check on device" item |

---

## 5. Open conflicts

Found while carrying the boards over. None was settled silently: each spec quotes its own board, and the
list below says which way the rules point. Decide each before building the screen it touches, then update
both specs.

### Notification words (rules section 1 owns the defaults)

1. **Task approval title.** FixLockScreen: "A task needs your OK". FixNotifications (frame 10 and its copy
   table): "The {task} needs your OK" ("The backup task needs your OK"). FixLockScreen rule 2 says a
   task's own words are never a notification title, and the rules say titles give "who and what kind", so
   "A task needs your OK" fits the rules.
2. **Stuck helper.** FixLockScreen: "{Helper} is stuck · Open HQ to see where it stopped." FixNotifications:
   "A helper is stuck · Open HQ to see which one."
3. **Server question.** Three wordings: FixLockScreen "{Helper} needs your OK · On your server HQ. Open it
   to see what it wants."; FixNotifications "{Helper} needs your OK on your server HQ · Open it to see what
   it wants to do."; FixDestructive A16 header "AgentHQ · your server HQ" with "Open HQ to see what it wants
   to do."
4. **The details switch lines.** FixLockScreen "Off. Notifications say who needs you, not what." / "On
   while the phone is unlocked."; FixNotifications "Off: they say who needs you, not what. You answer in
   the app." / "Details show only while the phone is unlocked. Allow asks you to unlock first.";
   FixPlainWords A4 "Off: they say who and what kind". No-lock line: FixLockScreen "Needs a screen lock" +
   note; FixNotifications "Set a screen lock to use this. Without one, anyone holding this phone can
   answer."
5. **Channel rows in Settings › Notifications.** FixLockScreen draws importance words ("High · sound and
   pop-up", "Min · required while HQ runs", "Low · silent"); FixPlainWords and FixNotifications use
   alert words ("Sound and pop-up", "Silent", "Silent, folded away · Android needs it while HQ runs",
   "Silent · shows progress"); off reads "Off in Android" (FixNotifications) or "Off in Android settings"
   (FixPlainWords). FixPlainWords is the words board and the glossary maps importance to alert words, so
   its lines look right; the "off" phrase still needs one choice.
6. **The running notice.** FixStartup gives Standby a "Background work" action and bodies ("Step {n} of 4 ·
   {step}", "A setting changed. It is back in a few seconds."). FixOffline W1 says "no buttons on any
   version" and draws titles only. Rules section 1 lists the titles and "Tap opens the app"; it is silent
   on actions.

### First run and Settings

7. **The assistant card on Ready to start.** FixAssistantSetup: "Hold Home on any screen and ask by voice.
   Understands English. You can also do this later in Settings." FixNotifications: "Hold Home on any screen
   and ask HQ a question by voice. You can set this up later in Settings." FixDataTransfer: "Optional. Hold
   Home to ask HQ." FixNotifications says the card belongs to FixAssistantSetup, so use its words.
8. **The one-machine line.** FixAssistantSetup "Run HQ on one machine only. A second copy would answer the
   same chats twice." vs FixNotifications "HQ runs all the time on this phone. Run it on one machine only. A
   second copy would answer the same chats twice." (The moved phone has its own amber question, which is
   not in conflict.)

### Safe mode, backup and updates

9. **Where the copy code lives.** FixSafeMode: `safe/SafeModeActivity.kt` (process `:safe`, plain views)
   and `safe/NotesCopy.kt`. FixBackup: `data/SaveCopy.kt`, "shared with safe mode". It must be one
   implementation that the `:safe` process can load without Compose; FixBackup itself says FixSafeMode
   wins.
10. **File name pattern.** FixSafeMode "HQ notes {yyyy-MM-dd HHmm}.zip" vs FixBackup "{yyyy-mm-dd hhmm}".
    In Java patterns `mm` is minutes and `hh` is 12-hour, so use FixSafeMode's.
11. **A switch for Save a copy.** FixBackup rule 13 depends on "the signed channel file switches Save a copy
    off"; FixSafeMode's `switches` example and rules section 10's switch list have no such key.
12. **Withdrawn version words.** FixSafeMode: card "AgentHQ {version} has a problem" / "A fix is coming.
    Save a copy of your notes now, to be safe." / "Save a copy of my notes"; notice "AgentHQ {version} has a
    problem · Open HQ to see what to do."; found then pulled "Up to date · {version} was withdrawn. Nothing
    was installed." FixUpdates: card "This version has a known problem" / "A fixed version is coming. Save a
    copy of your notes now, to be safe." / "Save a copy now"; notice "AgentHQ {version} has a known
    problem."; row "Version {version} was withdrawn. You stay on {installed}."
13. **The channel file's `withdrawn` and `notes`.** FixSafeMode: `withdrawn` is a list of version codes in
    every file (`[92]`), `notes` one string. FixUpdates: `android-channel.sh` writes `withdrawn: false` per
    version and promote's withdraw input sets it `true`; notes are "up to three plain lines of 120
    characters". D12 adds "by language". The list shape is the one that lets a newer file withdraw an older
    installed version (FixSafeMode rules 18 to 20); pick it, and define how notes carry three lines and
    languages.
14. **The lock-screen assistant switch.** FixSafeMode has `switches.lock_screen_assistant` as one boolean;
    FixAssistantLocked rule 10 needs "models listed as failing in the signed channel file". The per-model
    shape is not drawn anywhere.
15. **The install-now dialog layout.** FixUpdates rule 14 says "Wait on the left"; FixSafeMode frame 13
    draws it stacked, which rules section 4 requires because "Stop the chat and install" is over 18
    characters. Stacked wins.

### Connection, startup and offline

16. **Order of building the Connection sheet.** FixStartup (wave 2) puts the start steps "in the Connection
    sheet"; the sheet, the status line and saved copies are FixOffline (wave 4); FixPlan wave 0 builds the
    Connection state with no UI. Build the sheet's start part in wave 2.
17. **The sheet's file.** FixStartup `ui/shell/ConnectionSheet.kt` vs FixOffline
    `ui/connection/ConnectionSheet.kt` (with the strip and banner beside it).
18. **Sheet words that differ.** Starting body: FixStartup "HQ runs on this phone. After a reboot it can
    take up to a minute." vs FixOffline "HQ runs on this phone. You can read and write while you wait."
    Battery line: "Then set Battery to Unrestricted." vs "In Battery, choose Unrestricted for AgentHQ."
19. **"Almost full".** FixStartup (and N15): under 200 MB free for the app's files, {size} up to 200 MB.
    FixOffline rule 6: {size} = the database file's size + 100 MB, less what is free. N15 decides the
    threshold; the {size} formula is still two.
20. **Saved-copy size measurement timing.** FixOffline asks to measure "before wave 0 ends"; FixPlan lists
    it as a wave 4 device check.
21. **Data-extraction rules shape.** FixOffline's Android 12 row describes excluding `savedcopy/`, the
    outbox and spoken ids; FixDataTransfer's file is an include list of `hqhome/.vault` only, which already
    leaves them out. Build FixDataTransfer's file; FixOffline's check (they are absent on the new phone)
    still applies.
22. **Tap-to-top hooks.** FixOffline says `LocalReselect` and `ReselectEffect` stay "as before"; neither
    exists in the code. Re-tapping the selected tab does nothing today, so this is new work.

### Words and strings

23. **The `subtasks_done` plural example.** Its `one` item renders "1 of 3 sub-task done". Choose the plural
    by the total `{M}` (FixProgress says "chosen by the total") and fix the example.
24. **Strings to resources, wave 0 or 1.** FixPlan wave 0 lists "strings moved to resources" and N20 makes
    the 172 literals reach zero in wave 0; the FixPlainWords board is filed under wave 1. Do the move and the
    lint in wave 0, the rewritten words in wave 1.
25. **A failure reason not in the catalogue.** FixDestructive's `fail_remove_device_pairing` ("Pairing is not
    running. Try again in a moment.") and FixSafeMode's "That place is full" and "The place you chose stopped
    answering" are reasons rules section 2 does not list. Add them to the catalogue.
26. **FixAssistantHandoff's flag.** It flags "HQ is fine. Try again soon." because "section 7 bans
    semicolons", but the sentence has none. Nothing to change.

### Links and gaps

27. **`hq://tasks/away`.** Rules section 8 adds it for While you were away; FixWebPages does not list it in
    its touchpoints or tests. Add it to `Links.kt` and `LinksTest` with the page.
28. **"No helpers match that."** FixSearch adds a Helpers scope but gives the no-match line only for notes,
    tasks and chats.
29. **Server HQ notes in Search (N3).** Decided for wave 4 as its own group, but FixSearch draws no frame or
    words for it.
30. **Verification briefing from the phone.** FixWebPages says no server change is needed because the device
    token passes the web auth middleware; the verification routes are described in `CLAUDE.md` as
    owner-only. Confirm the phone's token passes before relying on it.
