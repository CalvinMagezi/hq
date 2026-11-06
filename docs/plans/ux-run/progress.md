# Progress

The building agent keeps this file current: tick each spec when it is built and its gates pass, write
the wave's "Try this now" note, and record what the owner found. One line per item.

## Owner decisions to collect

From `decisions.md` "Owner to confirm": N4, N6, N19, N22, N25, W34, W44, W50, WR18, WR22.
Ask for them in one batch before the wave that needs each. Record the answer and the date here.

| Id | Answer | Date |
|---|---|---|
| N4 | Yes: transfer carries the vault; keys, config, bridge key, run mode and preferences stay behind, then the moved-phone flow | 2026-11-05 |
| N6 | Yes: the safe-mode copy is a plain .zip with no password, for now | 2026-11-05 |
| N19 | Yes: one catalogue of whole sentences builds the phone's resources and the web's strings | 2026-11-05 |
| W34 | The claude.ai web connector is retired: remove its row and text from the web entirely (owner, overrides the spec's "keep, off") | 2026-11-05 |
| N25 | Yes: Search before the balance chip on the phone, as on the web | 2026-11-05 |
| N22 | Yes: the screen stays on 30 s while HQ is thinking, then sleeps | 2026-11-05 |
| W44 | "This computer" (not "This PC") for HQ Hub's first tab | 2026-11-05 |
| WR22 | Yes: rename the bundle to AgentHQ, rewrite only HQ Hub's own Claude entries, say so once | 2026-11-05 |
| W50 | After this run: no Windows or Linux hub builds now | 2026-11-05 |
| WR18 | Yes: "Typing to the helper", off by default, Ctrl ] always leaves | 2026-11-05 |
| Cutover | PR to main when the gates pass, then the CI release on the A32 and install-hq.sh on the laptop | 2026-11-05 |
| L1 | Yes: the losing tools and commands as listed below | 2026-11-05 |
| L2 | Yes: a session's command policy keeps saying yes to a losing command its own rules allow | 2026-11-05 |
| L3 | Yes: a command dialog cut too short to read counts as losing for automatic answers | 2026-11-05 |
| Locked hold log | Skipped for now: the A32's lock screen has no Home button to hold. Wave 3 ships on N21's path: the unlocked assistant, the lock-screen switch off (signed channel `lockScreenAssistant` false), card B in Settings. A later Pass can turn the switch on without a build | 2026-11-05 |
| B11 (resolutions.md) | Approved as recommended: "This window", saved copy and outbox keyed by HQ id, "Saved in HQ Hub at 14:32", no web push, HQ Hub system notices | 2026-11-05 |

## Web wave 0 (live bugs and shared parts)

- [x] web/wave-0/01-live-bugs.md: web push content and buttons. Try: with push on, have a helper ask; the lock screen shows "{Helper} needs your OK" / "Open HQ to see what it wants to do." with no buttons and no command. Settings test push reads "HQ" / "Notifications from HQ are on in this browser." A finished reply alert reads "HQ answered" / "Tap to read." Not built: the "Your server HQ needs your OK" title (see the note under Found by the owner).
- [x] web/wave-0/01-live-bugs.md: the hub proxy and a taken port. Try: on the Mac, run `hq bridge serve` in a terminal so it holds the web forward, then turn pairing on in HQ Hub. The Host tab shows "Couldn't open HQ on your phone" with "Port in use" and Open HQ is off. Stop the terminal one, press Look again, Open HQ works. The tag is grey for now: the amber token arrives with the hub redesign (wave 3). The proxy also checks the forward on every request, so a port taken after the window opened gets nothing.
- [x] web/wave-0/01-live-bugs.md: wipe the browser on a refused sign-in. Try: sign a laptop browser out from another device's Settings › Devices, then use it: every place shows "Sign in to see your notes" (or chats, tasks, settings, what needs you) and nothing from HQ. DevTools › Application: no `hq-query-cache`, `hq-store`, `hq-thread-store`, `hq-web-token` or `hq-api` cache. A second tab of the same HQ empties too. Reload offline: still nothing.
- [x] web/wave-0/01-live-bugs.md: guard drive, goal and answer routes. Try: nothing to see; a current web app and the phone app keep working (both send the header). A cross-site POST to `/api/harness-sessions/{id}/drive` now gets 403.
- [x] web/wave-0/02-shared-parts.md (the parts below; what is left of it is listed as open)
  - Open from this spec, each with where it lands: the saved strip with each place's own time (`.a-stale`, drawn with the shell in web wave 1); the wipe on "Sign out this browser" and Sign out everywhere (web wave 1, sign-in safety; the 401 wipe is built); new notes and tasks through the outbox (web wave 4, Notes and Tasks; chat is wired); the What HQ noticed page on the web (web wave 1, Settings); `navigator.storage.persist()` after sign-in (web wave 4, sign-in); N2, the saved-copy size on a 2,000-note vault (to measure on the A32 at the wave 0 check).
  - [x] The one string catalogue (N19): `apps/strings/en.yaml`, `scripts/strings/build.ts`, the inline-string ratchet. Try: nothing to see yet.
  - [x] The saved copy keyed by HQ id, capped at 256 KB a place and 2 MB in all, kept until replaced; the wipe and the worker's cache follow the id. Try: in HQ Hub open this computer's HQ, then the phone's: each shows only its own saved notes (DevTools › Application shows `hq-store:{id}` per HQ).
  - [x] The outbox (Waiting to send, client ids, the 100 cap, 24 h for messages) and the What HQ noticed counters, as modules; wired into Chat once the Connection state lands. Built by Copilot (Sonnet 5.5), reviewed; the drain was rewritten so a message added mid-send is never lost.
  - [x] HQ drops a repeated `client_id` for chat messages (socket), new notes and new tasks, and answers with what it already made; the web's chat ids are now random. Try: nothing to see until the outbox is wired; sending a chat message works as before.
  - [x] The Connection state, the status line and the connection sheet (seven states, the retry schedule, "HQ is back. Lists are up to date."); the banner is gone. First draft by agy, reviewed; a double poll, a first-paint "can't be reached" and the sheet's layout fixed. Try: open HQ, choose the status line top right: the sheet says Connected. Run `hq web stop`: within about 8 s it reads "Your HQ can't be reached" with the next try counting down. Start HQ: "HQ is back. Lists are up to date."
  - [x] `GET /api/status/ai` (signed in): per AI service the last good and failed call and the failure kind, from the calls HQ already records; the status line says "The AI service isn't answering" only when every service HQ used in the last 15 minutes fails. HQ Hub's proxy marks its own 502 so the window knows its HQ is gone.
  - [x] "This is a shared computer" on the sign-in prompt: the sign-in lasts for this tab only, nothing from HQ is saved (query cache, stores, drafts, the worker's API copy), push is not offered, and the tab signs out after 30 minutes without input. The export warning comes with the sign-in redesign (wave 4). Try: in a private window, sign in with a code and tick it first; close the tab and open HQ again: it asks to sign in.
  - [x] Chat's outbox wired: a message sent while HQ is out of reach shows under "Waiting to send" and goes out once the socket is back, one at a time, waiting for HQ's answer by client id; a resend after a lost answer is dropped by HQ. Checked in a browser against a running HQ (stopped, sent, started: one copy on HQ, and again after a reload). Try: `hq web stop` with a chat open, send a message, start HQ: it appears once.
  - [x] `GET /api/hello` and the starting flag. Try: `hq web stop`, then start HQ and open `/api/hello` in a browser quickly: it shows the start step, then `"starting": null`.

## Phone wave 0 (safety and shared parts)

- [x] Strings moved to the catalogue (N20, D-8): every inline string in the phone's composables now comes from `apps/strings/en.yaml`, words unchanged (the rewrites are wave 1); the lint baseline is gone, so any new inline string fails the build. Keys are mechanical for now and get real names with the wave 1 rewrite. Try: nothing changes on screen.
- [x] Test setup (resolutions.md D-11): Compose UI tests and a screenshot test under Robolectric (`app/src/test/.../testing/Screenshots.kt`, goldens in `src/test/resources/screenshots`, `-Phq.recordScreenshots=true` to re-record), and the `:lint-checks` module with `HqInlineText` (words in a composable come from the catalogue). The 137 inline strings found today are in `app/lint-baseline.xml`; new ones fail the build. Try: nothing on the phone.

- [x] Server for phone wave 0: `hq vault moved` (forgets a moved phone's device sign-ins, pairings, edge, claude.ai grants and push subscriptions; notes stay) and `GET /api/status/busy` (a chat reply or a scheduled backup running), for the update idle gate.
- [x] mobile/wave-0/01-lock-screen.md: notifications say only "{Helper} needs your OK" and the like by default, with Open alone; "Show details in notifications" (off; needs a screen lock) shows the request only while unlocked, with Decline · Allow once · Open on Android 12+ (Allow asks for the unlock) and never for a losing request; at lock every notice goes general, at unlock details come back; a public version on every notice; the five channels made once; no server address in any notice. First draft by Copilot (Sonnet 5.5), reviewed. Open: a losing request is spotted by its words until HQ flags it (N17, wave 1); the Inbox note for upgraders. Try: turn the switch on, lock the phone with a helper asking: the lock screen says "Claude needs your OK" with Open only; unlock: the request and the three buttons appear.
- [x] mobile/wave-0/02-safe-mode.md: when HQ cannot start (two crashes in a row, or from the "Save notes" shortcut) the app opens a plain screen in its own process to save the notes as a .zip (N6, no password) through the system picker; Restore takes that .zip with no secret (names that climb out of the folder and copies that would not fit are refused), then forgets the old phone's sign-ins; an update installs only when HQ is idle (a chat reply, a backup, notes waiting to send, the speech model downloading), with Wait, Stop it and install, or Not now; the channel file's new fields parse with safe defaults, and withdrawn versions are refused. First draft by Copilot (Sonnet 5.5), reviewed: an unpacked-size cap added, Settings and Restore wired. Try: long-press the app icon › Save notes: save the copy to Downloads; on a test install, Restore it.
- [x] mobile/wave-0/03-data-transfer.md: data-extraction rules (only the vault moves to a new phone, cloud backup takes nothing), an install marker, the Moved first-run step (keys added again, Start fresh, notes only when the database copied badly) that runs `hq vault moved`, dead keys hidden, the old-phone card. First draft by Copilot (Sonnet 5.5), reviewed. Open: the .zip restore path comes with safe mode. Try: on a test install, copy `files/hqhome/.vault` from another install with adb and open the app: it says "Your notes came with you" with the counts and asks for keys again.
- [x] The phone's Connection model and words (resolutions.md D-5): `data/Connection.kt` with the first-wins order, stopped judged by progress (A7), and the A2 reason and outcome sentences from the catalogue. No UI yet: the sheet is wave 2, the shell moves onto it in wave 4.
- [x] mobile/wave-0/04-measure.md: "What HQ noticed" on the phone: counts only (grants, locked presses, notices posted and tapped, bytes by purpose and metered or not), kept in `noBackupFilesDir/counters.json` (8 KB cap, this version and the one before), never sent; Settings › Technical details › What HQ noticed, and Copy details carries the block. First draft by Copilot (Sonnet 5.5), reviewed. Call sites whose code comes later (the Connection sheet, Undo, NotifAccess) count from then. Try: Settings › Backup and about › Technical details › What HQ noticed.

## Wave 1 to wave 4

Add one line per spec file as the wave starts (see `plan.md` for the order).

### Owner to confirm (raised in wave 1)

These are rules about what HQ may do on its own, so they wait for a yes. What is built until then is
the safer side.

| Id | Question | Built until answered |
|---|---|---|
| L1 (answered yes) | Which calls count as losing (red "Allow {verb}", never an automatic yes). Built: a tool or action named delete, remove, stop, revoke, disconnect, drop, overwrite, logout, trash, purge, uninstall or kill; for `bash` and helper command dialogs `rm`, `rmdir`, `unlink`, `shred`, `git rm`, `git clean`, `git branch -d/-D`, `git push --force/-f/--force-with-lease/--mirror/--delete`, `git reset --hard`, `git checkout --`, `git restore`, `git stash drop/clear`, `kill`, `pkill`, `killall`, `dropdb`, `gh repo delete`, `gh auth logout` (`crates/hq-core/src/losing.rs`, `command_policy.rs` `LOSING_COMMANDS`). Add or drop any? | as listed |
| L2 (answered yes) | Should a session's command policy (the owner's own allow rules) still say yes by itself to a losing command it allows? The built-in builder and reviewer policies allow none. | yes, the policy keeps deciding |
| L3 (answered yes) | A command dialog cut too short to read whole is treated as losing for Drive and other automatic answers (they may decline it, not say yes). Keep? | yes |

### Web wave 1

- [x] 01-shell-status.md: header with the place's title and the status line, the saved strip per place, state pills for helpers, one amber Inbox badge, colours by the rules (A3). Copilot (Sonnet 5.5), reviewed.
- [x] 06-settings-groups.md: the phone's four groups, This browser (Needs you, Updates, Show details), one card slot, `/settings` opens This browser › Notifications, "This window" inside HQ Hub (its proxy now sends `x-hq-window: hub`). The claude.ai connector row and every line about it are gone (W34). Server: `GET /api/backup/status`.
- [x] 02-dialogs-approvals.md: one red `<dialog>` for every confirm, approval cards that quote the tool and its arguments, red "Allow {verb}" for a losing request. Server: `owner_approvals`, `metadata.approval`, `loses` and `verb` on dialogs, no automatic yes to a losing dialog (N17, W15; see L1 to L3).
- [x] 03-undo.md: Undo toasts for archive, clear and done, ten minutes on HQ; Clear all updates never touches Needs you (server fixed: it used to dismiss approvals too); Undo never brings back a declined approval.
- [x] 04-plain-words.md: Helpers, Inbox with Needs you, Updates and Done, Messaging apps, Let HQ say yes to its requests, whole relative times.
- [x] 05-sign-in-safety.md: names from the user agent, 90 days unused then signed out unless kept (an expired row stays, so sign-in never turns off), a Needs you item per new sign-in with It was me and Sign it out, This is a shared computer, Sign out this browser, WEB_AUTH.md.

### Phone wave 1

- [x] 03-destructive.md: red dialogs through `ui/shell/Confirm.kt`, Undo through `ui/shell/Undo.kt`, server restore routes for chats, updates and tasks (ten minutes, 409 "changed" after).
- [x] 01-notifications.md: the first-run Notifications step, the upgrader sheet once, Turn on in three places, folding, the replay when notifications come back (C-11), "Messaging apps" (B15).
- [x] 02-backup.md and 05-progress.md: the backup row and Home card from facts, Save a copy with its states, `GET /api/backup/status`, progress counts and 99+; the Inbox's red Allow {verb} now comes from HQ (`metadata.approval`), never the card's words.
- [x] 04-plain-words.md: every owner sentence through the catalogue, real plurals, 12 or 24-hour time by the phone, `Instants.parse` for every time HQ sends.

### Web wave 2

- [x] 01-route-table.md: one route table (`/chat`, `/notes`, `/tasks`, `/inbox`, `/helpers`, `/settings`), every stored link and QR code redirects with its id, the manifest and shortcuts. Server: HQ writes the new link forms (`/helpers/{id}`, `/inbox`, `/chat/{id}`, `/tasks/{id}`).
- [x] 02-embed-and-lite.md: `?embed=1` beside `?host=android`, the Lite shell by the edition.
- [x] 03-version-and-data.md: "A newer HQ is running" with drafts kept, change events over the socket (server: `inbox:change`, `notes:change`, `helpers:change`, `budget:change`, `settings:change`), nothing runs while hidden, Data used. Chat first load about 303 kB gzip (baseline).
- [x] 04-time-zones.md: HQ's zone in `/api/hello`, every hq-db time sent as RFC 3339 with Z, the display zone per browser.

### Phone wave 2

- [x] 01-downloads.md and 02-updates.md: the download engine (paid data and paid Wi-Fi asks, resume, hash check, Undo), the updater on it, installed only when idle, staged and withdrawn versions, the Update page. The releases repo comes from `-Phq.releasesRepo` (no owner name in the source).
- [x] 03-assistant-model.md and 04-startup.md: the speech model on the download engine, the Connection sheet's screen in `ui/connection/`.

### HQ Hub (web wave 3)

- [x] 05-commands-and-platforms.md and 01-this-pc.md: the new commands (allow-listed `open_main_window(path)`, `host_log` with keys hidden, `pick_program` refusing shared folders, local `host_unlock`), a native chooser and notice with no plugin, AgentHQ and HQ Hub names, the Claude Desktop entry rewrite after the rename (WR22), This computer.
- [x] 02-devices.md, 03-helpers-tab.md, 04-helper-detail.md: Devices, the Helpers tab (versions from package files, sandboxed checks, two at a time), Try it, Connect and Remove with Undo, sign-in links only on the helper's own site. The sign-in sites per helper are the vendors' own domains, chosen by the worker: please glance at them in `crates/hq-desktop/src/helpers.rs`.
- Not checked in a running app: the hub needs a `tauri build`, done once at the end for the cutover check.

### Phone wave 3

- [x] 01-assistant-setup.md, 03-assistant-mic.md, 04-assistant-handoff.md: the Phone assistant row and page, the first-run card, the mic asks, Stop, 30 s screen on while thinking (N22), the voice hand-off (notices, Waiting for HQ, resume after a dropped socket, You said and Keep going on approvals, the spoken mark).
- [x] 02-assistant-locked.md on N21's path (no locked hold log, owner 2026-11-05): no HQ code runs at a locked press, the lock-screen switch stays off, Settings says why. The Pass path is not built.

### Web wave 4

- [x] 01-chat.md: start screen, Get started, Stop over HTTP, resume, banners by state, the Enter rule, grouped sidebar links, the spoken mark, approval cards in the chat, Let HQ say yes with its explainer and the list of what HQ said yes to (server: `GET /api/harness-sessions/{id}/answers`, approvals name their chat).
- [x] 02-first-run-sign-in.md and 03-notes.md: covers, `/signin`, notes that never overwrite silently (409, See their version, Keep both), Quick note, Export, About this note and Looks right (server: `GET /api/note/versions`, `POST /api/note/review`), the leave dialog. Copy link stays Copy path (no note ids).
- [x] 04-search.md and 05-inbox.md: one Search (Ctrl K, folder filter from HQ, Back reopens it), the Inbox redesign, Done by day.
- [x] 06-tasks.md and 07-helpers.md: Today and Upcoming, checking in plain words, Board and Timeline, HQ's own work, Start a helper (WR17 done 2026-11-06: "Start a helper on this task" from a task page, `task_id` on the start route), typing to a helper (WR18), the helper page.
- [x] 08-settings.md and 09-spending.md: Settings in plain words, the key sheet, Support and Updates, Spending limits by card with origins HQ records (chat, harness, background, verification).

### Phone wave 4

- [x] 01-offline.md, 02-search.md, 03-web-pages.md: the Connection pattern in every place, outbox sends HQ never runs twice, note conflicts, one search with helpers and folders, web pages reduced to Usage, Board and Timeline with `embed=1`, While you were away native.

### Closing passes

- Every quoted sentence in every spec's Exact copy section is in the catalogue (checked by script). Strings no screen used were wired in or, where they duplicated others or named Windows (W50), deleted.
- The web carries only its share of the catalogue: Chat first load about 301 kB gzipped (from 322), the worker 31 kB.
- Reviews: the approval and sign-in seams (fail-open losing check fixed), then the hub and later server work (Try it folder could empty any folder: fixed with an ownership marker; the rename retry; inbox change events; Lite search through a link).
- Workspace: clippy clean, 5574 passed, 0 failed, 44 ignored (94 binaries). Web 685, hub UI 20, Android green, HQ Hub shell compiles.

### Server parts for wave 4 (built ahead)

- `/api/search?folder=` filtered before the limit; notes save with `base` and 409; `source: voice` and `client_id` in a message's meta, `turn_end.waiting_for_owner`, "You said" on a voice turn's approvals; `POST /api/threads/{id}/stop` and `resume`. The task field on the start route (WR17) landed after the cutover.

Workspace gates after these: clippy clean; `cargo test --workspace` 5512 passed, 0 failed, 44 ignored (94 binaries).

## Try this now

### Web wave 0, the four live bugs (2026-11-05)

Install on the laptop, from `~/Documents/GitHub/hq-ux-run` (branch `ccr-66a72c4c-52kq29`):

```bash
cd apps/hq-web && bun install && bun run build && cd ../..
./scripts/install-hq.sh            # builds, installs, restarts the daemon, verifies
./scripts/install-hq.sh --check    # nothing stale
hq web --json                      # open the login_url it prints
```

For the hub: `cd apps/desktop && bun install && bun run build:ui && bun run tauri dev`.

1. Push: in Settings turn push on, press Test. You see "HQ" / "Notifications from HQ are on in this browser." Start a helper that asks for approval and lock the screen: "{Helper} needs your OK" / "Open HQ to see what it wants to do.", one tap target, no Allow or Decline, no command anywhere in the notification history.
2. Reply alert: switch reply alerts on, send a chat, switch tabs. The alert reads "HQ answered" / "Tap to read."
3. Signed out: open HQ in a second browser profile and sign in with a code. From the first browser, Settings › Devices, sign the second one out. In the second browser every place reads "Sign in to see your notes" (or chats, tasks, settings, what needs you) and nothing else. DevTools › Application: no `hq-query-cache`, `hq-store`, `hq-thread-store`, `hq-web-token`, no `hq-api` cache. Offline reload: still nothing.
4. Hub proxy (phone as hub): run `hq bridge serve` in a terminal first, then turn pairing on in HQ Hub. The Host tab shows "Couldn't open HQ on your phone", "Port in use", and Open HQ is off. Stop the terminal one, press Look again: Open HQ works.
5. Guarded routes: nothing to see. The web app and the phone app keep approving and driving sessions as before.

Not covered yet: "Your server HQ needs your OK" (see Found by the owner), and the amber colour on "Port in use" (the hub's tokens arrive in wave 3).

Gates at hand-over: workspace clippy clean; `cargo test --workspace` 5432 passed, 0 failed, 44 ignored (94 binaries), then hq-web and hq-desktop re-run green after the review fixes; web 364 tests and the build pass; desktop typecheck passes. Android not built: nothing in these four changes touches the app.

### Wave 0, shared parts and the phone (2026-11-05)

Laptop (from `~/Documents/GitHub/hq-ux-run`, branch `ccr-66a72c4c-52kq29`), as for the live bugs above:
`cd apps/hq-web && bun install && bun run build && cd ../.. && ./scripts/install-hq.sh`, then `hq web --json`.
After any rebuild of the web app, restart HQ: it reads the shell's script hashes once at start.

1. Status line: top right reads "Connected". Choose it: a side sheet says "Nothing to fix." and the address.
2. `hq web stop`: within about 8 s "Your HQ can't be reached", with the last answer and the next try
   counting down. `hq web --json`: "HQ is back. Lists are up to date."
3. Starting: stop HQ, start it, and open `/api/hello` quickly: the start step, then `"starting": null`.
4. Waiting to send: open a chat, stop HQ, send "test": it shows under "Waiting to send". Start HQ: it is
   sent once (reload the page: still one copy).
5. Shared computer: in a private window, tick "This is a shared computer" before signing in with a code.
   Close the tab and open HQ again: it asks to sign in. Push is not offered.
6. Two HQs: in HQ Hub open this computer's HQ, then the phone's: each shows only its own saved notes.

Phone (needs the phone plugged in, and a test build: `apps/android/README.md`, with `-Phq.appId` set to a
test id so it installs beside the real app):

7. Lock screen: Settings › Notifications. Details are off. Have a helper ask while the phone is locked:
   "Claude needs your OK", Open only. Turn details on (it asks for a screen lock), unlock: the request with
   Decline · Allow once · Open; Allow once asks for the unlock.
8. Safe mode: long-press the app icon › "Save notes": save the copy to Downloads (about the vault's size).
9. Moving phones: on a second test install, Restore that .zip (no passphrase): "Your notes came with you",
   keys asked again, Other devices empty.
10. Updates: with a reply running, Settings › Backup and about › Install update: "Install now?" with Wait
    until it's done, Stop it and install, Not now.

Gates at hand-over: workspace clippy clean; `cargo test --workspace` 5444 passed, 0 failed, 44 ignored (94 binaries); web 401 tests, typecheck and build; Android assembleDebug, 606 unit tests, lint with no issues and no baseline.

Device checks still to log in `mobile/device-checks.md`: wave 0 rows (lock-screen swap on a real lock,
Smart Switch or Google copy, a 4 GB zip on 8.1).

### Web wave 1 (2026-11-05)

Build the web app and restart HQ after it (the page's script hashes are read when HQ starts):
`cd apps/hq-web && bun run build && cd ../.. && ./scripts/install-hq.sh`, then `hq web --json`.

1. Settings: `/settings` opens This browser › Notifications. The list has the phone's four groups and no Claude row. Inside HQ Hub the group reads This window.
2. A losing request: ask the chat to delete a task. The Inbox card quotes `task_delete` and its arguments, then In HQ's words, with Decline and a red Allow delete. Ask it to list files instead: the button is green Allow once.
3. Drive never says yes to a losing dialog: let a helper ask to run `rm -rf build` with Drive on. It stays waiting for you; its Watching panel says only the owner may say yes.
4. Undo: archive a chat, then Undo in the toast; clear one update, then Undo. After ten minutes Undo says it changed or too much time passed.
5. Clear all updates leaves every Needs you item in place.
6. A new sign-in: in a second browser profile sign in with a code. The first browser's Inbox shows A new sign-in to your HQ with It was me and Sign it out. Sign it out asks once in a red dialog; the second browser then sees only Sign in.
7. Other devices: the code card says "Type this only into HQ on your own device." Rows are named like "Firefox on Windows". A row's menu has Keep signed in.
8. Shared computer: tick This is a shared computer on the sign-in sheet. Close the tab and open HQ again: signed out, nothing stored.

Checked in a browser against a test HQ (`target/debug/hq` from this branch, a throwaway vault and token): items 1, 2 (card and words), 6 and 7, and the sign-in sheet's words. That check found and fixed: the red dialog drawn off screen, updates listed under Needs you, a notice offered as an approval, the Notifications row saying on while push was off, "used not used yet". Not checked in a browser: 3 (needs a real helper), 4 past ten minutes, 8.

Gates: web 521 tests, typecheck and build pass; hq-core 296, hq-db, hq-tools and hq-web suites pass; clippy clean on the touched crates. The review of the approval and sign-in seams found the losing check failing open on shell keywords, wrapper options, `xargs`, `find -delete` and hidden code; fixed and tested (commit d24ca547).

## After the cutover (2026-11-06)

- [x] WR17: `POST /api/harness-sessions` takes `task_id` (refused before a folder is made when the task is missing, archived or complete); the task page has "Start a helper on this task", and the dialog says "For this task: {title}".
- [x] Note ids (W22, C-10): Copy link mints `note_id:` as the note's first frontmatter line (`POST /api/note/link`, not recorded as the owner's version, answers the new `hash`), `GET /api/note/by-id/{id}` finds it after a move, the web's `/notes/n/:id` opens it or shows the gone page, and the phone opens `hq://notes/n/{id}` the same way. Lite answers the same 404 for a note it keeps out.

Try this now: open a note, Copy link, move the note to another folder in Finder, paste the link: the note opens from its new folder. On a task page, Start a helper on this task: the helper's task records its start.

## Found by the owner

| Date | Where | What | Fixed in |
|---|---|---|---|
| 2026-11-05 | web/wave-0/01-live-bugs.md check 10 (agent note) | "Your server HQ needs your OK" cannot be told apart on a web push: the HQ sending the push is the HQ asking, and no inbox item marks itself as from a server HQ. It is a phone notification (the phone talking to a server HQ). Left out of the web push until the owner says otherwise. | open |
