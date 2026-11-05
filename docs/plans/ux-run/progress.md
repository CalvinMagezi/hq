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
| B11 (resolutions.md) | Approved as recommended: "This window", saved copy and outbox keyed by HQ id, "Saved in HQ Hub at 14:32", no web push, HQ Hub system notices | 2026-11-05 |

## Web wave 0 (live bugs and shared parts)

- [x] web/wave-0/01-live-bugs.md: web push content and buttons. Try: with push on, have a helper ask; the lock screen shows "{Helper} needs your OK" / "Open HQ to see what it wants to do." with no buttons and no command. Settings test push reads "HQ" / "Notifications from HQ are on in this browser." A finished reply alert reads "HQ answered" / "Tap to read." Not built: the "Your server HQ needs your OK" title (see the note under Found by the owner).
- [x] web/wave-0/01-live-bugs.md: the hub proxy and a taken port. Try: on the Mac, run `hq bridge serve` in a terminal so it holds the web forward, then turn pairing on in HQ Hub. The Host tab shows "Couldn't open HQ on your phone" with "Port in use" and Open HQ is off. Stop the terminal one, press Look again, Open HQ works. The tag is grey for now: the amber token arrives with the hub redesign (wave 3). The proxy also checks the forward on every request, so a port taken after the window opened gets nothing.
- [x] web/wave-0/01-live-bugs.md: wipe the browser on a refused sign-in. Try: sign a laptop browser out from another device's Settings › Devices, then use it: every place shows "Sign in to see your notes" (or chats, tasks, settings, what needs you) and nothing from HQ. DevTools › Application: no `hq-query-cache`, `hq-store`, `hq-thread-store`, `hq-web-token` or `hq-api` cache. A second tab of the same HQ empties too. Reload offline: still nothing.
- [x] web/wave-0/01-live-bugs.md: guard drive, goal and answer routes. Try: nothing to see; a current web app and the phone app keep working (both send the header). A cross-site POST to `/api/harness-sessions/{id}/drive` now gets 403.
- [ ] web/wave-0/02-shared-parts.md
  - [x] The one string catalogue (N19): `apps/strings/en.yaml`, `scripts/strings/build.ts`, the inline-string ratchet. Try: nothing to see yet.
  - [x] The saved copy keyed by HQ id, capped at 256 KB a place and 2 MB in all, kept until replaced; the wipe and the worker's cache follow the id. Try: in HQ Hub open this computer's HQ, then the phone's: each shows only its own saved notes (DevTools › Application shows `hq-store:{id}` per HQ).
  - [x] The outbox (Waiting to send, client ids, the 100 cap, 24 h for messages) and the What HQ noticed counters, as modules; wired into Chat once the Connection state lands. Built by Copilot (Sonnet 5.5), reviewed; the drain was rewritten so a message added mid-send is never lost.
  - [x] HQ drops a repeated `client_id` for chat messages (socket), new notes and new tasks, and answers with what it already made; the web's chat ids are now random. Try: nothing to see until the outbox is wired; sending a chat message works as before.
  - [x] The Connection state, the status line and the connection sheet (seven states, the retry schedule, "HQ is back. Lists are up to date."); the banner is gone. First draft by agy, reviewed; a double poll, a first-paint "can't be reached" and the sheet's layout fixed. Try: open HQ, choose the status line top right: the sheet says Connected. Run `hq web stop`: within about 8 s it reads "Your HQ can't be reached" with the next try counting down. Start HQ: "HQ is back. Lists are up to date."
  - [x] `GET /api/status/ai` (signed in): per AI service the last good and failed call and the failure kind, from the calls HQ already records; the status line says "The AI service isn't answering" only when every service HQ used in the last 15 minutes fails. HQ Hub's proxy marks its own 502 so the window knows its HQ is gone.
  - [x] "This is a shared computer" on the sign-in prompt: the sign-in lasts for this tab only, nothing from HQ is saved (query cache, stores, drafts, the worker's API copy), push is not offered, and the tab signs out after 30 minutes without input. The export warning comes with the sign-in redesign (wave 4). Try: in a private window, sign in with a code and tick it first; close the tab and open HQ again: it asks to sign in.
  - [x] `GET /api/hello` and the starting flag. Try: `hq web stop`, then start HQ and open `/api/hello` in a browser quickly: it shows the start step, then `"starting": null`.

## Phone wave 0 (safety and shared parts)

- [x] Test setup (resolutions.md D-11): Compose UI tests and a screenshot test under Robolectric (`app/src/test/.../testing/Screenshots.kt`, goldens in `src/test/resources/screenshots`, `-Phq.recordScreenshots=true` to re-record), and the `:lint-checks` module with `HqInlineText` (words in a composable come from the catalogue). The 137 inline strings found today are in `app/lint-baseline.xml`; new ones fail the build. Try: nothing on the phone.

- [x] Server for phone wave 0: `hq vault moved` (forgets a moved phone's device sign-ins, pairings, edge, claude.ai grants and push subscriptions; notes stay) and `GET /api/status/busy` (a chat reply or a scheduled backup running), for the update idle gate.
- [ ] mobile/wave-0/01-lock-screen.md
- [ ] mobile/wave-0/02-safe-mode.md
- [ ] mobile/wave-0/03-data-transfer.md
- [ ] mobile/wave-0/04-measure.md

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

## Found by the owner

| Date | Where | What | Fixed in |
|---|---|---|---|
| 2026-11-05 | web/wave-0/01-live-bugs.md check 10 (agent note) | "Your server HQ needs your OK" cannot be told apart on a web push: the HQ sending the push is the HQ asking, and no inbox item marks itself as from a server HQ. It is a phone notification (the phone talking to a server HQ). Left out of the web push until the owner says otherwise. | open |
