# Wave 4 · 03 · Web pages in the app: retire Home, bring While you were away native (finding #6)

Board: **FixWebPages** ("Fix 6: web pages"), finding 6, designed, not built, revised after review,
5 Nov 2026. Area: `ui/webhost`, `ui/tasks`, `ui/shell/Links.kt`. Shared rules: `../../rules-shared.md`
sections 2, 3, 6, 7, 8 win.

Headline: **Retire the web Home, keep its one useful part, and bring Board and Timeline up where they are
seen.**

## 1. Finding and the review items it closes

**The problem.** The web Home page (`/vault`) opens only through `hq://web/home`, so no one sees it; Board
and Timeline sit after the last row of a long All list; and each web page fails in words the rest of the
app does not use.

**Who it hurts, how often.** Every owner who lets HQ check and finish tasks. The summary of what HQ
finished while they were away exists on the phone only inside the page nobody can open. Board and Timeline
are found only by people who scroll to the end of All.

What changed in this revision:

- **Review #6, "sent back and fixed" was false.** The briefing counts tasks that failed a check and were
  moved back to `to_do` or `blocked` (`crates/hq-db/src/tasks/briefing.rs:312-321`). They were reopened,
  not fixed. The line now reads "{N} went back to you to finish", or names the helper when one holds the
  task.
- **"Unless you stop it" is gone** (D18, stopping from the phone, is not built). A task in its window
  reads "Finishes at 18:00", and the page says holds are on the web app.
- **Renamed:** the "Needs you" section is "Waiting on you", as on Today. "Sweeps and audits" is "Spot
  checks".
- **`/vault` was still reachable** from inside Board, because `WebHost.allows()` lets any path on the
  local port load (`WebHost.kt:35-38`). It now refuses `/vault` and opens Notes.
- **New frames:** the server HQ variant of While you were away (A7, A8), the Today card at 360 dp and font
  scale 2.0 (A9), and an Android versions card. Shared rules applied: "Connected" in grey, "HQ" in
  capitals, no red status, failure lines from rules section 2.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/kotlin/example/hq/spike/`.

- `ui/webhost/WebHost.kt:10` `Home("home", "Home", "/vault")`; `:34-37` `allows()` accepts any `http`
  URL on `127.0.0.1` and the hub port, `/vault` included.
- `ui/shell/Links.kt:54-55` `webPage()` maps `"vault"` (and `"home"`) to `WebPage.Home`.
- `ui/tasks/TasksScreen.kt:156` `WebViews()` draws Board and Timeline after the last row of All.
- HQ: `crates/hq-web/src/lib.rs:460` `/api/verification/briefing`; `crates/hq-db/src/tasks/briefing.rs:312-321`
  counts reworked tasks (verdicts whose task moved from `ready_for_review` to `to_do` or `blocked`);
  `crates/hq-web/src/auth.rs:386` `web_auth_middleware`.
- `ServerHq.kt:125` `call(address, path, token, body, etag, …)`.

### Before frames

- **B1 · Tasks All, scrolled to the very end.** "Tasks · 9 active · Live · Copilot 912 left"; "To do 4":
  T-6, T-7, T-8 "for Alex", T-9 "HQ edge worker"; then "Board", "Timeline" (pin A); [New task].
- **B2 · Home (/vault), only from hq://web/home** (pin B). "Home", "Search vault...", "While you were
  away": "Verified and completed: Phone app, 3 tasks · Backups, 1 task", "Failed and reworked: 1, without
  you", "Tier 2, inside the veto window: T-7 Rotate the backup key" (pin C); "Pinned: Mobile-First-Roadmap
  · Projects"; "Recent Project Notes: Backup-Setup-2026-10-20 · Projects › Agent-HQ"; "In Progress Tasks:
  Workstream C: a native view instead of the WebView"; "System Activity".
- **B3 · A web page when HQ does not answer** (pin D). "Board" / "Couldn't load Board" / "HQ did not
  answer. Check that it is running, then try again." / [Try again].

### The four web pages: what each offers, and what the phone already has

| Page | What it shows | Already native | Verdict |
|------|---------------|----------------|---------|
| Home · `/vault` | Search vault, New note, Get started, While you were away (what HQ checked and finished, what failed a check and was reopened, what is inside its finishing window), Pinned, Recent project notes, In progress tasks, System activity | Search, New note, Pinned and Recent on Notes; in progress on Tasks Today; first run replaces Get started | **Retire.** Bring While you were away to Tasks Today as a native card and page. System activity stays on the web app in a browser. |
| Usage · `/usage` | Budgets, a forecast for this month, the biggest models and kinds of work over 7 days, a section per AI service | Budgets only, on Spending limits | **Keep as a web page,** reached from Spending limits as today. It gains the web page line and the shared HQ states. |
| Board · `/tasks?view=board` | All tasks in columns by status | Nothing like it | **Keep;** move the way in to the top of All. |
| Timeline · `/tasks?view=timeline` | All tasks on a dated line | Upcoming groups a week by day | **Keep;** same row as Board. |

Pins: **A** Board and Timeline come after the last row of All, so a list of 40 tasks hides them. **B** no
button opens Home (only `hq://web/home` or `hq://web/vault`, and nothing in the app makes that link).
**C** While you were away is on the phone only inside Home. **D** its own failure words, and the page does
not load again by itself when HQ is back.

## 3. Decision

**Retire Home on the phone.** Everything on it but two sections is already native and better placed. Of
those two, **While you were away** earns a place: it is the only summary of what HQ checked and finished
on its own, so it becomes a native card on Tasks Today that opens a native page. **System activity** (notes
HQ writes about its own runs) stays on the web app in a browser. `hq://web/home` and `hq://web/vault` keep
working and open Notes, and the web host itself refuses `/vault`, so no page inside it can reach Home
either.

**Keep Usage, Board and Timeline as web pages:** nothing native does what they do. Board and Timeline get
one row near the top of All. Every web page gets a second line saying it is a web page and what it shows,
and uses the app's HQ states from `01-offline.md`.

**Say only what the briefing knows.** A task that failed a check went back to whoever works on it; it was
reopened, not fixed. A task in its finishing window finishes by itself, and the phone cannot hold it yet,
so the phone does not say "unless you stop it".

Rejected:

- **Give Home a button** (a sixth place, or a row in Settings). It would repeat Notes and Tasks in another
  look, in a WebView that is slower on an A32 and fails differently, to reach one card.
- **Retire Board and Timeline too.** Owners who plan a week ahead use Timeline; Upcoming covers only seven
  days.

## 4. Screens and states (After: three web pages, one native summary)

- **A1 · Tasks Today, with what HQ did** (pin 1). "Tasks · Connected · Copilot 912 left", tabs Today,
  Upcoming, All. Card at the top: heading "While you were away" with "last 24 hours", lines "HQ checked and
  finished 4 tasks.", "1 went back to you to finish.", "1 finishes at 18:00.", chevron on the right.
  "Waiting on you 2": "Soak, reboot test, cleanup of old copies · Stuck · needs a reboot window", "Nightly
  off-device backup · Ready to check · HQ finished 3 h ago · [Check it]". [New task].
- **A2 · While you were away, the page.** Title "While you were away" / "Last 24 hours · checked at
  14:32". "Finishing soon 1" (pin 2): "T-7 · Rotate the backup key · Finishes at 18:00". "Checked and
  finished 4": "Phone app · 3 tasks", "Backups · 1 task". "Went back to you to finish 1": "T-9 · Fix the
  share sheet title · Failed a check. Reopened for you to finish." "Waiting on you 2" (pin 3): "2 tasks
  wait on you. They are on Today." [Go to Today]. Footer: "From HQ's checks of finished work. Spot checks
  and holds are on the web app."
- **A3 · Tasks All: Board and Timeline at the top** (pin 4). "Connected · 9 active"; chips Active, Stuck
  1, Ready to check 1, Done; row "See all tasks as · [Board] [Timeline]" right under the chips; then "In
  progress 2" (T-3 "1 of 3 sub-tasks done", T-4 "HQ is working on it · due Sunday"), "Stuck 1" ("T-1 ·
  Stuck 2d · Soak, reboot test, cleanup of old copies · Waiting on you · needs a reboot window").
- **A4 · Board, marked as a web page** (pin 5). Bar "Board" with the second line "Web page · all tasks".
  "The web app's Board page. Loaded as today, with host=android. Not drawn here. The filter on All does
  not carry over: the line says "all tasks". A link inside the page to /vault does not load: the app opens
  Notes instead."
- **A5 · A web page while HQ is starting** (pin 6). Bar "Timeline / Web page · all tasks by date". The
  shared NothingYet: "Timeline opens when HQ is up" / "HQ is still starting. It will try again by itself."
  Note: "HQ stopped: "HQ has stopped. Start HQ to try again." with Start HQ. Server HQ pages are not loaded
  here. The renderer gone: "The page stopped working." with Try again, as today."
- **A6 · hq://web/home now lands on Notes** (pin 7). Notes home: "Notes · Connected · Copilot 912 left",
  "Search your notes", folders Inbox, Projects, Daily, Logs; Pinned; Recent; [New note]. No message.
- **A7 · Today, signed in to a server HQ** (pin 8). Card "While you were away · last 24 hours": "HQ checked
  and finished 4 tasks.", "1 went back to you to finish."; sub-heading "On your server HQ": "HQ checked and
  finished 6 tasks.", "2 went back to Codex to finish."
- **A8 · The page, server HQ can't be reached** (pin 9). "While you were away / Last 24 hours · checked at
  14:32". "On this phone": "Checked and finished · 4 tasks", "Went back to you to finish · 1 task". "On
  your server HQ": amber strip "Your server HQ can't be reached. Check your internet." / "Saved on this
  phone at 14:18."; "Checked and finished · 6 tasks · hq.example.ts.net", "Went back to Codex to finish ·
  2 tasks". Footer as A2.
- **A9 · Today card at 360 dp, font scale 2.0** (pin 10). The heading "While you were away" and "last 24
  hours" stack on two lines; the three lines wrap as whole sentences; the chevron stays on the right.

## 5. Exact copy

| Where | String |
|-------|--------|
| Card heading | "While you were away" · "last 24 hours" |
| Card lines (zero parts left out) | "HQ checked and finished 4 tasks." · "1 went back to you to finish." · "1 finishes at 18:00." |
| Went back, a helper's task | "2 went back to Codex to finish." |
| Server part (heading, lines) | "On your server HQ" · then the same lines from that server |
| Server can't be reached | "Your server HQ can't be reached. Check your internet." · "Saved on this phone at 14:18." |
| One task | "HQ checked and finished 1 task." |
| Page title, line | "While you were away" · "Last 24 hours · checked at 14:32" |
| Section labels | "Finishing soon" · "Checked and finished" · "Went back to you to finish" · "Waiting on you" |
| Finishing row | "Finishes at 18:00" · "Finishes tomorrow at 09:00" (phone's 12 or 24-hour setting) |
| Finished row, per initiative | "Phone app · 3 tasks" |
| Went back row | "Failed a check. Reopened for you to finish." |
| Waiting on you | "2 tasks wait on you. They are on Today." Button: "Go to Today" |
| Footer | "From HQ's checks of finished work. Spot checks and holds are on the web app." |
| Page, HQ not answering | From rules section 2: "Couldn't load While you were away. HQ is still starting. It will try again by itself." (or: "HQ has stopped. Start HQ to try again.") |
| All row | "See all tasks as" · "Board" · "Timeline" |
| Web page line | Board: "Web page · all tasks" · Timeline: "Web page · all tasks by date" · Usage: "Web page · spend and forecast" |
| Web page, waiting | "Timeline opens when HQ is up" · "HQ is still starting. It will try again by itself." |
| Web page, stopped | "Timeline opens when HQ is up" · "HQ has stopped. Start HQ to try again." Button: "Start HQ" |
| Renderer gone (kept) | "The page stopped working. Try again." Button: "Try again" |
| Usage link (kept) | "Open the full usage page" |

All counts use plural resources ("1 task", "4 tasks").

## 6. Behaviour rules

1. **The card shows on Today only when there is something to say:** any of finished, went back to finish,
   or finishing soon above zero. Never while verification is only watching, and never on an HQ that has no
   verification (404). Zero parts are left out, not shown as "0".
2. **The page** is pushed over the shell like a task. Rows open the native task page; an initiative row
   opens All filtered to that initiative with the Done chip on. Finishing rows come first, nearest time
   first.
3. **Waiting on you is counted, not repeated:** those tasks are already under Waiting on you on Today, so
   the page points there.
4. **Board and Timeline sit right under the All chips,** before the first group, always in view without
   scrolling. They open the whole task list; the filter and chip on All do not carry over, which the web
   page line says.
5. **Every web page says it is one,** under its title, so the owner expects it to look and fail
   differently.
6. **A web page that cannot load** uses the shared state from `01-offline.md` and loads by itself as soon
   as HQ answers. Try again stays only for a renderer that died.
7. **Old Home links land on Notes home,** with no message: everything Home listed about notes is here. The
   same holds for a link to `/vault` inside Board, Timeline or Usage.
8, 9. **A server HQ** adds its own part under "On your server HQ", read with the phone's device token.
   When it can't be reached, the part shows what was saved on the phone, with its time. Sign-out,
   Disconnect or a refused sign-in removes the part at once.
10. **At font scale 2.0 and 360 dp** the card's lines wrap as whole sentences; the chevron stays on the
    right, never under the text.

- **"Went back to you"** only when the task has no helper assigned; a helper's task names the helper. Never
  "fixed".
- **Links that change:** `hq://web/home` and `hq://web/vault` open Notes (they opened the web Home).
  `hq://web/usage`, `board` and `timeline` are unchanged.
- **Reading the briefing:** once when Today opens, then at most every 2 minutes while it is on screen, and
  when HQ turns Connected. On patchy data a failed read keeps the copy saved on the phone and shows its
  time.

## 7. Android versions

| Version | Behaviour |
|---------|-----------|
| 8 and 9 | The card and the page are native, so they look the same. The web pages run in Android System WebView, which updates from the Play Store. If the web app does not run on the 8.1 emulator's shipped WebView, the page shows "The page stopped working." with Try again. |
| 10 and 11 | No difference for this fix. |
| 12 | No difference for this fix. |
| 13+ | No difference. Predictive Back (14+) on a web page goes back inside the page first, then to Tasks, as today. |
| Every version | At font scale 2.0 the card's heading and its "last 24 hours" stack on two lines (A9). Lines are whole sentences and wrap; nothing is cut with "…". |

**Server HQ:** when the phone is signed in to a server HQ, Today reads that server's briefing too, with the
phone's own device token. The server part gets its own heading, "On your server HQ". It is saved on the
phone like the task list and cleared on server sign-out, Disconnect or a refused sign-in.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `ui/webhost/WebHost.kt` | Drop `WebPage.Home` (line 10); add `subtitle` per page; `allows()` (lines 35-38, today any path on 127.0.0.1 and the hub port) also refuses the path `/vault` and anything under it, and the screen opens Notes for it; failure words are the string resources of rules section 2. | yes |
| `ui/webhost/WebHostScreen.kt` | The second line in the bar; the not-answering state is the shared `NothingYet`; reload by itself when the connection turns Connected. | yes |
| `ui/shell/Links.kt` | `webPage()` (line 54) maps "home" and "vault" to `LinkTarget(Place.Notes)`. | yes |
| `data/HqClient.kt` | `verificationBriefing(hours = 24)` on `GET /api/verification/briefing`; a 404 means this HQ has no verification. For a server HQ, the same route through `ServerHq.call()` (line 125) with the phone's device token, which the web auth middleware accepts like the web token (`crates/hq-web/src/auth.rs:386-420`). No server change is needed. | yes (function NEW) |
| `ui/tasks/Away.kt` | `AwayWords`, the parsed briefing: `completed_total`, `reworked` with each task's id, `veto_windows`. | NEW |
| `ui/tasks/AwayPage.kt` | The page. | NEW |
| `ui/tasks/TasksViewModel.kt` | Reads the briefing when Today opens and when HQ turns Connected, and saves it on the phone like the task list. The "went back" line names the helper from the task's assignee, else "you". | yes |
| `ui/tasks/TasksScreen.kt` | `WebViews()` (line 156) becomes `SeeAsRow` under the All chips; `AwayCard` at the top of Today. | yes |
| Tests | `LinksTest` (home and vault go to Notes), `AwayWordsTest` (zero parts left out, plurals, "you" or the helper, times past midnight), `WebHostTest` on `allows()` for `/vault`, `/vault/x` and `/tasks`. | `LinksTest`, `WebHostTest` yes; `AwayWordsTest` NEW |

## 9. Server, release or desktop changes

None, per the board. Check before building: CLAUDE.md describes the verification REST routes as
owner-only (`crates/hq-web/src/verification_api.rs`); confirm the phone's web token and a server's device
token both pass that gate for `GET /api/verification/briefing`.

## 10. Acceptance checks (each fails on today's build)

1. `WebHostTest`: `allows("http://127.0.0.1:{port}/vault")` and `/vault/Projects` are false,
   `/tasks?view=board` is true. Fails today: `WebHost.kt:35-38` allows every path. `hq://web/home` and
   `hq://web/vault` open Notes home.
2. With a briefing that has 4 finished, 1 reworked and 1 in its window, Today shows "HQ checked and
   finished 4 tasks.", "1 went back to you to finish." and "1 finishes at 18:00." in that order; with all
   zero, no card. A source check finds no "fixed" and no "unless you stop it" in `ui/tasks`.
3. A reworked task assigned to Codex reads "1 went back to Codex to finish." (`AwayWordsTest`).
4. Signed in to a test server HQ with 6 finished: the card shows "On your server HQ" and its lines. Stop
   that server: the page shows its saved part with "Saved on this phone at {time}". Disconnect: the part is
   gone from the card, the page and the phone's files.
5. On an HQ answering 404 for the briefing, Today shows no card and no error.
6. Each page row opens the native task; Go to Today switches the tab.
7. On All with 40 tasks, Board and Timeline are visible without scrolling.
8. Board, Timeline and Usage show their web page line; with hq stopped they show the shared state and load
   by themselves after Start HQ.

## 11. Check on device

- Board and Timeline on an 8.1 emulator with the WebView it ships with.
- Predictive Back (14+) on a web page, on a Pixel image.
- FixPlan "Still to prove": none.

## 12. Open questions and decisions it depends on

- **Stop a task in its finishing window from the phone?** The web app can; the window is 24 hours and the
  owner is often only on the phone. Recommendation: yes, as "Keep it open" on the task's own page, with a
  required note, in the next round. Until then the row opens the task. (Decided: D18, "Next round.")
- **Make Timeline native later?** Recommendation: yes, after Board, if owners use it; Board stays web.
  (Decided: D21, "Native Timeline: Later.")

Depends on: D18, D21, rules sections 2, 6, 8; `01-offline.md` (shared states, saved copies, server
parts); `../wave-1/05-progress.md` (counts).
