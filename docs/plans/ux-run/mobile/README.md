# UX run, mobile half: the Android fixes

This folder is the handover for the Android app's UX fixes. It carries everything the "HQ Mobile Design"
canvas boards say (page 2: FixPlan, FixReview, FixRules and twenty fix boards; the as-built Now boards and
the user journey), written so a coding agent with only this repository can build each fix.

How to read it:

1. `../resolutions.md` first: the last word on every conflict between the phone, web and hub boards. Then
   `../decisions.md`, then `../rules-shared.md` (with `../rules-web.md` one set of rules; `rules-shared.md`
   wins where they differ and `resolutions.md` is silent). All three win over every spec here.
2. Then the wave you are building, in order. Each spec has the same sections: finding and review items,
   Before (with file:line at `c04a100`), Decision, Screens and states, Exact copy, Behaviour rules,
   Android versions, Code touchpoints, server changes, Acceptance checks, Check on device, open
   questions.
3. Where two boards disagree, `../resolutions.md` wins, then `../decisions.md`, then the rules, then the
   revised board. Section 5 at the end of this file lists every conflict found while carrying the boards
   over, with the resolution that settles it; the few that no resolution covers are under "Still open",
   each with a recommendation the specs already follow.
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

From `../testing.md` (which wins if this section and it ever differ), `apps/android/README.md`,
`docs/HQ_MOBILE.md` and `.github/workflows/android-app.yml`.

**Prerequisites:** a JDK 17 or newer (CI uses 21), the Android SDK with platform `android-36`. Gradle
wraps itself (8.14.3).

**Before every hand-over** (exactly as `../testing.md`):

```bash
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace            # read the totals: a partial run is not a green run
cd apps/hq-web && bun install && bun run build && bun test
cd apps/android && ./gradlew :app:assembleDebug :app:testDebugUnitTest :app:lintDebug
cd apps/desktop && bun install && bun run typecheck     # when the hub changed
cargo test -p hq-desktop          # includes shell_config.rs (commands, capabilities, no plugins)
```

Then write the wave's "Try this now" note in `../progress.md`.

- CI runs the same Gradle line with `--no-daemon --console=plain` on every PR that touches
  `apps/android/**` (no hq inside the APK).
- `:app:testDebugUnitTest` runs the JUnit tests under `app/src/test`.
- `:app:lintDebug` runs Android Lint's `NewApi` check against `minSdk` 26: any call to an API newer than
  Android 8 that is not behind a version check fails the build. Every fix here has Android 8 paths, so run
  it.
- One test class: `./gradlew :app:testDebugUnitTest --tests 'example.hq.spike.ui.shell.LinksTest'`.

**A test copy for a phone,** installed beside the real app with its own data (`../testing.md`, "The
phone"):

```bash
export JAVA_HOME=<a JDK 17 or newer> ANDROID_HOME=<the Android SDK>
cd apps/android
./gradlew :app:assembleDebug \
  -Phq.bin=<hq aarch64-linux-android> -Phq.caddy=<caddy linux arm64> \
  -Phq.web=<apps/hq-web/dist/client> -Phq.keystore=<the signing key> \
  -Phq.appId=<a test id>
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Build `hq` for Android with `--features tunnel,bridge` (the recipe in `.github/workflows/android.yml`:
`cargo build -p hq-cli --profile release-small --target aarch64-linux-android --features tunnel,bridge
--locked`, which needs the NDK and the API 26 clang wrappers; the web app comes from `cd apps/hq-web &&
bun install && bun run build`, giving `apps/hq-web/dist/client`). All four `-P` paths must be absolute and
exist; keep them in `~/.gradle/gradle.properties`, never in the repository. The APK is a debug build on
purpose (`run-as` works), and it only updates a phone that runs a debug build signed with the same
`-Phq.keystore`. Never use a release id as the test id.

**Drive it:**

```bash
adb shell am start -n online.agenthq.app/.ShellActivity --es place chat     # no -S: it force-stops hq
adb logcat -s HQHUB            # hub: names imported and the run mode, never a value
adb logcat -s HqVoice          # assistant phases with a clock, never the words
adb shell cmd role add-role-holder android.app.role.ASSISTANT online.agenthq.app 0   # make HQ the assistant (10+)
adb shell input keyevent 219   # the assist key
```

(With `-Phq.appId`, use that id in place of `online.agenthq.app`.)

**Phone as the hub:** pair the laptop with `hq bridge` or HQ Hub's Devices tab (`docs/PAIRING.md`), then
open HQ in HQ Hub and sign in once with a code from the phone (Settings › Other devices).

**Release builds:** never build one locally. CI's `.github/workflows/release.yml` builds and signs the
release APK and `android-channel.json`; `.github/workflows/promote.yml` moves it to `channel-stable`
(`docs/UPDATE_SYSTEM.md`). Owners install `agent-hq-android.apk` from the GitHub Releases page, then the
app updates itself.

**Test setup wave 0 adds** (`../resolutions.md` D-11; acceptance checks that need it are written in the
specs that use it):

- Compose UI tests on the JVM (Robolectric with Compose UI test, so they run inside `testDebugUnitTest` and
  CI runs them; instrumented tests where a check needs a real device), for FixDestructive, FixSearch and
  FixPlainWords' word check. `app/build.gradle.kts` has only `junit:junit` and `org.json` today.
- A screenshot test with a colour probe (FixBackup, FixOffline's `x-` fixtures at 360 dp and font scale
  2.0).
- An Android lint-checks module for the string rules (FixPlainWords' `TextLiteralDetector`); none exists.
- Instrumentation tests (FixLockScreen check 2, a test `NotificationListenerService`): there is no
  `app/src/androidTest`, and CI runs no emulator. Add an emulator job, or run these as device checks.

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
created once at the hub's start (section 1); the failure-line catalogue as string resources (section 2,
which is `../resolutions.md` A2); the Connection data model and words, no UI yet (`data/Connection`,
`ConnectionWords`, section 6, shaped in `wave-4/01-offline.md`; the sheet's UI comes in wave 2,
`../resolutions.md` D-5); strings moved to resources with the literal lint and its 172-entry baseline
reaching zero (section 7, N20, `wave-1/04-plain-words.md`; the rewritten words land in wave 1, D-8); the
kill switches and new signed fields in the channel file in D-3's shapes (section 10,
`wave-0/02-safe-mode.md`); the test setup of section 2 (D-11); the saved-copy size measured on a
2,000-note vault (N2).

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
- **Approvals** carry a `loses` flag set by the runtime from the tool's category, never the model, and
  the tool and its arguments for the card's quote (`../resolutions.md` B1); they lapse after 24 hours
  (C-7). (`wave-1/03-destructive.md`)
- **Restore routes** for a cleared update and every other Undo, accepted for 10 minutes, shared with the
  web (A6, C-1). (`wave-1/03-destructive.md`)
- **Client ids** on new notes and tasks, and a version on note saves, shared with the web (C-2, C-3).
  (`wave-4/01-offline.md`)
- **Device sign-ins** write a Needs you item and expire after 90 days without use, on the phone hub too
  (C-4). (`wave-1/03-destructive.md`)
- **A signed-in busy route** for the idle gate before an install. (`wave-0/02-safe-mode.md`,
  `wave-2/02-updates.md`)
- **`hq vault moved`** clears sign-ins, paired devices, connector tokens and push subscriptions on a moved
  phone. (`wave-0/03-data-transfer.md`)
- **The channel file** gains seq, withdrawn (a list of versionCodes), rollout, switches (with `save_copy`
  and `lock_screen_assist_off_models`), size and notes (a map from language to up to three lines), as D-3
  shapes them; promote re-signs instead of copying; a withdraw workflow. (`wave-0/02-safe-mode.md`,
  `wave-2/02-updates.md`)
- **`UpdateLogic.kt` hardcodes a real GitHub owner** (`RELEASES_URL`, line 40); move it behind config.

### Decisions

Decisions live in `../decisions.md` (D, N, W, WR) and the cross-platform settlements in
`../resolutions.md`; this file does not repeat them. The ids each wave depends on:

| Wave | `decisions.md` | `resolutions.md` |
|------|----------------|------------------|
| 0 | D11, D12, D17, N2, N4, N5, N6, N8, N12, N14, N19, N20, N21 | A1, A2, A4, A5, B19, D-1, D-3, D-4, D-5, D-8, D-11 |
| 1 | D1, D2, D3, D4, D5, D13, N5, N11, N16, N17, N18, N19, N20, N23 | A2, A3, A4, A6, B1 to B5, B8, B10, B14, B15, B18, B19, C-1, C-4 to C-8, D-1, D-4, D-6, D-7, D-8, D-12 |
| 2 | D8, D10, D11, D12, D14, D15, N7, N9, N10, N12, N13, N14, N15 | A2, A7, B17, D-2, D-3, D-5 |
| 3 | D6, D7, D9, N21, N22 | A2, A4, B1, B18, D-3, D-6 |
| 4 | D16, D17, D18, D19, D20, D21, N1, N2, N3, N15, N24, N25 | A2, A3, A7, B6, B7, B16, C-2, C-3, C-9, C-10, D-5, D-9, D-10, D-12 |

Owner to confirm before the part that depends on it: N4, N6, N19, N22, N25 (the full list is in
`../decisions.md`). Later, after this run: resume a stopped helper and archived helpers on the phone, and
a server HQ running Lite (`../resolutions.md` C-15).

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

## 5. Conflicts found while carrying the boards over

Each one is now settled, and the specs say what the settlement says. "Still open" at the end lists the
few that no resolution covers, each with the recommendation the specs already follow.

### Notification words

1. **Task approval title.** Settled: resolutions.md B19 ("A task needs your OK", never the task's name).
2. **Stuck helper.** Settled: resolutions.md B19 ("A helper is stuck · Open HQ to see which one.").
3. **Server question.** Settled: resolutions.md B19 (title "Your server HQ needs your OK"; both specs use
   the body "Open HQ to see what it wants to do.").
4. **The details switch lines.** Settled: resolutions.md D-1 for the off line, FixPlainWords' "Off: they
   say who and what kind". The on line and the no-lock lines are not FixPlainWords' (Still open 1).
5. **Channel rows in Settings › Notifications.** Settled: resolutions.md D-1 (FixPlainWords' lines; off
   reads "Off in Android settings").
6. **The running notice.** Settled: resolutions.md D-2 (no buttons on any version; a tap opens the app).

### First run and Settings

7. **The assistant card on Ready to start.** Settled: resolutions.md D-6 (FixAssistantSetup's body).
8. **The one-machine line.** Settled: resolutions.md D-6 (FixAssistantSetup's line).

### Safe mode, backup and updates

9. **Where the copy code lives.** Settled: resolutions.md D-4 (`safe/NotesCopy.kt`, used by safe mode's
   `:safe` process and Backup's Save a copy now).
10. **File name pattern.** Settled: resolutions.md D-4 (FixSafeMode's "HQ notes {yyyy-MM-dd HHmm}.zip").
11. **A switch for Save a copy.** Settled: resolutions.md D-3 (`switches.save_copy`, a boolean).
12. **Withdrawn version words.** Still open 2.
13. **The channel file's `withdrawn` and `notes`.** Settled: resolutions.md D-3 (`withdrawn` a list of
    versionCodes; `notes` a map from language to up to three lines).
14. **The lock-screen assistant switch.** Settled: resolutions.md D-3 (`lock_screen_assist_off_models`, a
    list of model names, for N21).
15. **The install-now dialog layout.** Settled: decisions.md D2 and rules-shared section 4 (a label over
    18 characters stacks, so "Stop the chat and install" stacks). No resolution was needed.

### Connection, startup and offline

16. **Order of building the Connection sheet.** Settled: resolutions.md D-5 (data model and words in wave
    0, the UI in wave 2 with Startup, extended in wave 4).
17. **The sheet's file.** Settled: resolutions.md D-5 (`ui/connection/`).
18. **Sheet words that differ.** Still open 3.
19. **"Almost full".** Settled: resolutions.md D-5 (one formula: under 200 MB free for the app's files,
    N15; {size} is 200 MB less what is free).
20. **Saved-copy size measurement timing.** Settled: decisions.md N2 (measured on a 2,000-note vault in
    wave 0) and resolutions.md B16 (one cap on phone and web, 256 KB a place, 2 MB in all).
21. **Data-extraction rules shape.** Still open 4.
22. **Tap-to-top hooks.** Settled: resolutions.md D-12 (`LocalReselect` and `ReselectEffect` are NEW).

### Words and strings

23. **The `subtasks_done` plural example.** Settled: resolutions.md D-7 (plural keyed on the total).
24. **Strings to resources, wave 0 or 1.** Settled: resolutions.md D-8 (the move, the catalogue and the
    lint in wave 0; the rewrites in wave 1).
25. **A failure reason not in the catalogue.** Settled for pairing: resolutions.md A2 ("Pairing is off.
    Switch it on and try again."). Safe mode's two place reasons: Still open 5.
26. **FixAssistantHandoff's flag.** Settled: resolutions.md A2 keeps "HQ is fine. Try again soon." as the
    AI service's outcome; nothing to change.

### Links and gaps

27. **`hq://tasks/away`.** Settled: resolutions.md D-9 (in `Links.kt` and `LinksTest` with the page).
28. **"No helpers match that."** Settled: resolutions.md D-9.
29. **Server HQ notes in Search (N3).** Settled: resolutions.md D-9 (a frame is drawn when wave 4 starts).
30. **Verification briefing from the phone.** Settled: resolutions.md D-10 (owner-only, and the phone
    signs in as the owner; confirm in `crates/hq-web/src/verification_api.rs` before building and stop if
    it is limited further).

### Still open

All of these are now settled in `resolutions.md` section E (5 Nov 2026); build what section E says. The notes below explain how each was reached.

1. **The details switch's on line and the no-lock lines** (FixLockScreen against FixNotifications; D-1
   names FixPlainWords, which draws only the off line). Recommendation, now in both specs: on "Details
   show only while the phone is unlocked. Allow asks you to unlock first."; no lock, the row's line
   "Needs a screen lock" and the note "Set a screen lock to use this. Without one, anyone holding this
   phone can answer." with Open Android settings and "Then tap Screen lock."
2. **Withdrawn version words** (FixSafeMode against FixUpdates). Recommendation, now in both specs:
   FixSafeMode's, since safe mode owns the action: card "AgentHQ {version} has a problem" / "A fix is
   coming. Save a copy of your notes now, to be safe." / "Save a copy of my notes"; notice "AgentHQ
   {version} has a problem · Open HQ to see what to do."; found then pulled "Up to date · {version} was
   withdrawn. Nothing was installed."
3. **The Connection sheet's starting body and battery line** (FixStartup against FixOffline; D-5 places
   the UI but not these words). Recommendation, now in both specs: "HQ runs on this phone. After a reboot
   it can take up to a minute. You can read and write while you wait." and, under Open Android settings,
   "Then set Battery to Unrestricted."
4. **Data-extraction rules shape.** Recommendation, already in both specs: build FixDataTransfer's file
   (an include list of `hqhome/.vault` only), which leaves `savedcopy/`, the outbox and spoken ids out;
   FixOffline's check that they are absent on the new phone still applies.
5. **Safe mode's two save-copy reasons**, "That place is full." and "The place you chose stopped
   answering.", which A2 does not list. Recommendation: add them to the catalogue as reasons that do not
   retry, with the outcomes FixSafeMode already wrote ("Choose another place, or free up {size}." and
   "Choose another place and try again."); until then `wave-0/02-safe-mode.md` keeps them as written.
6. **The running notice's bodies.** D-2 settles the buttons only. Recommendation, applied in
   `wave-2/04-startup.md`: keep FixStartup's body per state ("Answering your chats and running scheduled
   tasks.", "Step {n} of 4 · {step}", "A setting changed. It is back in a few seconds.", "Tap to see why.").
7. **"See all tasks as" on Tasks › All.** Not a search list, so B6 does not name it. Recommendation,
   applied in `wave-4/03-web-pages.md`: "Show all tasks as", so the app has one "Show all" wording.
