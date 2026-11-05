# Web wave 4 · 07 · Helpers: what they ask, what they do (WB5, WB6, W8, W25, W26, WR17, WR18)

Boards: **AlignedHelpers** (desktop, a helper asking, the details panel; laptop width, working, its screen
opened; Stop and Stop every helper; stopped, lost, archived, one task's helpers, HQ's own work),
**AlignedHelpers2** (Start a helper; phone width; tablet; 2200; 320 and 200%; the phone as the hub and
Lite), **AlignedShell** frames 2 and 3, AlignedReview WB5, WB6, W8, W25, W26, M12, M16, Drift 01, 03, 04,
09. Follows `../../rules-web.md` sections 6 and 8 and `../../rules-shared.md` section 4. Where this spec and
the rules differ, the rules win.

## 1. Finding and the review items it closes

- **WB6, W32.** Decline on the left, Allow once on the right; the card quotes the command as the helper drew
  it; HQ's own description is labelled "In HQ's words".
- **W8 revised.** Technical details (the live terminal) is closed by default at every width. Opened, it
  carries the terminal rules: Esc goes to the helper only while "Typing to the helper" is on, Ctrl ] always
  leaves, Ctrl C asks for a second press.
- **WB5, W25 revised.** No "Allow helpers again" on the web; the lock says it lifts only in HQ Hub on that
  computer.
- **M16.** "Stop every helper" no longer claims an unreachable computer stops later: "That computer can't
  be reached, so its helpers were not stopped." Stopping one helper no longer says "start a new helper":
  Resume exists.
- **M12.** Resume a stopped helper, Lost contact and Stopped, Show archived and Restore, the per-task
  filter, HQ's own runs ("HQ is working on it", then "Done by HQ", `../../resolutions.md` B4), the
  purpose picker, the name field and Advanced (its own branch).
- **W26.** Edge-worker runs are "Done by HQ", read from `/api/edge-worker`, open their chat, no Stop or
  Rename.
- **WR17.** "For a task" is gone from Start a helper: the start route takes no task (owner decision 5).
- **WR18 (owner to confirm).** The "Typing to the helper" switch.
- **Drift 01** "14m ago ago"; **Drift 03** five colours for waiting; **Drift 04** on a phone the answer
  panel covers the agent's screen; **Drift 09** internal names; **W06** title-link cards.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/routes/sessions.tsx:28` `?id=`, `?task=`; `:46` filters with `include_archived`; `:104,
  248` "New agent"; `:155` the details panel only at `2xl` (1536 px); `:294` "Show archived".
- `components/sessions/SessionDetail.tsx`, `SessionTerminal.tsx` (the live coloured terminal, always on
  when a helper is open), `BlockedCallout.tsx:60` sticky at the bottom (`sticky bottom-0`), growing with
  every option (Drift 04), "This agent is waiting for you" (63), Approve then Decline (76-85), More keys.
- `components/sessions/SessionInspector.tsx:37-38` "{started} ago", "{seen} ago" with `relTime` already
  returning "14m ago".
- `components/sessions/SessionSendBox.tsx:11` `INTERRUPT_CONFIRM_MS = 4_000`; `:55` the arm timer; `:70`
  "Confirm stop what it is doing (Ctrl+C)".
- `components/sessions/SessionActions.tsx:103-131` Stop ("Yes, stop it", "Keep going"), Resume (133),
  Archive and Restore (149-159).
- `components/sessions/EmergencyStop.tsx` "Stop every agent"; `:103, 192` "Run hq host unlock on it to
  start agents again."
- `components/sessions/NewAgentDialog.tsx:100` "New agent": Computer, Folder, Agent (grouped "Coding agents"
  and "General-purpose agents"), Purpose for a general one (general, research, review, monitor), "What
  should it do first?", "Name (optional)" up to 80, "Its own branch (optional)" for a coding one.
- `lib/workbench.ts:82-90` `statusInfo`: "Lost contact" (warn) when the pane is gone or orphaned.
- `lib/sessionsApi.ts:194` lockdown; `HostLockdown` source "breaker".
- Server: `crates/hq-web/src/workbench_api.rs:164-176` `SpawnBody` (harness, host, folder, new_folder,
  prompt, label, purpose, branch) has no task field; `:189-190` `lockdown_handler` stops only reachable
  computers and reports each ("an unreachable one is not locked"); `POST /api/harness-sessions/{id}/resume`
  with an optional prompt; `/api/edge-worker` (`edge_worker_api.rs`).

## 3. Decision

**The Workbench becomes Helpers, with the phone's cards, words and order: waiting on you, working, ready,
offline or lost, finished or stopped. A question is answered on the helper's page with both outcomes said
first. Its screen sits under Technical details, closed until you open it. The web keeps what the phone
lacks: other answers from the screen, typing into it, the details panel, Resume, archived helpers and Stop
every helper.**

Rejected:

- **The terminal open from 1024 (W8 as first drawn).** It costs data and CPU, and can show secrets on a
  shared screen.
- **A sticky answer panel.** It covered the question on a phone (Drift 04); the answer card sits in the page.
- **"Allow helpers again" on the web.** A stolen sign-in could undo the stop.

## 4. Screens and states

### 4.1 Desktop, a helper asking, the details panel (Helpers 1)

- **H1 · 1440.** Top bar "Helpers" / "Connected · 2 working" / chip. List pane "5 helpers" [Start a helper];
  chips "All", "Needs your OK 1", "Working 2", "Finished 4". Groups, each heading in its state's one colour
  (amber for Needs your OK and Computer offline, blue for Working, ink for Ready, grey for Finished):
  "Move the build from scripts to Gradle · Needs your OK · Claude · on your Mac mini · 40 min · Wants to run
  a command"; "Build the Android app · Working on it · Codex · on your Mac mini · 12 min · Updating the
  build files"; "Build the native task list · Codex · on your Mac mini · 25 min · Writing the list screen";
  "Ready": "Check the tunnel notes · Ready · Claude · on your Studio laptop · 1 h · Waiting for your next
  instruction"; "Computer offline": "Tidy the release notes · Computer offline · Claude · on your Studio
  laptop · 3 h · HQ cannot reach your Studio laptop right now"; "Finished and stopped": "Fix the backup
  script · 2 h ago".
- **H2 · The open helper.** "Claude wants to run a command" / "What it would run, as Claude drew it" /
  `./gradlew assembleDebug --offline` / "Claude's reason, from its screen: "Building once with the new
  Gradle files to check nothing else uses the old scripts."" / "In HQ's words: Claude, on your Mac mini,
  wants to run a command in hq." / "If you allow it, it runs this one command on your Mac mini, then keeps
  going." / "If you decline, it stops that step and waits. You can tell it what to do instead." / [Decline]
  [Allow once] / "Allow once lets it run this one command." "Other answers · Its choices, as on its screen":
  "2. Yes, and don't ask again for ./gradlew · It will not ask about this again", "3. No, and tell Claude
  what to do". "Technical details · Its screen and keys. Closed until you open it." [Tell Claude what to do
  instead].
- **H3 · Details panel (from 1400).** "Helper · Claude", "Computer · Mac mini", "Folder · hq", "Started · 40
  min ago", "Last seen · 1 min ago", "Task · Gradle build", "Chat following it · Release checklist" [Rename]
  [Follow in Chat] [Stop Claude] (ink; red only on the dialog's button).
- **Fixes:** "Started 14m ago ago" reads "Started" with "40 min ago"; Needs your OK amber everywhere; cards
  are links so Tab reaches every one; the answer card sits in the page, never sticky, and nothing covers the
  question at any width. Allow once shows only when HQ can read that the highlighted choice is a plain yes,
  once; otherwise the card offers its choices under Other answers. When the screen moved on: "It asked
  something else just now. Look again before you answer." and the buttons wait for the new question.

### 4.2 Laptop width, working, its screen opened (Helpers 2)

- **K1 · 1024 to 1399.** List and helper side by side. "Right now" says what it is doing in one sentence;
  quick replies [Keep going] [Explain what you did] send the phone's words. Technical details open: the
  screen (`.a-term`): "Edit app/build.gradle.kts", "+ android { namespace = "example.hq.spike" }", "Running
  ./gradlew tasks to check the new layout", "> Task :app:preBuild UP-TO-DATE", "> Task
  :app:compileDebugKotlin", "Updated a few seconds ago". Switch "Typing to the helper" / "While on, keys and
  Esc in its screen go to Codex. Ctrl ] always leaves the screen." Keys: Esc, Enter, Up, Down, y, n, Ctrl C
  (armed: "Press again", "Press Ctrl C again within 4 s to stop what Codex is doing.", amber with the warn
  icon, then back). [Tell Codex something]. Details "Codex", "12 min ago", "just now", "Android build",
  "None" [Stop Codex].
- **Rules:** Esc closes sheets everywhere, except inside a helper's screen while "Typing to the helper" is
  on; then Esc is the helper's. Ctrl ] always leaves the screen and returns focus to the page. Ctrl C needs
  two presses within 4 s (`INTERRUPT_CONFIRM_MS`). Closed by default: the screen opens on request and stays
  open for that helper in this browser; with it closed, the screen is not read.

### 4.3 Stop, and Stop every helper (Helpers 3)

- **S1 · Stop every helper** (in the Helpers menu): "Stop every helper?" / "Every helper on the computers you
  pick stops now. Their work so far is kept. New helpers can't start there until you allow them again in HQ
  Hub on that computer." / "Where": "Every computer HQ can reach · 3 helpers on your Mac mini", "Your Mac
  mini · 3 helpers", "Your Studio laptop · Can't be reached right now. Its helpers can't be stopped from
  here." / "Why (optional)" / [Keep them running] [Stop every helper] (red, stacked when the pair does not
  fit on one line, `../../decisions.md` WR27).
- **S2 · Stop one helper.** Word for word (`../../resolutions.md` B18): "Stop Codex?" / "It stops now. Its
  work so far is kept." / [Keep it running] [Stop Codex]. While it runs: "Stopping…", both buttons off.
  Failed: "Couldn't stop Codex. Your HQ can't be reached. Try again when it's back." The dialog stays.
- **S3 · After Stop every helper.** The lockdown banner (`../../resolutions.md` C-5, the same words on the
  phone): "All helpers are stopped. Allow them again in HQ Hub on that computer." Under it, the fact:
  "Your Mac mini · you stopped every helper at 14:02". On Helpers and in Start a helper until lifted. No
  button. Technical details: "or run hq host unlock on that computer".
- **S4 · A computer that can't be reached.** "That computer can't be reached, so its helpers were not
  stopped." [Try again], kept until closed.
- **S5 · Stopped by HQ itself.** The same banner, "All helpers are stopped. Allow them again in HQ Hub on
  that computer.", with the fact "Your Mac mini · too many started at once, so HQ stopped them at 09:12"
  (source "breaker").

### 4.4 Stopped, lost, archived, one task's helpers, HQ's own work (Helpers 4)

- **O1 · One task's helpers** (`/helpers?task=T-13`, phone link `hq://helpers`): "Connected · 1 working";
  banner "Helpers on T-13 · Showing the helpers on T-13 Android build." [Show all]. [Show archived] [Archive
  finished and stopped].
- **O2 · Lost contact.** "Sign the debug build · Lost contact · Codex · on your Mac mini · 2 h · HQ lost
  contact with this helper" (amber, wifi-off). An unreachable computer is "Computer offline", never
  "Stopped".
- **O3 · Finished and stopped.** "Stopped · Codex · on your Mac mini · Stopped 2 h ago"; "Fix the Gradle
  cache · Finished · Claude · on your Mac mini · Finished yesterday" (grey).
- **O4 · Archived.** "First try at the Android build · Codex · on your Mac mini · Finished 4 days ago"
  [Restore] beside the title link. "Archive finished and stopped" acts at once: "3 helpers archived · Undo".
  Only a helper that is not running can be archived.
- **O5 · HQ's own work** (`../../resolutions.md` B4). The group is "HQ's own work". While working:
  "Android build: bump the target version · HQ is working on it · on your phone · 6 min · Working on
  T-13. Opens its chat." Finished: "… · Done by HQ · on your phone · finished 12 min ago". No screen, no
  Stop, no Rename; a card opens its chat. On the task: "HQ is working on it", then "Done by HQ".
- **O6 · Resume.** "Codex stopped 2 h ago" / "Resume starts it again in the same folder. It keeps its
  earlier conversation when Codex can." / "What should it do next? (optional)" [Resume] [Archive]. "Last
  thing it said · Stopped by you at 12:40. The debug build had finished." Failed: "Couldn't resume Codex."
  and the catalogue's reason.

### 4.5 Start a helper (Helpers 2, section 1)

- **N1.** A centred sheet from 768: "Start a helper" / "What should it do first? (optional)" ("Move the
  build from scripts to Gradle") / "Which helper": "Claude", "Codex", "Cursor, not set up here", "Hermes",
  "OpenClaw" (two groups as the built dialog: coding helpers and general helpers) / "Name (optional)" (up to
  80) / "Where": "Your Mac mini", "Your Studio laptop, can't be reached" / "Folder · hq" [Change] /
  "Advanced": "Its own branch (optional) · It works in a copy of the folder on this branch, so two helpers
  never edit the same files. Needed when another helper already works in this folder." / "It asks before
  it runs commands or changes files. You answer each step here and in your Inbox." / [Cancel] [Start].
- **N2 · A general helper.** "Read the new relay notes each morning" / "What is it for?" General, Research,
  Review, Monitor / "Hermes is a general helper. It never changes code. A monitor or reviewer comments on
  its task and never moves it."
- **N3 · After Start.** "Claude has started." [Open the helper]. If it already waits: "It is already
  waiting for you. You can answer once it opens." and its screen.
- **N4 · Can't start.** No computer answers: Where shows "HQ cannot reach your Mac mini right now." and
  Start is off with "Start needs a computer HQ can reach." Locked: the lock banner (S3) and Start off.
  Failed: "Couldn't start Claude." and the catalogue's reason.
- **Owner decision 5 (WR17):** no task field until the start route takes one; then "Start a helper on this
  task" from the task page.

### 4.6 Other widths, the phone as the hub, Lite (Helpers 2)

- **390.** The phone's Helpers. Technical details starts closed. The list card answers in place when
  everything needed is on it (title link, buttons beside it): "Tidy the export names · Needs your OK ·
  Claude · on your Mac mini · 8 min · Wants to edit one file: ExportMenu.tsx · Decline · Allow once · Allow
  once lets it change this file one time." Otherwise the card opens the helper's page, whose question card
  keeps its quoted command (`./gradlew assembleDebug --offline`, "Claude's reason: …", "If you allow it, …",
  "If you decline, it stops that step and waits.", Other answers). Its screen opened on request: "Bash
  command", "./gradlew assembleDebug --offline", "Do you want to proceed?", "> 1. Yes", "2. Yes, and don't
  ask again for ./gradlew commands in hq", "3. No, and tell Claude what to do differently (esc)", "Updated a
  few seconds ago"; "Typing to the helper" starts off on a phone (the on-screen keys send one key each);
  Ctrl C asks for a second tap within 4 s. Start on a phone: the job, the helper and where; the name, the
  purpose and the branch fold under one row ("Name and Advanced"). Answered from a card: "Allowed. Claude is
  working again." or "Declined. Claude is waiting." as a toast; failed, on the card in amber with the
  buttons live: "Couldn't send your answer. Your HQ can't be reached. Try again when it's back."
- **Saved helpers.** "Your HQ can't be reached", amber, no chip, one strip "Saved in this browser at
  14:02." "Claude · on your Mac mini · 40 min" / "Answers wait until your HQ can be reached." Answers, Start
  and Stop off with why; Technical details shows the saved screen, marked with the same time, keys off.
- **Starting.** "Starting HQ · 12 s"; cards with [Decline] [Allow once] off and "You can answer once HQ has
  started."
- **Empty per chip:** "Nothing is waiting for your OK." "No helper is working right now." "No finished
  helpers." All: "No helpers yet. Start one and it shows up here."
- **768 to 1023 (900).** The rail; the list takes the page; a helper opens in its place with "Back to
  helpers"; Start a helper is a centred sheet; cards run the full width, up to 760 px of text; details a
  side sheet; Search an icon until 1024.
- **2200 (60%).** The list keeps 360, the helper's page 760 and centred, the details panel 320. A wider
  screen never makes the terminal wider than its text.
- **320 and 200%.** At 320 the top bar holds the title and the state; Search, the balance and Settings in
  one menu. At 200% a 1040 window lays out at 520. The answer buttons stack only when they do not fit side by side
  on one line (`../../decisions.md` WR27).
- **The phone as the hub (HQ window).** "HQ on your phone · Pixel 8"; "Connected · HQ on your phone". The
  helpers still run on your computers; the phone's HQ reaches them as any HQ does. "Helpers from your
  phone's HQ · Pick one to see what it is doing." No web push; Needs you arrives as HQ Hub's system
  notice, Open only; a Needs you item for a helper on this computer says so. Saved copies and the outbox
  keyed by the HQ's id, labelled "Saved in HQ Hub at 14:32" (`../../resolutions.md` B11).
- **Lite.** No Helpers place (Lite does not serve helpers, the Workbench or hosts). The rail has Notes,
  Tasks and Inbox. A stored link to `/helpers` opens Notes with "This HQ is HQ Lite. It has no Helpers."

## 5. Exact copy

Every string quoted in section 4 is exact. Keys `helpers_*`. Retired: "Workbench", "New agent", "Stop all",
"Stop every agent", "This agent is waiting for you", "Approve", "Yes, stop it", "Keep going" (as the safe
choice of a stop), "Run hq host unlock on it to start agents again.", "HQ is not running. These are the
last helpers it had.", "carries on", "For a task (optional)", "start a new helper", "Resume starts it
again later." in the Stop dialog, "Helpers can't start on your Mac mini" as the banner, "Done by HQ" on a
run still working.

## 6. Behaviour rules

1. **Order and colour by state:** Needs your OK (amber), Working (blue), Ready (ink), Computer offline and
   Lost contact (amber, wifi-off), Finished and Stopped (grey). One colour per state everywhere.
2. **Answer cards** sit in the page; quote the command as the helper drew it, its reason, then "In HQ's
   words"; both outcomes said first; Decline left, Allow once right (red "Allow {verb}" when losing).
3. **A fresh read** before sending an answer; if the screen changed, say so and wait.
4. **Technical details closed by default** at every width; opening it starts the screen stream; closing
   stops it. Remembered per helper in this browser.
5. **Typing to the helper** (off by default): while on, keys and Esc go to the helper; Ctrl ] always
   leaves; Ctrl C needs a second press within 4 s.
6. **Stop and Stop every helper** are red dialogs; the lock lifts only in HQ Hub on that computer; no button
   on the web.
7. **Unreachable computers** are never reported as stopped or locked.
8. **Resume** reuses the same folder and resume token in a new process, or starts fresh when the helper
   cannot resume; an optional next instruction.
9. **Archive** only what is not running; "Archive finished and stopped" with Undo.
10. **HQ's own work** rows come from `/api/edge-worker`, in the group "HQ's own work", reading "HQ is
    working on it" while working and "Done by HQ" when finished (B4); they open their chat; no Stop or
    Rename.
11. **The details panel** docks from 1400 (W9), not 1536.
12. **The task filter** is `/helpers?task=`.

## 7. Browser and platform notes

- Keyboard capture in the terminal uses `keydown` on the terminal element only while the switch is on;
  Ctrl ] (`e.code === 'BracketRight'` with Ctrl) is handled before passing keys on, so it works on every
  layout.
- Ctrl C in the terminal must not copy when text is selected: with a selection, Ctrl C copies and does not
  arm.
- The screen stream pauses when the tab is hidden (`../wave-2/03-version-and-data.md`).

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/routes/helpers.tsx`, `helpers/$id.tsx` | The place (from `sessions.tsx`: 28, 46, 104, 155, 248, 294). | NEW |
| `apps/hq-web/src/components/sessions/SessionDetail.tsx`, `BlockedCallout.tsx` | The answer card in the page (60-85); Technical details. | yes |
| `apps/hq-web/src/components/sessions/SessionTerminal.tsx`, `SessionSendBox.tsx`, `useScreenStream.ts` | Closed by default; the typing switch; Ctrl ] and two-press Ctrl C (11, 55, 70). | yes |
| `apps/hq-web/src/components/sessions/SessionInspector.tsx` | The details panel facts (37-38). | yes |
| `apps/hq-web/src/components/sessions/SessionActions.tsx`, `EmergencyStop.tsx` | Dialogs, Resume, Archive, the lock banner without a button (103-159; 103, 192). | yes |
| `apps/hq-web/src/components/sessions/NewAgentDialog.tsx` | Start a helper (100). | yes |
| `apps/hq-web/src/components/sessions/SessionRow.tsx`, `lib/workbench.ts` | Cards and states (82-90). | yes |
| `apps/hq-web/src/lib/sessionsApi.ts` | Edge-worker runs; lockdown source words (194). | yes |
| `crates/hq-web/src/workbench_api.rs` | (Later, WR17) a task field on `SpawnBody` (164-176). | yes |

## 9. Server, desktop or release changes

Server: the losing flag on helper dialogs (where the host can tell), the client header on every changing
route (wave 0), an optional task field on the start route later (WR17). Desktop: lifting the lock in HQ Hub
(wave 3).

## 10. Acceptance checks (each fails on today's build)

1. Opening a helper does not open the screen stream until Technical details is opened (network check).
   Today the terminal streams at once.
2. The answer card is not `position: sticky`; at 390 the question's text is visible above the buttons.
3. Esc in the terminal with the switch off closes nothing in the helper and returns focus; with it on, it
   sends `esc`; Ctrl ] always leaves.
4. The lock banner has no "Allow helpers again" button. (Guards WB5.)
5. Stop every helper with one unreachable computer reports "That computer can't be reached, so its helpers
   were not stopped." Today the words differ.
6. The details panel docks at 1400. Today at 1536.
7. "Started" reads "40 min ago" once. Today "ago ago".
8. The Stop dialog reads exactly "Stop Codex?" · "It stops now. Its work so far is kept." · "Keep it
   running" · "Stop Codex"; the lock banner reads "All helpers are stopped. Allow them again in HQ Hub on
   that computer."; an edge-worker run reads "HQ is working on it" in the group "HQ's own work" and "Done
   by HQ" once finished (B4, B18, C-5).

## 11. Check on a laptop or phone

- Start Claude Code on the laptop from the web; answer its first question from the Inbox on the phone.
- Turn on "Typing to the helper", press Esc in a dialog of the helper, then Ctrl ] to leave.

## 12. Open decisions it depends on

- **WR18** (owner to confirm). **W8, W9, W11, W25, W26, WR17, WR27** (Recommended or Revised),
  `../../resolutions.md` B4, B11, B18, C-5.
- Depends on: `../wave-1/02-dialogs-approvals.md`, `../wave-3/01-this-pc.md` (lifting the lock),
  `06-tasks.md` (Helpers on a task), `04-search.md` (helpers in Search).
