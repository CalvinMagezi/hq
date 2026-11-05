# Wave 4 · 01 · One connection pattern for the whole app (finding #14, R4, M8, M9, D16, D17)

Board: **FixOffline** ("Fix 14: connection states"), finding 14, designed, not built, revised after
review, 5 Nov 2026. Area: one connection pattern for the whole app (`data/HqServices`, `ui/shell`, every
place); other fixes refer to it. Shared rules: `../../rules-shared.md` sections 2, 3, 5, 6, 7 win.

Headline: **When HQ is starting, stopped, or the AI cannot be reached: one status line, one sheet, one
strip, one set of words.**

Note on waves: FixPlan wave 0 builds "the Connection state and the failure catalogue (no UI yet)"; wave 2
(`../wave-2/04-startup.md`) puts the start steps in the Connection sheet; wave 4 (this file) builds the
connection UI in every place, saved copies and server HQ rows. Build the `Connection` type and the
catalogue first, the sheet with its start part in wave 2, the rest here.

## 1. Finding and the review items it closes

**The problem.** Each place handles a quiet HQ its own way. Notes shows a banner and nothing else,
Settings an amber box with nothing to tap, Tasks, Helpers and Inbox keep their last lists with three
different amber lines and live buttons that then fail; after 60 s a full screen hides everything. The
status line cannot be tapped, and tapping the selected tab does not scroll to the top.

**Who it hurts, how often.** Every owner, every day. HQ runs on the phone, so it is "down" each time
Android stops it to save battery, after every reboot and every restart from Settings. On patchy mobile
data the AI service drops out several times a day while HQ itself is fine, and the app does not tell
those apart.

What changed in this revision:

- **Review #14 and R4:** the full-screen start cover shows only on the very first launch. The Connection
  sheet now holds the four start steps, "usually about {N} s", "Stopped at" and "The phone is almost
  full" (P2, P2b, P3, P3b). It opens by itself at most once a day.
- **M9, slow starts:** "HQ has stopped" only when the process ended or its step stalled for the longer of
  60 s and twice the usual start. While it lives, the sheet says "HQ is still opening your notes.
  Restarting starts over."
- **R5, one catalogue:** every failure line is a rules section 2 sentence, amber with an icon, and the
  source check now covers `voice/`, `data/`, `capture/` and `AnswerReceiver.kt`. Check names and exit
  codes live only in Technical details.
- **M8, server HQ:** two server states, a server row in the sheet, "on your server HQ" on items, a strip
  for server items, and Settings › Server HQ (row 5).
- **D16:** the AI status comes from a new signed-in route, `/api/status/ai`, never `/health`. "The AI
  service isn't answering" no longer tells the owner to check the internet.
- **D17 and M1:** "Saved on this phone at 14:32", kept until replaced with its time shown, written
  atomically under a size cap, server parts cleared on sign-out, and data-extraction rules. Tested with a
  real phone-to-phone copy, not adb backup.
- **Shared words and colours:** "Connected" in grey, a blue dot while HQ works, an amber stop icon for
  "HQ has stopped" (was a red dot), "You're offline", one label "AI service and keys", "It will be sent
  when HQ is back.", "HQ" in capitals. Settings is drawn as rules section 5 ("1 signed in").
- **New:** the running notice (W1), 360 dp and font scale 2.0 frames (W2, W3), an Android versions card.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/` (`K` = `kotlin/example/hq/spike/`).

- `K/data/HqServices.kt:16-21` `HqStatus { Live, Starting(seconds), Saved(age) }`: "stopped for an hour"
  reads "Starting". The poll loop is at lines 40-52.
- `K/ui/shell/HqShell.kt:113-115` status words and colours: green "Live", amber "Starting HQ · {s} s",
  amber "Saved · {age}".
- `K/ShellActivity.kt:299-300` the "Saved" status after the chat link drops.
- `K/ui/startup/Startup.kt:34` "HQ did not start" covers every launch after 60 s.
- `K/data/HqClient.kt:49` `healthy()` reads `/health`.
- `java/example/hq/spike/HubService.java:219-237` `supervise()` waits on the hq process.
- `K/ServerHq.kt:52` `markSignedOut()`, `:57` `disconnect()`.
- `AndroidManifest.xml:16` `allowBackup="false"`; no data-extraction rules; `app/build.gradle.kts:84`
  targetSdk 35.
- Nothing in the app handles a re-tap of the selected tab: there is no `LocalReselect` or
  `ReselectEffect` anywhere in `apps/android` (the board says "as before"; it is new work).
- HQ: `crates/hq-web/src/lib.rs:265` `/health` (open), `:266` `/api/vault-status`; `crates/hq-web/src/auth.rs:119-128`
  `web_path_is_guarded` (everything under `/api/` needs a token); `crates/hq-web/src/profile_gate.rs:18`
  `LITE_API`. No `/api/status/ai` route.
- Retired phrases still in code, for example `K/voice/VoiceTurn.kt:148-154` ("HQ is not answering yet…",
  "HQ went quiet…").

### Before frames

- **B1 · Notes, HQ down.** "Notes · Saved · 4m"; banner "HQ is not running. Your notes will show again
  when it starts." (pin A); "Search your notes"; [New note].
- **B2 · A Settings page, HQ down.** "Messaging apps"; amber box "HQ is not running, so there is nothing to
  show yet. Try again when it is back." (pin B).
- **B3 · Tasks, HQ down.** "Tasks · Starting HQ · 40 s"; "HQ is not running. These are the last tasks it
  had." (pin C); "Workstream A: … · 1 of 3 steps done", "Workstream C: … · High · due Sunday"; "Could not
  finish it. HQ is not answering." (pin D).
- **B4 · Inbox, nothing kept.** "Inbox · Saved · 2m"; "HQ is not answering yet, so what is waiting on you
  cannot be shown." (pin E).
- **B5 · After 60 s, on any launch.** "HQ did not start" / "Android may have stopped it to save battery,
  or the phone ran low on memory. Nothing you captured is lost." / "Health check timed out after 60 s." /
  "Last start: 08:41 · exit code none" / Try again · Battery settings · Copy details (pin F).
- **B6 · Tasks All, deep in the list.** "Tasks · 9 active · Saved · 3m" (pin H); "To do 4": "T-6 Release
  build and certificate renewal · due Saturday", "T-7 Shrink or retire the hub droplet · due Saturday",
  "T-8 Write the pairing help page · for Alex", "T-9 Fix the share sheet title · HQ edge worker"; tapping
  the Tasks tab again does nothing (pin G).

What goes wrong: **A** Notes: a banner and an empty screen (nothing is kept from before). **B** Settings:
an amber box that says "Try again" with nothing to tap. **C** Tasks, Helpers and Inbox keep the last list
under an amber line, each in its own words and with no time; the lists live in memory only. **D** buttons
stay live and fail after the tap. **E** Inbox with nothing kept gets one amber line and no way forward.
**F** at 60 s "HQ did not start" covers the whole app on every launch. **G** tapping the selected tab does
nothing. **H** the status line is not a button; "Saved · 3m" shows on Tasks when only the chat link
dropped; "Starting HQ" is also what it says when HQ has been gone an hour. Six wordings for one fact: "HQ
is not running", "HQ is not answering", "HQ didn't reply. It may be restarting.", "HQ did not answer.
Check that it is running", "Not answering", "nothing to show yet". No word at all for "no internet" or
"the AI service did not answer": a chat just fails.

## 3. Decision

**Compute the connection once, in HqServices, and give every screen the same parts:** the status line (a
button), the Connection sheet, the OfflineStrip over a saved copy, and NothingYet when there is no copy.
Every list a place shows is saved on the phone after each good read, so HQ being down never means an empty
screen after the first launch. Actions that change HQ turn off instead of failing; new notes and tasks go
to the capture outbox that already exists.

**Name the state for what the owner can do about it:** wait (starting, restarting, reconnecting), act
(stopped, signed out, almost full), or it is not HQ (offline, the AI service, the server).

**The full-screen start cover shows only on the very first launch.** After that the sheet carries the
start: the four steps, how long it usually takes, and where it stopped. A slow start that is still alive
is never called stopped, so owners do not kill a start that would have finished.

Rejected:

- **A full-screen cover whenever HQ is down.** It hides notes and tasks that are on the phone and
  readable, which is most of what an owner on a bus wants.
- **"Stopped" at a fixed 60 s.** A large vault on an A32 can take longer. A fixed limit makes owners
  restart a start that was about to finish, again and again (M9).
- **The AI state in `/health`.** `/health` is open to anything on the phone and any proxy in front of a
  server (`auth.rs:121`). The AI state goes on a signed-in route.
- **Queue every change made while HQ is down.** Ticking a task done or answering a helper an hour late can
  be wrong by then; only new notes and tasks, which cannot clash, are queued.

### The pattern in three lines (for the other fixes to point at)

1. **One Connection state** from HqServices (Connected, Starting HQ, Restarting HQ, HQ has stopped,
   Reconnecting, Your server HQ can't be reached, Signed out of your server, You're offline, The AI
   service isn't answering), shown in the top bar's status line, a button that opens the Connection
   sheet: what is wrong, what still works, one action.
2. **While HQ itself is down,** every place shows the copy saved on the phone under one OfflineStrip ("HQ
   is starting. Saved on this phone at 14:32."), actions that change HQ turn off, new notes and tasks go
   to the outbox; with no copy, NothingYet; everything reads again by itself when HQ is back.
3. **Every failure line comes from one catalogue** (rules section 2): what could not happen, then the
   reason. For example: "Couldn't post the comment. HQ is still starting. It will try again by itself."

The first launch, with nothing on the phone yet, keeps its own full screen ("Starting HQ", First run
board). That is the only cover left; after it, the sheet holds the start steps.

## 4. Screens and states (After)

### 1 · The pattern: the status line, the sheet it opens, and the shared parts

**P1 · The status line, every state** (pin 1). The top bar's title column (place name, status line and
any place line) is one 48 dp button.

| Example | Meaning |
|---------|---------|
| "Notes · Connected · Copilot 912 left" | HQ answers and the phone is online. Grey, never green. The balance chip shows only in this state. |
| "Notes · Starting HQ · 12 s" | HQ has not answered since the app opened or the phone restarted. Blue: HQ is working on it. |
| "Notes · Restarting HQ · 4 s" | Only after the app asked HQ to restart (Background work, Restart HQ, a messaging change). |
| "Notes · HQ has stopped" (pin 2) | The process ended, or its start step stalled for the longer of 60 s and twice the usual start. Amber dot with a stop icon. |
| "Chat · Reconnecting · 1m" | Chat only: HQ answers, but the chat link dropped 5 s ago or more. |
| "Inbox · Your server HQ can't be reached" | Inbox and Helpers, where server items show; every place on a phone in Standby. |
| "Inbox · Signed out of your server" | The server refused this phone's sign-in. What came from it is already cleared. |
| "Chat · You're offline" | No working internet for 3 s. HQ on this phone is fine. |
| "Chat · The AI service isn't answering" | From HQ's signed-in AI status. HQ and the internet are fine. |
| "Tasks · Starting HQ · 12 s · 9 active" | The state comes first. A long line wraps; the state is never cut off. Name, line and status are one 48 dp button. |

**P2 · The sheet: HQ starting, with the start steps** (pin 3). Bottom sheet over Notes. "HQ is starting" /
"HQ runs on this phone. You can read and write while you wait." Checklist "Starting on this phone",
"Loading HQ", "Opening your notes and tasks", "Almost ready" with the 4 dp blue bar; "Step 3 of 4 · 12 s ·
usually about 25 s". Rows: "Internet · Mobile data", "AI service · OpenRouter · answered at 14:30", "Saved
on this phone · at 14:32". Button [Background work].

**P2b · Slow, but HQ is still alive** (pin 4). "Starting HQ · 72 s". "HQ is starting" / "HQ is still
opening your notes. Restarting starts over." Checklist; "Step 3 of 4 · 72 s · usually about 40 s";
"Internet · Wi-Fi"; "Saved on this phone · at 14:32". [Keep waiting] (primary) [Restart HQ] (second).

**P3 · The sheet: HQ has stopped** (pin 5). "HQ has stopped" / "Android may have stopped it to save
battery, or the phone ran low on memory. Nothing you wrote is lost." "Stopped at: Opening your notes and
tasks". Rows "Internet · Wi-Fi", "AI service · OpenRouter · answered at 14:30", "Saved on this phone · at
14:32". "Technical details" (closed). [Start HQ] (green), [Open Android settings] with "In Battery, choose
Unrestricted for AgentHQ.", [Copy details].

**P3b · Stopped: the phone is almost full** (pin 6). "HQ has stopped" / amber "The phone is almost full.
Free up 300 MB and try again." "Stopped at: Opening your notes and tasks". Rows "Free on this phone · 140
MB", "Saved on this phone · at 14:32". [Open Android settings] with "In Storage, free up space.", [Start
HQ], "Technical details".

**P4 · The sheet: you're offline** (pin 7). Over Chat. "You're offline" / "HQ is running on this phone, so
notes and tasks work. Chats need the internet to reach the AI service." Rows "HQ on this phone ·
Connected", "Internet · None since 14:41", "AI service · Needs the internet". [Open Android settings] with
"Turn on Wi-Fi or mobile data."

**P5 · The sheet: the AI service isn't answering** (pin 8). Over Chat. "The AI service isn't answering" /
"HQ is running and the phone is online. OpenRouter did not answer at 14:38. It may be busy, or your key may
be out of credit." Rows "HQ on this phone · Connected", "Internet · Mobile data", "AI service · OpenRouter
· no answer at 14:38", "Checked by HQ · at 14:39". [AI service and keys].

**P6 · The shared parts, built once in `ui/connection`.**

- OfflineStrip · starting (pin 9): "HQ is starting. Saved on this phone at 14:32." Top of the list in
  Notes, Tasks, Helpers, Inbox and Settings. The text opens the sheet.
- OfflineStrip · restarting: "HQ is restarting. Saved on this phone at 14:32." Same place, same look.
- OfflineStrip · stopped: "HQ has stopped. Saved on this phone at 14:32." [Start HQ]. The only state with
  an action in the strip. Amber with a stop icon.
- Older copy: "HQ is starting. Saved on this phone on Tue 3 Nov." A copy from before today names the day.
  It stays until a new read replaces it.
- Items from your server HQ: "Your server HQ can't be reached. Saved on this phone at 14:18." Only over
  items from the server. Cleared on server sign-out, Disconnect or a refused sign-in.
- Chat banner · stopped: "HQ has stopped. Your draft is kept." [Start HQ]. Chat keeps a banner above the
  conversation in every state but Connected.
- Failure line (pin 10): "Couldn't post the comment. HQ is still starting. It will try again by itself."
  Whole sentences, one string resource per action and reason (`fail_post_comment_starting`). Amber with
  an icon.
- Turned off while HQ is down: [Decline] [Allow once] at 50% opacity. Anything that changes HQ is off,
  never live and then failing. TalkBack says why.
- Nothing saved yet (pin 11): "Nothing saved on this phone yet" / "Your notes show here once HQ has
  started." When there is no saved copy. Stopped adds "HQ has stopped. Start HQ to try again." and Start
  HQ.
- Waiting to send (pin 12): heading "Waiting to send 1", row "Check the backup status · Saved on this
  phone. It will be sent when HQ is back." New notes and tasks made while HQ is down go to the capture
  outbox and show at the top of their list.
- Reading again: the 4 dp blue line under the top bar while a list is read again. Used by a second tap on
  the selected tab, and when HQ comes back.
- Technical details, opened: "Health check timed out after 60 s." / "Last start: 08:41 · exit code none"
  / "Stopped at: step 3 of 4". The only place for check names, raw times and exit codes. Closed by
  default. Copy details copies it.

### 2 · Every place while HQ is starting, with a copy saved on the phone

- **S1 · Chat** (pin 13). "Chat · Starting HQ · 12 s"; banner "HQ is starting. Your draft is kept. Send
  turns on when HQ is up."; thread "Samsung battery steps": "How do I stop Samsung putting the app to
  sleep?" / "Open Settings, then Battery, find AgentHQ and choose Unrestricted."
- **S2 · Notes** (pin 14). Strip "HQ is starting. Saved on this phone at 14:32."; "Search your notes";
  folders Inbox, Projects, Daily; Pinned "Mobile-First-Roadmap · Projects · 1d ago",
  "Phone-Cutover-Evidence · Projects · 1d ago"; Recent "Backup-Setup-2026-10-20 · 3h · Projects ›
  Agent-HQ", "Quick idea: widget copy · 1d · Inbox"; [New note] stays on (pin 15).
- **S3 · Tasks.** Strip; "Waiting on you 2": "Soak, reboot test, cleanup of old copies · Stuck · needs a
  reboot window", "Nightly off-device backup · Ready to check · HQ finished 3 h ago" with [Check it] off
  (pin 16); "In progress 2": "Workstream A: … · 1 of 3 sub-tasks done"; [New task].
- **S4 · Helpers** (pin 17). Strip; "Move the build from scripts to Gradle · Needs your OK · Claude · on your
  Mac mini · at 14:32 · Wants to edit one file: Chat.dc.html · [Decline] [Allow once]" (off); "Build the
  Android app · Last seen 14:32 · Codex · on your Mac mini · Updating the build files"; [Start a helper]
  (off).
- **S5 · Inbox** (pin 18). "Needs you 2 · Updates 4 · Done"; strip; "Claude needs your OK · Mac mini · 2
  min ago · It stopped to ask: Do you want to edit Chat.dc.html? · [Decline] [Allow once]" (off); "Delete 3
  old backups? · Backup task · 5 min ago · … · [Decline] [Allow delete]" (off, red).
- **S6 · Settings home, whole page, HQ starting** (pin 19). Strip "HQ is starting. Saved on this phone at
  14:32." This phone: "Background work · On · answers chats, runs tasks", "Keep running · 1 thing to
  check", "Notifications · Needs you on · Updates on · App updates on", "Phone assistant · Not chosen",
  "Capture and widgets · 2 widgets", "Keys on this phone · 1 key". Your accounts: "AI service and keys ·
  OpenRouter · 1 key", "Messaging apps · Telegram on · Discord off", "Other devices · 1 signed in". How HQ
  behaves: "Spending limits · $4.25 of $5.00 today", "Safety and limits · Asks before changes outside
  notes". Your data: "Backup and about · HQ version 0.9.1". Caption: "Drawn as rules section 5. The card
  slot on top is empty: no update, notifications are on, backup is fine. Speech model is hidden until it is
  downloaded. Server HQ shows only when used (V4)."

### 3 · HQ has stopped, nothing saved yet, a new task while down, offline, the AI service isn't answering

- **R1 · Chat, HQ has stopped** (pin 20). Banner "HQ has stopped. Your draft is kept." [Start HQ].
- **R2 · Notes, nothing saved yet** (pin 21). "Notes · Starting HQ · 30 s"; "Nothing saved on this phone
  yet" / "Your notes show here once HQ has started. The first start after a reboot can take up to a
  minute."; [New note].
- **R3 · Tasks, a new task while HQ is stopped** (pin 22). Strip "HQ has stopped. Saved on this phone at
  14:32." [Start HQ]; "Waiting to send 1": "Check Samsung battery whitelist · Saved on this phone. It will
  be sent when HQ is back."; "Waiting on you 2": "Soak, reboot test, cleanup of old copies · Stuck · needs a
  reboot window"; snackbar "Task saved on this phone. It will be sent when HQ is back."
- **R4 · A Settings page, stopped, nothing saved** (pin 23). "Messaging apps": "Nothing saved on this phone
  yet" / "These settings come from HQ. HQ has stopped. Start HQ to try again." [Start HQ] [What happened].
- **R5 · Chat, offline** (pin 24). Banner "You're offline. Messages wait here and send when you're back
  online."
- **R6 · Chat, the AI service isn't answering** (pin 25). Banner "The AI service isn't answering. HQ is
  fine. Try again soon." [AI service and keys]; in the thread "And for the Galaxy Tab?" then amber
  "Couldn't finish the reply. The AI service isn't answering. HQ is fine. Try again soon." [Try again].

### 4 · Tapping the selected tab, and HQ coming back

- **T1 · Deep in All, tap Tasks again** (pin 26). "Tasks · Connected · 9 active", "To do 4": T-6, T-7, T-8
  "for Alex", T-9 "Fix the share sheet title · for HQ". Tap the Tasks tab: scrolls to top.
- **T2 · At the top; a second tap reads again** (pin 27). The tabs and chips; the 4 dp blue line under the
  top bar; "T-3 · Workstream A … · 1 of 3 sub-tasks done", "T-4 · High · Workstream C … · HQ is working on
  it · due Sunday".
- **T3 · Chat, tap Chat again** (pin 28). Scrolls to the newest message: "Thanks. And the Galaxy Tab?" /
  "The same steps work on the Tab; the Battery page is under Device care." (fixture text; it holds a
  semicolon, which owner text may not, but this is an AI reply, not app copy).
- **T4 · Inbox, HQ back: the copy is replaced** (pin 29). "Inbox · Connected · Copilot 912 left"; "Claude
  needs your OK · Mac mini · 2 min ago · … · [Decline] [Allow once]"; "A helper is stuck · Mac mini · just
  now · Codex keeps hitting the same question, so HQ stopped answering it. · [Open helper]".

### 5 · Owners with a server HQ

- **V1 · Sheet: your server HQ can't be reached** (pin 30). Over Inbox. "Your server HQ can't be reached"
  / "Your server HQ can't be reached. Check your internet." / "HQ on this phone is fine. Notes and tasks
  on this phone still work." Rows "HQ on this phone · Connected", "Your server HQ · hq.example.ts.net · no
  answer since 14:20", "Internet · Wi-Fi", "Saved from your server · at 14:18". [Server HQ settings] [Try
  now].
- **V2 · Sheet: signed out of your server** (pin 31). "Signed out of your server" / "This phone is signed
  out of your server HQ. Sign in again." / "Its questions and saved items were removed from this phone."
  Rows "Your server HQ · hq.example.ts.net · signed out at 14:25", "HQ on this phone · Connected". [Not now]
  [Sign in again].
- **V3 · Inbox with server items, server not answering** (pin 32). Status "Your server HQ can't be
  reached"; strip "Your server HQ can't be reached. Saved on this phone at 14:18."; "Claude needs your OK ·
  on your server HQ · 14:15 · It stopped to ask: Do you want to edit Chat.dc.html? · [Decline] [Allow once]"
  (off, waits for its own HQ); "Delete 3 old backups? · on this phone · 5 min ago · … · [Decline] [Allow
  delete]" (live).
- **V4 · Settings › Server HQ** (pin 33). Strip "Your server HQ can't be reached. Saved on this phone at
  14:18." "Your server HQ": "Address · hq.example.ts.net", "Status · Can't be reached since 14:20",
  "Version · HQ version 0.9.1 · saved at 14:18", "Saved from it · 2 questions, 1 helper, its briefing".
  [Try now] [Disconnect server]. Note: "Disconnect asks first: "Keep connected · Disconnect server". It
  removes everything saved from the server at once."

How the server shows up:

- **Two HQs, one status line.** The HQ on this phone always comes first. The server's states show where
  its items are (Inbox, Helpers), and in every place on a phone in Standby, whose work runs on the server.
- **Server items say so:** "on your server HQ" in their second line, and their own strip when the server
  can't be reached (M8).
- **Sign-out is final for its data.** A 401 from the server, Disconnect, or the owner signing the phone out
  there clears every saved server item before the sheet draws. `ServerHq.markSignedOut()` and
  `disconnect()` call `SavedCopyStore.clearServer()`.
- **Approvals wait for their own HQ.** A server question can't be answered while the server can't be
  reached; a question from this phone still can.

### 6 · The running notice, Android versions, 360 dp at font scale 2.0

- **W1 · The running notice, Android 8 to 15** (pin 34). Four states drawn together: "AgentHQ · HQ is
  running / HQ is running", "… / HQ is starting", "… / HQ is restarting", "… / HQ has stopped / Tap to see
  why". "One notice at a time; the four are drawn together here. Channel "HQ is running", min importance,
  silent, no buttons on any version. A tap opens the app on the place last used, and the sheet when HQ has
  stopped. Server states never post here."
- **W2 · The stopped sheet at 360 dp, font scale 2.0** (pin 35). "Notes · HQ has stopped"; sheet title,
  body, "Stopped at: Opening your notes and tasks", [Start HQ] and [Open Android settings] stacked full
  width; the sheet scrolls.
- **W3 · Tasks at 360 dp, font scale 2.0** (pin 36). "Tasks · Starting HQ · 12 s · 9 active" wraps; strip
  "HQ is starting. Saved on this phone at 14:32."; "Nightly off-device backup · Ready to check · HQ
  finished 3 h ago".

## 5. Exact copy

Status line (state first, then the place's line):

| State | String |
|-------|--------|
| Connected | "Connected" (grey dot) |
| Starting | "Starting HQ · 12 s" (blue dot) |
| Restarting | "Restarting HQ · 4 s" (blue dot) |
| Stopped | "HQ has stopped" (amber dot with a stop icon) |
| Chat link dropped | "Reconnecting · 1m" (blue dot) |
| Server not answering | "Your server HQ can't be reached" |
| Server refused sign-in | "Signed out of your server" |
| No internet | "You're offline" |
| AI service | "The AI service isn't answering" |
| With a place's line | "Starting HQ · 12 s · 9 active" |
| TalkBack | "Status: Starting HQ, 12 seconds. Open details" |

OfflineStrip (over a saved copy):

| State | String |
|-------|--------|
| Starting | "HQ is starting. Saved on this phone at 14:32." |
| Restarting | "HQ is restarting. Saved on this phone at 14:32." |
| Stopped | "HQ has stopped. Saved on this phone at 14:32." Action: "Start HQ" |
| Server items | "Your server HQ can't be reached. Saved on this phone at 14:18." |
| Copy from before today | "… Saved on this phone on Tue 3 Nov." |

Chat banner:

| State | String |
|-------|--------|
| Starting | "HQ is starting. Your draft is kept. Send turns on when HQ is up." |
| Restarting | "HQ is restarting. Your draft is kept. Send turns on when HQ is up." |
| Stopped | "HQ has stopped. Your draft is kept." Action: "Start HQ" |
| Reconnecting | "Reconnecting. Your draft is kept. It can be sent once HQ is back." |
| Offline | "You're offline. Messages wait here and send when you're back online." |
| AI service | "The AI service isn't answering. HQ is fine. Try again soon." Action: "AI service and keys" |

Connection sheet:

| Element | String |
|---------|--------|
| Titles | "Connected" · "HQ is starting" · "HQ is restarting" · "HQ has stopped" · "Reconnecting" · "Your server HQ can't be reached" · "Signed out of your server" · "You're offline" · "The AI service isn't answering" |
| Connected | "HQ is running on this phone and answering." |
| Starting | "HQ runs on this phone. You can read and write while you wait." |
| Slow, still alive | "HQ is still opening your notes. Restarting starts over." Buttons: "Keep waiting" · "Restart HQ" |
| Steps (FixStartup) | "Starting on this phone" · "Loading HQ" · "Opening your notes and tasks" · "Almost ready" |
| Time | "Step 3 of 4 · 12 s · usually about 25 s" ("usually" from the second start) |
| Restarting | "HQ is restarting because a setting changed. It is back in a few seconds." |
| Stopped | "Android may have stopped it to save battery, or the phone ran low on memory. Nothing you wrote is lost." |
| Where it stopped | "Stopped at: Opening your notes and tasks" |
| Phone almost full | "The phone is almost full. Free up 300 MB and try again." |
| Reconnecting | "HQ is running, but the chat link dropped. The app reconnects by itself." |
| Server | "Your server HQ can't be reached. Check your internet." · "HQ on this phone is fine. Notes and tasks on this phone still work." |
| Signed out | "This phone is signed out of your server HQ. Sign in again." · "Its questions and saved items were removed from this phone." |
| Offline | "HQ is running on this phone, so notes and tasks work. Chats need the internet to reach the AI service." |
| AI service | "HQ is running and the phone is online. OpenRouter did not answer at 14:38. It may be busy, or your key may be out of credit." |
| Row labels | "HQ on this phone" · "Your server HQ" · "Internet" · "AI service" · "Checked by HQ" · "Free on this phone" · "Saved on this phone" · "Saved from your server" |
| Row values (as drawn) | "Mobile data" · "Wi-Fi" · "None since {time}" · "Needs the internet" · "{service} · answered at {time}" · "{service} · no answer at {time}" · "{address} · no answer since {time}" · "{address} · signed out at {time}" · "at {time}" |
| Buttons | "Start HQ" · "Keep waiting" · "Restart HQ" · "Open Android settings" · "Copy details" · "Background work" · "AI service and keys" · "Try now" · "Server HQ settings" · "Sign in again" · "Not now" |
| Line under Open Android settings | "In Battery, choose Unrestricted for AgentHQ." · "Turn on Wi-Fi or mobile data." · "In Storage, free up space." |
| Technical details (closed) | "Health check timed out after 60 s." · "Last start: 08:41" · "exit code none" · "Stopped at: step 3 of 4" |

Nothing saved yet:

| Element | String |
|---------|--------|
| Title | "Nothing saved on this phone yet" |
| Starting | "Your notes show here once HQ has started. The first start after a reboot can take up to a minute." |
| Stopped, a Settings page | "These settings come from HQ. HQ has stopped. Start HQ to try again." Buttons: "Start HQ" · "What happened" |

Failures (rules section 2), turned off, outbox:

| Element | String |
|---------|--------|
| Shape | "Couldn't post the comment." + the reason's sentence |
| Reasons | "HQ is still starting. It will try again by itself." · "HQ is restarting. Try again in a moment." · "HQ has stopped. Start HQ to try again." · "HQ didn't answer in time. Try again." · "Your server HQ can't be reached. Check your internet." · "This phone is signed out of your server HQ. Sign in again." · "You're offline. It will send when you're back online." · "The AI service isn't answering. HQ is fine. Try again soon." · "The phone is almost full. Free up {size} and try again." |
| Turned off, TalkBack | "Allow once. Needs HQ, which is starting" |
| Outbox heading, row | "Waiting to send" · "Saved on this phone. It will be sent when HQ is back." |
| Outbox snack | "Task saved on this phone. It will be sent when HQ is back." |
| Helper, last seen | "Last seen 14:32" |
| Server item | "on your server HQ · 14:15" |
| Chat reply failed (R6) | "Couldn't finish the reply. The AI service isn't answering. HQ is fine. Try again soon." · "Try again" |

Running notice (channel "HQ is running"): "HQ is running" · "HQ is starting" · "HQ is restarting" · "HQ
has stopped" · "Tap to see why".

## 6. Behaviour rules

1. **One state, computed once.** When several hold, the first wins: HQ has stopped, Restarting, Starting,
   Signed out of your server, Your server HQ can't be reached, Reconnecting (Chat only), You're offline,
   The AI service isn't answering, Connected. Connected is grey; the balance chip shows only then.
2. **HQ has stopped** only when the hq process has ended and HubService is not starting it again, or its
   start step has not moved for the longer of 60 s and twice the usual start. Health is polled every 1 s
   while down and 5 s while connected, as now.
3. **The starting sheet** shows FixStartup's four steps, a 4 dp blue bar in four parts, the seconds, and
   "usually about {N} s": the median of the last five good starts, from the second start on.
4. **Slow but alive:** "HQ is still opening your notes. Restarting starts over." Keep waiting closes the
   sheet. Restart HQ starts over and is counted as "Start HQ tapped while HQ was alive" (rules section 10).
5. **Stopped:** "Stopped at: {step}", Technical details closed. Start HQ restarts the service and the wait.
   Open Android settings opens the battery page (Samsung's first, then Android's), with one line naming
   the switch.
6. **Almost full:** when free space is below what HQ needs, the starting and stopped sheets add the line
   and Open Android settings goes to Storage. {size} is the database file's size plus 100 MB, less what is
   free: a design number to measure on the A32.
7. **You're offline** after 3 s with no validated network. Only the status line and Chat change. A message
   sent while offline waits in the chat as "Waiting to send" and goes, in order, when the network is back.
8. **The AI state** comes from `/api/status/ai`, read when the sheet opens and after a failed turn; never
   `/health`. A loopback AI backend never shows "You're offline". It clears on the next good turn, or after
   10 minutes.
9. **The strip** shows only while HQ (or, for server items, the server) is not answering, and only over a
   saved copy. It is the first list item; its text opens the sheet.
10. **Failure lines** are whole sentences from string resources, amber with an icon. A call that failed
    because HQ was still starting is sent again once when HQ answers, within 60 s; after that the line
    becomes "HQ didn't answer in time. Try again."
11. **With no saved copy** a place shows NothingYet; New note and New task stay on.
12, 22. **New notes and tasks while down** save to the outbox, show under "Waiting to send" and go when HQ
    is back, in the order made.
13, 20. **Chat while starting or stopped:** the draft stays, Send is off with its reason. Offline and the
    AI state leave Send on.
14, 19. **Saved copy:** written after each good read for Notes home and each folder opened, the three task
    tabs, Helpers, Inbox and the Settings snapshot; shown at once on a cold start; the time is when HQ
    answered. Kept until a newer read replaces it. Chats are not saved.
15. **New note and New task stay on** while HQ is down.
16, 17, 18. **While down, anything that changes HQ is off:** checkboxes, swipes, Check it, Decline and
    Allow once, Start a helper, Settings switches. Reading, opening, scrolling and drafts stay on. Helper
    cards say "Last seen 14:32". A losing request shows a red "Allow delete" on the right (rules section
    4).
19. **Settings** is drawn as rules section 5. Rows from HQ show saved values under one strip, never
    "Checking…". "This phone" rows are read on the phone and stay current.
20, 23. **Start HQ** appears only in the stopped state: strip, Chat banner, NothingYet and sheet.
21. **NothingYet keeps counting** in the status line; the body changes only at Stopped.
24, 25. **Chat's banner** is one component for every state but Connected. The AI banner's action is "AI
    service and keys", the one label for keys everywhere.
26. **Tapping the selected tab scrolls its list to the top,** animated within 30 rows, a jump past that.
    Tasks keeps its tab.
27. **At the top, a second tap reads the list again** from HQ, with the blue line; while HQ is down it
    opens the sheet instead.
28. **A second tap on Chat** scrolls to the newest message.
29. **When HQ is Connected again,** every place reads again by itself, the blue line shows, the strip goes
    and buttons turn on.
30, 31. **Server HQ:** the sheet gains a "Your server HQ" row with its name. A 401, Disconnect or a
    sign-out on the server clears every saved server item before the sheet draws.
32, 33. **Server items** say "on your server HQ", wait for their own HQ, and get their own strip. Settings
    › Server HQ shows its state and what is saved from it.
34. **The running notice** follows the phone's HQ only: one notice, min importance, silent, no buttons. A
    tap opens the app, and the sheet when HQ has stopped.
35, 36. **At 360 dp and font scale 2.0** the state wins the status line and wraps; sheet buttons stack
    from font scale 1.5; the sheet scrolls.

- **The sheet opens by itself at most once a day:** on the first stop of the day, never over a dialog or
  while the keyboard is up. Tapping the status line opens it any time.
- **Saved copy safety:** atomic writes (a temp file, then rename); at most 256 KB a place and 2 MB in all,
  cutting a place's list to its first rows; excluded from cloud backup and phone-to-phone copy; all
  cleared on Sign out everywhere.
- **TalkBack** announces only the change to HQ has stopped and the return to Connected, politely.
- **Links:** none change; a link opened while HQ is down opens its place with the saved copy.

## 7. Android versions

| Version | Behaviour |
|---------|-----------|
| 8 and 9 | `allowBackup="false"` (`AndroidManifest.xml:16`) already keeps the saved copy out of cloud backup, adb backup and phone-to-phone copy; no rules file is read. The default network callback (Android 7+) gives "validated". Notices that offer Undo stay 10 s. |
| 10 and 11 | Same backup behaviour as 8 and 9. Undo-type notices follow the phone's accessibility timeout. TalkBack announces "HQ has stopped" and the return to Connected, politely, as on every version. |
| 12 | The app targets SDK 35 (`app/build.gradle.kts:84`), so `allowBackup="false"` no longer stops phone-to-phone copy. A new `res/xml/data_extraction_rules.xml`, named in the manifest, excludes `savedcopy/`, the outbox and spoken ids from both cloud-backup and device-transfer. (Build it as `../wave-0/03-data-transfer.md` shapes it: an include list of `hqhome/.vault` only, which leaves these out.) |
| 13+ | With notifications off, the running notice is not in the shade; Android lists the app in its task manager only. The status line and the sheet are then the only signs of HQ's state. On 14+ the notice can be swiped away; the next state change posts it again. |
| 360 dp, font scale 2.0 | The state wins the status line and wraps (W3). Sheet buttons stack at font scale 1.5 and up, and the sheet scrolls (W2). The strip wraps to as many lines as it needs. |

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `K/data/HqServices.kt` | Replace `HqStatus` (lines 16-21) with a sealed `Connection`: `Connected`, `Starting(step, s, usual)`, `Restarting(s)`, `Stopped(since, step, reason)`, `Reconnecting(since)`, `ServerUnreachable(since)`, `ServerSignedOut`, `Offline(since)`, `AiNotAnswering(service, at)`, one `StateFlow`. The poll loop (lines 40-52) stays at 1 s and 5 s. | yes |
| `java/example/hq/spike/HubService.java` | `supervise()` (lines 219-237) already waits on the hq process; expose whether it is alive and the start step, so Stopped means the process ended. | yes |
| `K/data/HqClient.kt` | `healthy()` (line 49, reads `/health`) stays for liveness only; new `aiStatus()` reads `/api/status/ai` with the token. | yes |
| `K/data/NetworkWatch.kt` | The default network callback on `NET_CAPABILITY_VALIDATED`, debounced 3 s. | NEW |
| `K/data/SavedCopy.kt` | `SavedCopyStore`, one JSON file per place under `filesDir/savedcopy/`, each with the time HQ answered and its source (this phone or the server). Written with AtomicFile; 256 KB a place, 2 MB in all; `clearServer()` and `clearAll()`. | NEW |
| `K/ServerHq.kt` | `markSignedOut()` (line 52, the 401 path) and `disconnect()` (line 57) call `SavedCopyStore.clearServer()`; its poll result feeds the two server states. | yes |
| `res/xml/data_extraction_rules.xml`, manifest `android:dataExtractionRules` | As wave 0 builds it; `android:allowBackup="false"` stays for Android 11 and below. | NEW (wave 0) |
| `K/ui/connection/` | `ConnectionSheet.kt` (with FixStartup's `StartStep`), `OfflineStrip.kt`, `ChatBanner.kt`, `NothingYet.kt`, `TechnicalDetails.kt`, `Modifier.needsHq()`. Words are string resources: `fail_<action>_<reason>` from rules section 2. | NEW (package; FixStartup names `ui/shell/ConnectionSheet.kt`, see "Open conflicts") |
| `K/ui/shell/HqShell.kt` | The status text and colours (lines 113-115: green "Live", amber "Starting", amber "Saved") become the Connection words, grey, blue or amber; the title column is one button; `LocalReselect` and `ReselectEffect`. | yes (`LocalReselect`, `ReselectEffect` do NOT exist today; the board says "as before") |
| `K/ShellActivity.kt` | The "Saved" status (lines 299-300) goes; the sheet's daily auto-open is one preference. | yes |
| `K/ui/startup/Startup.kt` | "HQ did not start" (line 34) covers only the first launch (`FirstLaunch.markDone` in HqServices). | yes |
| Failure strings to replace | `AnswerReceiver.kt`, `capture/Outbox.kt`, `voice/VoiceTurn.kt` (lines 148-154), `ui/startup/Startup.kt`, `ui/chat/ChatViewModel.kt`, `NoteExporter.kt`, `ui/helpers/` (AnswerWords, HelperWords, HelpersScreen, HelpersViewModel), `ui/inbox/` (InboxFixtures, InboxScreen, InboxViewModel), `ui/notes/` (NotesHome, NotesPages, NotesViewModel), `ui/search/SearchWords.kt`, `ui/settings/` (SettingsScreens, SettingsViewModel, SettingsWords), `ui/tasks/` (Briefing, TaskFilterSheet, TasksScreen, TasksViewModel), `ui/webhost/` (WebHost, WebHostScreen). | yes |
| `K/ui/chat/ChatViewModel.kt` | Holds a message sent while offline as "Waiting to send" and sends it when the network is validated again. | yes |
| Fixtures for screenshots | `x-stopped`, `x-slow`, `x-full`, `x-restarting`, `x-offline`, `x-ai`, `x-server`, `x-signedout`, a "-copy" variant per place, and each at 360 dp and font scale 2.0. | NEW |

## 9. Server changes this needs

- **New `GET /api/status/ai`** in `crates/hq-web/src/api.rs`, routed in `crates/hq-web/src/lib.rs` beside
  `/api/vault-status` (line 266). It sits under `/api/`, so `web_path_is_guarded` (`auth.rs:122-127`)
  requires the web token or a device token. It returns, per AI service: `name`, `last_ok_at`,
  `last_fail_at` and `fail_kind` (`timeout`, `refused`, `rate_limited`, `out_of_credit`, `server_error`).
  No prompt or answer text.
- **`crates/hq-llm`:** the backend chain records the last good and the last failed call per backend, in
  memory, for that route.
- **`/health` stays as it is** (`lib.rs:265`, open by design, `auth.rs:121`): liveness only.
- **Lite** serves no chat turns, so the route is not added to `LITE_API` (`profile_gate.rs`) and the sheet
  leaves out the AI row there.
- **No change for server sign-out:** `ServerHq.kt` already treats a 401 as signed out.

## 10. Acceptance checks (each fails on today's build)

1. Slow start: a test vault whose open takes 70 s, usual start 40 s. At 70 s the status line still reads
   "Starting HQ" and the sheet says "HQ is still opening your notes. Restarting starts over."; "HQ has
   stopped" appears only at 80 s. Today the app covers itself at 60 s.
2. Dead process: kill hq with its restart blocked. Within 2 s the status line reads "HQ has stopped".
   Today it counts "Starting HQ" for 60 s.
3. Cold start with HQ unable to start and a copy present, after the first launch: no full-screen cover;
   Notes, Tasks, Helpers, Inbox and Settings show the copy with "Saved on this phone at {time}".
4. Stop HQ three times in one day: the sheet opens by itself only the first time.
5. Failure lines: a test renders every `fail_*` resource for each reason and matches the rules section 2
   catalogue. A source check over `app/src/main` (`ui/`, `voice/`, `data/`, `capture/`, `update/`,
   `AnswerReceiver.kt`) finds no "HQ is not answering", "HQ didn't reply", "HQ is not running", "HQ did
   not answer" or "Not answering", and no "exit code" outside TechnicalDetails. Today `VoiceTurn.kt:148`
   fails it.
6. Atomic writes: kill the app between write and rename 100 times; every next start shows either the old
   or the new copy, never an empty or broken list. A place over 256 KB keeps its first rows.
7. Data extraction: on two Android 12+ phones, copy phone A to phone B with Android's own phone-to-phone
   copy at setup. `savedcopy/`, the outbox and spoken ids are absent on B. Today `allowBackup="false"`
   does not stop this on 12+.
8. Server sign-out: sign the phone out on the server HQ (Settings, Devices). Within one poll the status
   line reads "Signed out of your server", and server items are gone from Inbox and from `savedcopy/`.
   Disconnect does the same.
9. AI status: with a revoked key on HQ, open the sheet before sending anything: it reads "The AI service
   isn't answering". `/api/status/ai` without a token answers 401; `/health` holds no AI fields. Today the
   route does not exist.
10. Airplane mode with HQ fine: "You're offline" within 3 s on Chat only; a message sent then goes when the
    network is back.
11. Screenshot tests at 360 dp and font scale 2.0 for every `x-` fixture: no status text is cut or ends in
    "…".
12. A fresh install still shows the first-launch "Starting HQ" screen; a second tap on a selected tab
    scrolls to the top, then reads again.

## 11. Check on device

- The saved-copy size on a 2,000-note vault (FixPlan wave 4 device check; N2 "measured first"; the board
  asks for it "before wave 0 ends").
- Repeat the data-extraction check with Samsung Smart Switch, and vendor copy tools (Tecno and Infinix
  clone apps), which may not follow Android's rules.
- On some 8.x phones a min-importance channel still shows an icon in the status bar.
- The almost-full `{size}` formula, measured on the A32.

## 12. Open questions and decisions it depends on

- **Hold chat messages sent while offline?** Rules section 2 says "It will send when you're back online",
  so the app must keep its word. Recommendation: yes, held in the chat for up to 24 hours, in order, each
  one removable, never sent twice. (Decided: N1.)
- **Size of the saved copy:** 256 KB a place and 2 MB in all. Recommendation: accept these numbers, then
  measure on a 2,000-note vault before wave 0 ends. (Decided: N2, "256 KB a place, 2 MB in all, measured
  first.")

Depends on: D16, D17, N1, N2, N15, rules sections 2, 3, 5, 6, 7, 10; `../wave-2/04-startup.md` (steps,
slow rule); `../wave-0/03-data-transfer.md` (rules file); `../wave-0/02-safe-mode.md` (stopped sheet's
Save a copy).
