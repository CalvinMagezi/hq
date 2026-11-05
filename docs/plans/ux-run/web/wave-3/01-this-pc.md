# Web wave 3 · 01 · HQ Hub: the window, its names and This PC (WB3, WB5, M1 to M4, M16)

Boards: **AlignedHub** ("This PC, part 1"), **AlignedHub2** ("This PC, part 2"), **AlignedHub3**
("sizes and what to build"), **AlignedFlow** ("The desktop hub and the HQ window"), AlignedReview WB3,
WB5, M16, and Drift desktop 01 to 05, 08 to 10. Follows `../../rules-web.md` sections 2, 5 and 6 (above
all) and `../../rules-shared.md` sections 3, 4 and 6. Where this spec and the rules differ, the rules win.

HQ Hub is the control window of the desktop app (`apps/desktop`, `crates/hq-desktop`). This spec covers
the window, its names and its first tab. Devices is `02-devices.md`; Helpers `03-helpers-tab.md` and
`04-helper-detail.md`; every command `05-commands-and-platforms.md`.

## 1. Finding and the review items it closes

- **Drift desktop 01.** Hub mode looks like every other mode: Start host runs only the coding agents
  (`hq host serve`), but the buttons say Start host and Stop host and the badge says Running.
- **Drift desktop 02.** In hub mode Open HQ waits on the wrong thing (a local host), and neither
  condition is explained beside the off button.
- **Drift desktop 03.** "Set the path in Settings" points nowhere; the path lives in
  `~/.hq/desktop/settings.json`.
- **Drift desktop 04.** The Open HQ note always talks about the phone.
- **Drift desktop 05.** Needs you and errors are bold only.
- **Drift desktop 08.** Exited points at a log file with no way to open it.
- **Drift desktop 09.** Two windows called HQ.
- **Drift desktop 10.** Two looks side by side: the control window in system styling, the HQ window in
  the dark glass web app.
- **WB3** (blocker, the window side; the proxy fix shipped in wave 0). **WB5** (blocker): "Allow helpers
  again" lives here, on that computer. **M16:** the WSL frame runs helpers only (`hq host serve`); the
  probe uses 2 s. **M1 to M4:** status words, colour, failure lines, names. **W4, W5, W38, W39, W43, W44,
  WR19 to WR22, WR25.**

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/desktop/src-tauri/tauri.conf.json`: `productName` "HQ", bundle target `app` (macOS),
  `shortDescription` "HQ on your Mac". `src-tauri/src/main.rs:23-24` control window 940 by 700 (minimum
  640 by 480), `:90` titled "HQ"; `:174-175` the HQ window, titled "HQ", 1200 by 800.
- `apps/desktop/ui/index.html:13-15` tabs Host, Devices, Claude; `:21-29` Host: "Start host", "Stop
  host", "Open HQ", the paragraph "The Start button runs HQ on this Mac. When this Mac works with the HQ
  on your phone, it starts only the coding agents here…" and the fixed note "Open HQ shows the HQ on your
  phone. The first time, it asks you to sign in…" (line 29); `:34` "The agents running on this Mac's
  host, the ones waiting for you first. Answer them in HQ."
- `apps/desktop/ui/src/main.ts:156-162` `HOST_LABEL`: Stopped, Starting, Running, Running elsewhere,
  Exited; `:168-169` "Started outside this app, so this app will not stop it." and "Ended by itself (code
  {n}). See ~/.hq/desktop/host.log."; `:174-177` Open HQ enabled only while a host answers; `:181-188`
  `NEED_LABEL` Needs you, Finished, Working, Idle, Running, Ended; `:204` the blocked badge is the grey
  pill with bold ("badge error"); `:113-117` `toast()` clears after 3000 ms.
- `crates/hq-desktop/src/desktop.rs:166-167` the missing-program message ends "Install HQ, or set the path
  to its hq program in Settings."; the window's only setting is the start checkbox.
- `crates/hq-desktop/src/host.rs:61-77` `HostSpec::for_config`: agents only (`hq host serve`) when the
  config serves a hub, else `hq start all`. No start step, no start limit, no usual start time.
- `crates/hq-desktop/src/settings.rs:23-32` `start_host_on_launch`, `proxy_port`, `bridge_port`, `relay`,
  `device_name`, `hq_path`.
- `apps/desktop/ui/style.css` system appearance, plain styling. `docs/DESKTOP.md:130-137` the program
  search (path in Settings, next to the app, `~/bin`, `/usr/local/bin`, `PATH`, then the login shell's
  `PATH`, five seconds; `hq --version` must answer within five seconds).
- The web app shows a lockdown with "Run `hq host unlock` on it" (`apps/hq-web/src/components/sessions/EmergencyStop.tsx:103, 192`).

### Before frames (page 1: DesktopHost, -Stopped, -Starting, -Elsewhere, -Exited, -NoHq, -Hub, DesktopWindows)

- Host running, an agent needs you: "Running", grey "Needs you" pill in bold, "Answer them in HQ."
- Stopped / Starting / Running elsewhere ("Started outside this app…") / Exited ("Ended by itself (code
  1). See ~/.hq/desktop/host.log.") / hq not found ("…set the path to its hq program in Settings.") /
  hub mode, identical to running.
- Two windows both titled "HQ" in the Window menu.

## 3. Decision

**One control window, "HQ Hub", inside the app named AgentHQ, built with `aligned-ui.css`, with three
tabs This PC · Devices · Helpers. This PC says in the rules' words what runs on this computer, starts
and stops it by progress, opens the log in a sheet, and is the only place that lifts "Stop every
helper".**

- **Names (rules-web 5):** the app is AgentHQ where the system shows its name (the Dock, Finder, the
  taskbar, the app list); its windows are **HQ Hub** (this one) and **HQ** (the web app). The first tab
  is named This PC on every system, a Mac too (W44, owner to confirm; reviewers suggested "This
  computer"). "This PC" is the tab's name only (`../../resolutions.md` B12): every line inside the
  tab says the machine's own name or "this computer".
- **Status line (rules-web 2):** the state first, then one fact: "Connected · 1 helper needs you". Grey
  when fine, blue while starting, amber for "HQ has stopped" (with a stop icon) or "Your HQ can't be
  reached". "You stopped HQ" and "HQ is off" are grey. No balance chip: Spending lives in HQ.
- **Outcomes stay** where they happened until the next change or until closed. Nothing clears after 3 s.
- **Dialogs** follow rules-shared 4. Red only on a button that stops or removes. Allowing helpers again
  is not red (WR25).
- **Rename plan (WR22, owner to confirm):** renaming the bundle moves HQ.app to AgentHQ.app. On first open
  after the rename HQ Hub rewrites only the Claude entries it added, by name and exact command, and says
  so once.

Rejected:

- **"Start host" in hub mode.** It hides which of the two this computer does.
- **A 60 s "did not start".** Stopped is judged by progress (rules-shared 6).
- **Lift the lockdown from the web.** A stolen sign-in could undo the stop (`docs/AGENT_HOST.md`, WB5).

## 4. Screens and states

The window: 940 by 700 of content under a 28 px title bar, in a neutral frame (each system draws its own
window buttons). Top: the HQ mark, title "HQ Hub", the status line under it. A segmented switch
(`.a-seg`) with **This PC · Devices · Helpers**; the Helpers tab carries an amber count of helpers that
need you (`.a-tab em.hub-need`). Content one column, 760 px wide at most, centred.

### This PC: HQ in every real state

- **P1 · HQ is off (first open).** Status "HQ is off" (grey). Card "HQ on this computer" / "Off" / "HQ Hub
  starts nothing by itself. Start HQ when you want to use it." [Start HQ] (green, the one primary) and
  [Open HQ] off with the why line "Open HQ needs HQ running. Start it first." Rows: "On this computer · Start HQ
  when HQ Hub opens" (a switch, off) with "HQ Hub never starts anything when you log in."; "The HQ
  program · ~/bin/hq · version 0.9.2" [Change…]; "What HQ printed · Nothing yet. HQ Hub keeps a log once
  it starts HQ." [Open the log] off.
- **P2 · Starting.** Status "Starting HQ · 12 s" (blue). Card "Starting" / "Step 3 of 4 · 12 s · Opening
  your notes and tasks"; one blue bar (4 px); the four steps the hub can see for itself: the process
  started, it is alive, its port answers, its health check passes, with the phone's names ("Starting on
  this computer", "Loading HQ", "Opening your notes and tasks", "Almost ready"). "Usually about 20 s on this
  PC." (learned from the last starts here). Open HQ off: "You can open HQ once it has started."
- **P3 · Slow but alive.** "HQ is still opening your notes. Restarting starts over." with [Restart] as the
  second button.
- **P4 · Connected, a helper needs you.** Status "Connected · 1 helper needs you" (grey dot). Tab
  "Helpers 1" in amber. Card "On" / "HQ Hub started HQ. It stops when you quit HQ Hub." [Stop HQ]
  (secondary) [Open HQ]. Row "4 helpers at work here · 1 needs your OK · open the Helpers tab" with the
  amber pill "Needs your OK". Log row "The log of the HQ that HQ Hub started" [Open the log].
- **P5 · The system notice** for Needs you on this computer: "AgentHQ · HQ Hub" / "Claude needs your OK" /
  "Open HQ to see what it wants to do." / [Open]. Open goes to that helper in the HQ window
  (`open_main_window("/helpers/{id}")`). The HQ window has no web push, so this is the only notice here.
- **P6 · Stop HQ asks first** (`../../resolutions.md` B9). With helpers working it is red: "Stop HQ?"
  / "Chats and scheduled tasks pause. {Codex and Claude} are working here and stop too. Their work so
  far is kept." / [Keep it running] [Stop HQ] (red). With no helper working it is the Ask kind:
  "Stop HQ?" / "Chats and scheduled tasks pause until you start it again." / [Keep it running] [Stop
  HQ] (green). Esc, the close button and a click outside mean Keep it running.
- **P7 · You stopped HQ.** Status "You stopped HQ" (grey). "You stopped HQ at 14:40. Start it again when
  you want." [Start HQ] (green). Open HQ off: "Open HQ needs HQ running. Start it first." Log row "Last
  run today, 14:02 to 14:40". The Helpers tab still checks which helpers are installed.
- **P8 · HQ has stopped by itself.** Status "HQ has stopped" (amber, stop icon). Card "Stopped" / "HQ
  stopped by itself at 14:02, 3 minutes after it started. Start HQ to try again." [Start HQ] [Copy
  details]. Technical details: "Exit code 1 at 14:02", "Log: ~/.hq/desktop/host.log". Open the log opens
  the last lines in a sheet inside HQ Hub, keys blanked. Copy details puts the version, the exit and the
  last lines on the clipboard, keys blanked. When the start step did not move: "Stopped at: Opening your
  notes and tasks".
- **P9 · Started outside HQ Hub.** Status "Connected" with the fact "· started outside HQ Hub" (grey).
  "HQ was started outside HQ Hub, for example in a terminal or as a service. HQ Hub won't stop it." Why
  line "To stop it, stop it where it was started." No Stop button at all. The helpers row still shows.
  Log row "HQ Hub keeps a log only for an HQ it started".
- **P10 · The HQ program not found, and a refused choice.** Status "HQ is off". Amber card "Couldn't find
  HQ on this computer" / tag "Not found" / "HQ Hub runs the HQ program, and it isn't in any place HQ Hub looks.
  Choose it, or install HQ first." [Choose the HQ program…] [Look again] [How to install HQ]. A refused
  choice stays under the buttons: "Couldn't use that program. Anyone on this computer can change its folder.
  Choose one that only you can change." Technical details: "Looked in: next to HQ Hub, ~/bin,
  /usr/local/bin, each folder on PATH, then your login shell's PATH." Once chosen, the full path shows
  once ("/Users/you/bin/hq"), then the short form.
- **P11 · The phone as the hub, helpers here off.** Status "Connected · HQ on your phone". Card "Helpers
  here, for HQ on your phone" / "Off" / "HQ runs on your phone, Pixel 8. This computer runs only the helpers it
  starts." [Start helpers here] [Open HQ]. A note that closes by hand: "First time in the HQ window? Sign
  in there with a code from your phone. On your phone, open Settings, then Other devices, then Sign in
  another device." (HQ Hub cannot learn that the HQ window signed in, so it does not guess.) Open HQ is
  off only with a reason: "Start HQ first" or "Switch on Devices to reach your phone".
- **P12 · The phone as the hub, helpers on.** Status "Connected · HQ on your phone · 2 helpers at work
  here". "2 helpers are working here for HQ on your phone." [Stop helpers here]. Row "2 helpers at work
  here · 2 working · open the Helpers tab" with "Working on it".
- **P13 · Stop helpers here asks first.** "Stop helpers here?" / "The 2 helpers working here stop now.
  Their work so far is kept. HQ on your phone keeps running." / [Keep them running] [Stop helpers here]
  (red). Different from Stop every helper: it stops only what runs on this computer, and helpers can start
  again here at once.
- **P14 · Another program holds the port (WB3).** Status "Your HQ can't be reached". Amber card "Couldn't
  open HQ on your phone" / "Port in use" / "Another program holds this port. HQ Hub won't send your sign-in
  there." / "Often it is another HQ pairing started in a terminal. Close it, then look again." Why line
  "Open HQ waits until HQ Hub holds the port itself." Technical details: "web forward to Pixel 8:
  127.0.0.1:47480, held by another program" and "gate on this computer: 127.0.0.1:7747, opened by HQ Hub". The
  Devices tab shows the same line on that row.
- **P15 · A lockdown on this computer (WB5).** Status "Connected · helpers here are stopped". Amber card "Every
  helper here is stopped" / tag "Stopped for safety" / "Stop every helper was used at 14:20, from Pixel
  8." / "Reason given: It keeps editing the wrong folder." (quoted as typed, from `lockdown.json`) / "No
  helper can start on this computer until you allow them again. Only HQ Hub on this computer can do that." [Allow
  helpers again] (green). The Helpers tab shows the same card on top, and Try it there says why it is
  off.
- **P16 · Allow helpers again?** "Allow helpers again?" / "Helpers can start on this computer again. The ones
  that were stopped stay stopped." / [Keep them stopped] [Allow helpers] (green, WR25). Done: a toast
  "Helpers allowed again on this computer." that stays until closed. No Undo: lock again with Stop every
  helper. Stopped helpers do not come back by themselves; resume each in HQ.
- **P17 · Windows: helpers in Ubuntu (future scope, W50).** Card "Helpers here, in Ubuntu" / "On
  Windows, helpers run inside Ubuntu (WSL2) on this computer. HQ itself runs on your phone." / "Ubuntu:
  Ubuntu-24.04 (WSL2)" / "Started with: C:\Windows\System32\wsl.exe -d Ubuntu-24.04 -- ~/bin/hq host
  serve" / "All of HQ in Ubuntu instead: ~/bin/hq start all" / "HQ Hub never starts anything when you
  sign in to Windows." / "In Ubuntu · ~/bin/hq · version 0.9.2". With all of HQ in Ubuntu the card reads
  "HQ on this computer, in Ubuntu" and HQ Hub runs `hq start all` (`docs/WINDOWS.md`).
- **P18 · Restarting HQ (added by resolutions.md C-12).** Status "Restarting HQ" (blue). Card
  "Restarting" / "Step {1} of {4} · {Starting on this computer}" with the one blue bar; Open HQ off with
  "You can open HQ once it has started." If the start step stops moving, P8 by A7's rule.
- **P19 · The Connection sheet (added by resolutions.md C-12).** Choosing the status line opens a sheet
  with the four start steps, "Usually about {20} s", "Stopped at: {step}" when it stopped, and "This
  computer is almost full" (under 200 MB free on the disk that holds `~/.hq`, N15) with "Free up {size}
  and try again." The sheet opens by itself at most once a day, only for HQ has stopped or almost full.

### Sizes (AlignedHub3)

- **320 px:** the top bar holds the title and the state, which wraps onto two lines; the three tabs stay
  tabs; buttons wrap, each at least 44 px tall. Toasts never cover the focused field.
- **200% text:** the window's 940 px act as 470, so it takes the same one-column layout; tabs full
  width; a long state wraps.
- **Maximised on a 2200 px screen:** the content stays one 760 px column, centred; the tabs sit together
  on the left instead of stretching.
- **Lite:** HQ Hub is not part of HQ Lite (no helpers, no host, no pairing). Lite opens in the browser
  and starts on Notes.

## 5. Exact copy

| Key | String |
|-----|--------|
| `hub_title` | "HQ Hub" |
| `hub_tabs` | "This PC" · "Devices" · "Helpers" |
| `hub_off` | "HQ is off" · "HQ on this computer" · "Off" · "HQ Hub starts nothing by itself. Start HQ when you want to use it." |
| `hub_start` | "Start HQ" |
| `hub_open` | "Open HQ" |
| `hub_open_why_off` | "Open HQ needs HQ running. Start it first." |
| `hub_open_why_starting` | "You can open HQ once it has started." |
| `hub_open_why_phone` | "Start HQ first" · "Switch on Devices to reach your phone" |
| `hub_launch_row` | "On this computer" · "Start HQ when HQ Hub opens" · "HQ Hub never starts anything when you log in." |
| `hub_program_row` | "The HQ program" · "{~/bin/hq} · version {0.9.2}" · "Change…" |
| `hub_log_row` | "What HQ printed" · "Nothing yet. HQ Hub keeps a log once it starts HQ." / "The log of the HQ that HQ Hub started" / "HQ Hub keeps a log only for an HQ it started" / "Last run today, {14:02} to {14:40}" · "Open the log" |
| `hub_starting` | "Starting HQ · {12} s" · "Starting" · "Step {3} of {4} · {12} s · {Opening your notes and tasks}" · "Usually about {20} s on this computer." |
| `hub_slow` | "HQ is still opening your notes. Restarting starts over." · "Restart" |
| `hub_on` | "On" · "HQ Hub started HQ. It stops when you quit HQ Hub." |
| `hub_helpers_row` | "{4} helpers at work here" · "{1} needs your OK · open the Helpers tab" |
| `hub_stop_dialog_working` | "Stop HQ?" · "Chats and scheduled tasks pause. {Codex and Claude} are working here and stop too. Their work so far is kept." · "Keep it running" · red "Stop HQ" (B9) |
| `hub_stop_dialog_idle` | "Stop HQ?" · "Chats and scheduled tasks pause until you start it again." · "Keep it running" · green "Stop HQ" (B9) |
| `hub_restarting` | "Restarting HQ" · "Restarting" (C-12) |
| `hub_almost_full` | "This computer is almost full." · "Free up {size} and try again." (C-12) |
| `hub_you_stopped` | "You stopped HQ" · "You stopped HQ at {14:40}. Start it again when you want." |
| `hub_stopped` | "HQ has stopped" · "Stopped" · "HQ stopped by itself at {14:02}, {3} minutes after it started. Start HQ to try again." · "Copy details" · "Exit code {1} at {14:02}" · "Log: ~/.hq/desktop/host.log" · "Stopped at: {step}" |
| `hub_external` | "HQ was started outside HQ Hub, for example in a terminal or as a service. HQ Hub won't stop it." · "To stop it, stop it where it was started." |
| `hub_not_found` | "Couldn't find HQ on this computer" · "Not found" · "HQ Hub runs the HQ program, and it isn't in any place HQ Hub looks. Choose it, or install HQ first." · "Choose the HQ program…" · "Look again" · "How to install HQ" |
| `hub_refused_program` | "Couldn't use that program. Anyone on this computer can change its folder. Choose one that only you can change." |
| `hub_looked_in` | "Looked in: next to HQ Hub, ~/bin, /usr/local/bin, each folder on PATH, then your login shell's PATH." |
| `hub_phone_off` | "Helpers here, for HQ on your phone" · "Off" · "HQ runs on your phone, {Pixel 8}. This computer runs only the helpers it starts." · "Start helpers here" |
| `hub_phone_on` | "{2} helpers are working here for HQ on your phone." · "Stop helpers here" |
| `hub_first_signin` | "First time in the HQ window?" · "Sign in there with a code from your phone. On your phone, open Settings, then Other devices, then Sign in another device." |
| `hub_stop_here_dialog` | "Stop helpers here?" · "The {2} helpers working here stop now. Their work so far is kept. HQ on your phone keeps running." · "Keep them running" · "Stop helpers here" |
| `hub_port_held` | "Couldn't open HQ on your phone" · "Port in use" · "Another program holds this port. HQ Hub won't send your sign-in there." · "Often it is another HQ pairing started in a terminal. Close it, then look again." · "Open HQ waits until HQ Hub holds the port itself." |
| `hub_lockdown` | "Every helper here is stopped" · "Stopped for safety" · "Stop every helper was used at {14:20}, from {Pixel 8}." · "Reason given: {text}" · "No helper can start on this computer until you allow them again. Only HQ Hub on this computer can do that." · "Allow helpers again" |
| `hub_allow_dialog` | "Allow helpers again?" · "Helpers can start on this computer again. The ones that were stopped stay stopped." · "Keep them stopped" · "Allow helpers" |
| `hub_allowed_done` | "Helpers allowed again on this computer." |
| `hub_notice` | "AgentHQ · HQ Hub" · "{Claude} needs your OK" · "Open HQ to see what it wants to do." · "Open" |
| `hub_ubuntu` | "Helpers here, in Ubuntu" · "On Windows, helpers run inside Ubuntu (WSL2) on this computer. HQ itself runs on your phone." · "Ubuntu: {Ubuntu-24.04} (WSL2)" · "Started with: {cmd}" · "All of HQ in Ubuntu instead: ~/bin/hq start all" · "HQ Hub never starts anything when you sign in to Windows." |

Retired: "Start host", "Stop host", "Running", "Running elsewhere", "Exited", "Stopped" (as a status
word), "Ended by itself (code {n}). See ~/.hq/desktop/host.log.", "Started by this app (process {pid}).",
"Answer them in HQ.", "Open HQ shows the HQ on your phone…", "…set the path to its hq program in
Settings.", "No host answers on this Mac.", the titles "HQ" for the control window.

## 6. Behaviour rules

1. **The state first.** `host_status` returns the state, the start step and the facts; the window builds
   the status line from it: off › starting › stopped (by progress) › you stopped › connected (with the
   first true fact: helpers that need you, then helpers at work, then "started outside HQ Hub", then "HQ
   on your phone").
2. **Stopped by progress** (`../../resolutions.md` A7). "HQ has stopped" only when the process ended,
   or its start step has not moved for the longer of 60 s and twice its usual start time. No fixed
   timer, the same rule as the phone. While the process is alive and slow, P3.
3. **Usual start time** is learned from the last starts on this computer and saved in settings
   (Recommended: the median of the last five; the board says only "learned from the last starts here").
4. **Hub mode is named.** When `config.bridge.serves_hub()`, the card, the buttons and the status line
   say what this computer does (P11, P12); `HostSpec::for_agents_only` runs `hq host serve`.
5. **Open HQ is off only with a reason** next to it: "Open HQ needs HQ running. Start it first.", "You
   can open HQ once it has started.", "Start HQ first", "Switch on Devices to reach your phone", or the
   held-port line.
6. **Outcomes stay.** Results live on their row or card until the next change; toasts have a close
   button and do not time out.
7. **Needs you is amber.** Every "Needs your OK" pill and the tab count are amber (`.a-state.needs`,
   `.a-tab em.hub-need`); errors are amber with an icon.
8. **Lockdown is lifted here only.** [Allow helpers again] runs `host_unlock`, which runs `hq host unlock`
   on this computer; no web page can call it (the HQ window has no capability).
9. **The system notice** is posted from Rust when a helper on this computer starts needing the owner;
   words are rules-web 1; Open is the only action; it opens `/helpers/{id}` in the HQ window.
10. **The log** opens in a sheet with the last lines (Recommended: 200), keys blanked (anything matching the secret
    patterns in `crates/hq-agent/src/governance/secrets.rs` becomes "[hidden]"); Copy details adds the
    version and the exit.
11. **The two windows** are titled "HQ Hub" and "HQ"; the HQ window's title adds " · HQ on your phone"
    when it shows the phone's HQ.
12. **One look.** The control window loads `aligned-ui.css` and bundled Clear Sans (the CSP allows
    `style-src 'self'` and `font-src 'self'`); it is dark like the HQ window.
13. **Stop HQ's kind follows the helpers (B9).** `host_status` reports the helpers working on this
    computer; with one or more the dialog is red and names them, with none it is the Ask kind with a
    green "Stop HQ".
14. **Restarting and the sheet (added by resolutions.md C-12).** A restart HQ Hub started shows
    "Restarting HQ" until the health check passes. The Connection sheet opens by itself at most once a
    day (the day kept in `settings.json`); "almost full" uses N15's 200 MB, measured where `~/.hq` lives.

## 7. Browser and platform notes

- **macOS:** the bundle is "AgentHQ.app" after the rename (WR22); the Dock and the menu bar show
  AgentHQ. The Window menu lists "HQ Hub" and "HQ". Notices go through the system's notification centre
  from Rust (no Tauri plugin; the no-plugin test in `crates/hq-desktop/tests/shell_config.rs:183`).
- **Windows and Linux (future scope, W50):** the hub builds for macOS only today (`tauri.conf.json`
  `targets: ["app"]`). P17 is drawn for later; `wsl.exe` by its full path, `open_in_browser` needs its own
  opener (it calls `/usr/bin/open`, `apps/desktop/src-tauri/src/commands.rs:94`).
- The control window is a WKWebView on macOS: `<dialog>`, container queries and `:focus-visible` work.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/desktop/src-tauri/tauri.conf.json` | `productName` "AgentHQ"; `shortDescription` "HQ on this computer". | yes |
| `apps/desktop/src-tauri/src/main.rs` | Titles "HQ Hub" (90) and "HQ" (174); the HQ window title fact. | yes |
| `apps/desktop/ui/index.html`, `ui/src/main.ts`, `ui/style.css` | Rebuild with `aligned-ui.css`: tabs This PC, Devices, Helpers; P1 to P19; toasts with close (113-117); labels (156-188). | yes |
| `apps/desktop/scripts/build-ui.ts` | Copy `aligned-ui.css` and the fonts into `ui/dist`. | yes |
| `crates/hq-desktop/src/host.rs` | Start steps, usual start time, stopped by progress, the last run's times (151-245); the restarting state and free space under `~/.hq` (C-12). | yes |
| `crates/hq-desktop/src/desktop.rs` | `hq_problem` words (166-167); `host_log`; `host_unlock`; the notice; the rename rewrite of Claude entries. | yes |
| `crates/hq-desktop/src/settings.rs` | `usual_start_secs`, `look_for_helper_updates` (off), `try_it_dir`. | yes |
| `crates/hq-desktop/src/agents.rs` | Helper display names and states for the summary row. | yes |
| `apps/desktop/src-tauri/src/commands.rs`, `build.rs`, `capabilities/control.json` | The new commands (`05-commands-and-platforms.md`). | yes |

## 9. Server, desktop or release changes

- Desktop: every command in `05-commands-and-platforms.md` that this tab uses: `host_status` (more
  fields), `host_log`, `pick_program`, `host_unlock`, `open_main_window(path)`, the native notice.
- Server: the host's lockdown record carries who, when and the reason (`lockdown.json` already holds
  them; `host_status` reads it).
- Release: the bundle rename plan (WR22); `docs/DESKTOP.md` and `docs/security/DESKTOP.md` updated.

## 10. Acceptance checks (each fails on today's build)

1. The control window's title is "HQ Hub" and the bundle's `productName` is "AgentHQ"; `shell_config.rs`
   asserts it. Today both are "HQ".
2. With a hub config, the card reads "Helpers here, for HQ on your phone" and the button "Start helpers
   here". Today it reads "Start host".
3. A host that exits by itself shows "HQ has stopped" and a working [Open the log] that opens a sheet with
   the last lines. Today it shows "Exited" and a file path.
4. A start that has not moved for twice the usual time shows "Stopped at: {step}"; a slow alive one shows
   "HQ is still opening your notes. Restarting starts over." Today there are no steps.
5. Stop HQ with a helper working opens the red dialog naming it; with none working it opens the Ask
   dialog with a green "Stop HQ" and "Chats and scheduled tasks pause until you start it again."; Esc
   keeps it running (B9). Today Stop host acts at once.
6. A lockdown shows the card with who, when and the reason, and Allow helpers again runs `hq host unlock`
   (a test with a fake `hq`). Today the window has no lockdown state.
7. The pairing result toast stays after 3 s. Today it clears at 3000 ms.
8. A restart started from HQ Hub shows "Restarting HQ" in blue; with under 200 MB free the sheet says
   "This computer is almost full."; the sheet opens by itself once, not again the same day (added by
   resolutions.md C-12). Today none of these exist.

## 11. Check on a laptop or phone

- macOS: open AgentHQ from the Dock with no `hq` installed: P10; choose a program in `/tmp` (world
  writable): the refusal. Install HQ, Look again: P1.
- Start, wait, Stop: P2, P4, P6, P7. Kill the `hq` process from a terminal: P8.
- Pair a phone as the hub (`docs/PAIRING.md`): P11, P12. Start `hq bridge serve` in a terminal first:
  P14.
- From the phone, Stop every helper on this computer: P15; Allow helpers again: P16.

## 12. Open decisions it depends on

- **W44** ("This PC" on Macs), **W50** (Windows and Linux builds), **WR22** (the rename): owner to confirm.
- **W4, W5, W38, W39 (as `../../resolutions.md` B9 settles it), W43, WR19, WR20, WR21, WR25**
  (Recommended), and `../../resolutions.md` A7, B12, C-12.
- Depends on: `../wave-0/01-live-bugs.md` (the proxy), `05-commands-and-platforms.md`,
  `../wave-1/02-dialogs-approvals.md`, `../wave-0/02-shared-parts.md` (status words).
