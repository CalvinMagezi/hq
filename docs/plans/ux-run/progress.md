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

## Found by the owner

| Date | Where | What | Fixed in |
|---|---|---|---|
| 2026-11-05 | web/wave-0/01-live-bugs.md check 10 (agent note) | "Your server HQ needs your OK" cannot be told apart on a web push: the HQ sending the push is the HQ asking, and no inbox item marks itself as from a server HQ. It is a phone notification (the phone talking to a server HQ). Left out of the web push until the owner says otherwise. | open |
