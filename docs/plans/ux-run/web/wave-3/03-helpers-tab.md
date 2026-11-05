# Web wave 3 · 03 · HQ Hub: the Helpers tab (M6, W26, W47, WB5)

Boards: **AlignedHubHelpers** ("Helpers"), **AlignedHubHelpers2** ("Helpers, part 2": no helpers,
Windows, the phone as the hub, what exists today), AlignedReview M6, M16, W26, W47, WB5. Follows
`../../rules-web.md` sections 5 and 6 (6.5 Try it, 6.6 helper checks) and `../../rules-shared.md`
section 3. Where this spec and the rules differ, the rules win.

The tab replaces today's Claude tab. One helper's detail and Try it are `04-helper-detail.md`.

## 1. Finding and the review items it closes

- **M6 Helper checks run unknown programs, unsandboxed.** Up to 12 processes on each tab open;
  user-writable folders; WSL appends Windows PATH. Read versions from package files; else sandboxed, no
  network, two at a time, cached by path and date; show the full path once.
- **M16.** The check limit is 2 s, as the machine probe (the old board said 5 s).
- **W47.** Claude Desktop in "Apps that use HQ". Eleven helpers and one app, the same count everywhere.
- **W26, as `../../resolutions.md` B4 words it.** The edge worker's runs read "HQ is working on it"
  while working and "Done by HQ" when finished, in the group "HQ's own work", never a helper.
- **W06.** Title-link rows with the button beside the link.
- **WB5.** A lockdown state that points to This PC.
- **W42** (helpers at work move from This PC to this tab), **W45** (update check behind a switch, off),
  **W46** (no Connect to HQ for Hermes and OpenClaw), **W49** (install lines ship with HQ and are
  checked each release).

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/desktop/ui/index.html:73-80` the Claude tab: one row per paired hub with Add and Remove
  (`ui/src/main.ts:400-420`, "Added. Restart Claude to see it.", "Removed.", in a 3 s toast).
- `crates/hq-desktop/src/claude.rs` and `crates/hq-bridge/src/claude.rs`: adding a paired HQ to Claude
  Code and Claude Desktop (`hq claude add`); `crates/hq-cli/src/commands/claude.rs:12-17`.
- `crates/hq-desktop/src/agents.rs`: the hub's list of helpers at work (from the host), shown on the Host
  tab (`main.ts:195-210`).
- `crates/hq-tools/src/harness_session/spec.rs`: the eleven helpers and their programs (lines 120 to
  268), `trust_pattern: Some("trust the contents")` only for Antigravity (`agy`, line 218), `one_shot` for
  Hermes (line 251, `chat --oneshot -q {brief}`) and OpenClaw (line 268, `agent --local --message
  {brief}`); Codex starts with its update check off.
- `crates/hq-tools/src/harness_session/category.rs` coding or general; `preflight.rs` the program search
  before a helper starts.
- `crates/hq-cli/src/commands/mcp.rs` `hq mcp install|remove|status` with targets.
- No version reads, no checks, no install lines in the hub today.

### What exists today, and what this design needs built (AlignedHubHelpers2)

| Helper | Program | Found today | Version read today | Connect to HQ today | State rules today | Trust prompt HQ answers |
|--------|---------|-------------|--------------------|---------------------|-------------------|-------------------------|
| Claude Code | claude | yes | no | yes: `hq mcp install --target claude-code`. Paired HQ: `hq claude add` | yes | no |
| Codex | codex | yes | no | no (needs building) | yes | no |
| Cursor | cursor-agent | yes | no | yes: `--target cursor` | no | no |
| Pi | pi | yes | no | no (needs building) | yes | no |
| OpenCode | opencode | yes | no | yes: `--target opencode` | no | no |
| GitHub Copilot | copilot | yes | no | yes: `--target copilot` | yes | no |
| Kimi | kimi | yes | no | no (needs building) | no | no |
| Qwen Code | qwen | yes | no | no (needs building) | no | no |
| Antigravity | agy | yes | no | yes: `--target antigravity` | yes | yes: its folder trust prompt |
| Hermes | hermes | yes | no | not offered: HQ never connects itself into a general helper | no | no |
| OpenClaw | openclaw | yes | no | not offered, same rule | no | no |
| Claude Desktop (app) | not a program on PATH | no | no | yes: `--target claude-desktop`. Paired HQ: `hq claude add` | not started by HQ | not started by HQ |

"Found today" means HQ's launch check looks for the program before it starts a helper (`preflight.rs`).
"State rules" are the host's screen rules that tell ready from working from asking (`crates/hq-host`
detect manifests). "Trust prompt HQ answers" is `spec.rs` `trust_pattern`.

**Exists today:** the eleven helpers and their programs (`spec.rs`), the coding or general split
(`category.rs`), the program search (`preflight.rs`), a chosen program as a launch profile, the hub's
list of helpers at work (`agents.rs`), `hq mcp install`, `remove` and `status`, and connecting a paired
HQ to Claude Code and Claude Desktop (hq-bridge `claude.rs`). Codex starts with its update check off.

**Needs building:** a helper list the hub can read (program, display name, package file, install line,
sign-in site, connect target); package-file version reads, then sandboxed version runs with a 2 s limit,
two at a time, cached; Update available, behind a switch that is off; a settings-found check; Try it;
Codex, Pi, Kimi and Qwen connections; checks through Ubuntu; the hub on Windows and Linux.

**Install lines to confirm:** the repo names no install line for any helper. The board uses each
project's published line as an example. Antigravity, Hermes and OpenClaw lines must come from their own
pages when the list is built, and the list kept current like any other content (W49).

## 3. Decision

**One list of every helper HQ can use on this computer, and the apps that use HQ, each with its version, its
state in words and one next action. Checked, not watched: when the tab opens and on Check all again,
never on a timer, and only what changed since.** The tab answers three questions without a terminal:
which helpers are on this computer, does each one work, and can it reach HQ.

Rejected:

- **Run each program's `--version` on every open.** That ran unknown programs from user-writable folders
  with the network on (M6).
- **Install or update a helper from HQ Hub.** HQ Hub never runs an installer itself: it shows the line,
  the owner runs it, then checks again.

## 4. Screens and states

Same window and frame as This PC. Top line: "Every helper HQ can use here, its version, and what it needs
from you." and "Checked at 14:30, when you opened this tab." [Check all again].

- **H1 · At work here** (`.a-grp` "At work here 4"): each row opens that helper in the HQ window at
  `/helpers/:id`; Needs your OK first. "Fix the flaky relay test · Claude Code · started 40 min ago ·
  Needs your OK · Opens in HQ"; "Review the update installer · Codex · started 12 min ago · Working on it";
  "Sort the release notes · HQ is working on it · started 5 min ago" (HQ's own runs, never a helper, not
  counted; once finished "Done by HQ", `../../resolutions.md` B4);
  "Docs pass · Codex · finished 1 h ago · Finished".
- **H2 · Coding helpers 9.** The five that need you first, then Ready, then Not installed. Each row: the
  name as a link (opens the detail), its version or why there is none, one state pill, one button beside
  the link:
  - "Antigravity · 1.4.0 · not connected to HQ yet · Not connected to HQ" [Connect to HQ]
  - "Qwen Code · 0.2.1 · its sign-in screen showed · Needs sign-in" [Sign in…]
  - "Cursor · Its last Try it stopped on a question · Needs you on this computer" [See what it asked]
  - "Kimi · Its settings are here, its program is not · Program not found" [Choose the program…]
  - "OpenCode · Couldn't read its version in 2 s · Check failed" [Check again]
  - "Codex · 0.58.0 · 0.60.1 is out · Update available" [How to update] (grey)
  - "Claude Code · 2.1.12 · connected to HQ · Ready" [Try it]
  - "Pi · 0.12.3 · Ready" [Try it]
  - "GitHub Copilot · Not on this computer · Not installed" [How to install] (grey)
- **H3 · General helpers 2.** "They research, review and watch other helpers. HQ never gives them code
  changes." "OpenClaw · 2026.10.2", "Hermes".
- **H4 · Apps that use HQ 1.** "Apps you open yourself that can use HQ. HQ does not start them." "Claude
  Desktop · App · 1.0.9 · not connected to HQ yet" [Connect to HQ]. No Try it.
- **H5 · Checking.** "Checking 11 helpers and 1 app · 5 done" / "Two at a time. Most versions come from
  the program's own files." Rows: "1.4.0 · new here: /Users/alex/.local/bin/agy", "Reading its version ·
  Checking" (blue, spinner), "Looking for its program · Waiting its turn · Waiting" (grey).
- **H6 · First open: ask before running anything.** "Check this computer for helpers" / "HQ Hub looks for each
  helper HQ can use and reads its version. Most versions come from the program's own files." / "Other
  checks run the program once, closed off, with no network. Nothing is installed." [Check this computer]. With
  HQ off: "HQ is off, so no helpers are at work here." Checking does not need HQ running; only Try it
  does.
- **H7 · During a lockdown.** The amber card "Every helper here is stopped" / "Stopped for safety" / "Stop
  every helper was used at 14:20, from Pixel 8. Allow them again on This PC." [Go to This PC]. Stopped
  helpers "Claude Code · stopped at 14:20 · Stopped" (grey). "Try it waits until helpers are allowed
  again on This PC." Checking versions still works.
- **H8 · No helpers on this computer.** "No helpers on this computer yet" / "Coding work needs a helper. Most people
  start with Claude Code. Copy this line into a terminal, sign in once, then check again." /
  `npm install -g @anthropic-ai/claude-code` [Copy] / "It needs Node.js. Every helper's own install line
  is in its row below." Every row Not installed with How to install.
- **H9 · Windows: helpers in Ubuntu (future scope).** Status "Connected · HQ on your phone". Card "On
  Windows, helpers live in Ubuntu" / "Helpers run inside Ubuntu (WSL2) here, so HQ uses only helpers
  installed there. Install each one in Ubuntu, and sign in there." [How helpers work on Windows]. A helper
  found in Windows but not in Ubuntu: "In Windows, not in Ubuntu · Program not found" (amber). "Antigravity
  · 1.4.0 · in Ubuntu · Ready" [Try it]; "Pi · 0.12.3 · in Ubuntu". "Claude Desktop · App in Windows ·
  1.0.9 · Not connected to HQ" [Connect to HQ] (it runs in Windows, and Connect to HQ makes it start HQ
  inside Ubuntu, needs building).
- **H10 · The phone as the hub.** Card "Helpers here work for HQ on your phone" / "Off" / "HQ runs on
  Pixel 8. It starts these helpers here once you press Start helpers here on This PC. Connect to HQ
  connects a helper to your phone's HQ." "At work here · Helpers your phone starts here show up once
  helpers here are on." Rows "2.1.12 · connected to your phone's HQ", "1.0.105 · not connected to your
  phone's HQ", "0.12.3". "Try it needs helpers on here. Start them on This PC first." Needs building: the
  phone learns which helpers this computer has, so its Start a helper offers only those.
- **Sizes.** 320 px: rows wrap, the button goes under the name; 200% text the same; 2200 one 760 column.

### Colour (rules-shared 3)

Grey for fine or nothing to do (Ready, Update available, Not installed, Finished, Stopped, Waiting).
Amber with words for needs you (Not connected to HQ, Needs sign-in, Needs you on this computer, Program not
found, Check failed). Blue with a spinner for working (Checking, Trying it, Working on it). Red only in a
remove dialog. The tab count is amber and counts only helpers at work that need your OK; a helper that
needs setting up is amber in the list but does not raise the count.

## 5. Exact copy

| Key | String |
|-----|--------|
| `hh_intro` | "Every helper HQ can use here, its version, and what it needs from you." |
| `hh_checked` | "Checked at {14:30}, when you opened this tab." / "Checked at {14:30}." |
| `hh_check_all` | "Check all again" |
| `hh_group_work` | "At work here" |
| `hh_group_coding` | "Coding helpers" |
| `hh_group_general` | "General helpers" · "They research, review and watch other helpers. HQ never gives them code changes." |
| `hh_group_apps` | "Apps that use HQ" · "Apps you open yourself that can use HQ. HQ does not start them." |
| `hh_started` | "{Claude Code} · started {40 min ago}" · "Opens in HQ" |
| `hh_hq_own` | "HQ is working on it · started {5 min ago}" while working · "Done by HQ · finished {1 h ago}" when finished (B4) |
| `hh_states` | "Not connected to HQ" · "Needs sign-in" · "Needs you on this computer" · "Program not found" · "Check failed" · "Update available" · "Ready" · "Not installed" · "Checking" · "Waiting" · "Trying it" |
| `hh_row_lines` | "{1.4.0} · not connected to HQ yet" · "{0.2.1} · its sign-in screen showed" · "Its last Try it stopped on a question" · "Its settings are here, its program is not" · "Couldn't read its version in 2 s" · "{0.58.0} · {0.60.1} is out" · "{2.1.12} · connected to HQ" · "Not on this computer" · "Reading its version" · "Looking for its program" · "Waiting its turn" · "new here: {full path}" |
| `hh_actions` | "Connect to HQ" · "Sign in…" · "See what it asked" · "Choose the program…" · "Check again" · "How to update" · "Try it" · "How to install" |
| `hh_checking` | "Checking {11} helpers and {1} app · {5} done" · "Two at a time. Most versions come from the program's own files." |
| `hh_first_open` | "Check this computer for helpers" · "HQ Hub looks for each helper HQ can use and reads its version. Most versions come from the program's own files." · "Other checks run the program once, closed off, with no network. Nothing is installed." · "Check this computer" · "HQ is off, so no helpers are at work here." |
| `hh_lockdown` | "Stop every helper was used at {14:20}, from {Pixel 8}. Allow them again on This PC." · "Go to This PC" · "{Claude Code} · stopped at {14:20}" · "Try it waits until helpers are allowed again on This PC." |
| `hh_none` | "No helpers on this computer yet" · "Coding work needs a helper. Most people start with Claude Code. Copy this line into a terminal, sign in once, then check again." · "It needs Node.js. Every helper's own install line is in its row below." · "Copy" |
| `hh_windows` | "On Windows, helpers live in Ubuntu" · "Helpers run inside Ubuntu (WSL2) here, so HQ uses only helpers installed there. Install each one in Ubuntu, and sign in there." · "How helpers work on Windows" · "In Windows, not in Ubuntu" · "{1.4.0} · in Ubuntu" · "App in Windows · {1.0.9}" |
| `hh_phone` | "Helpers here work for HQ on your phone" · "HQ runs on {Pixel 8}. It starts these helpers here once you press Start helpers here on This PC. Connect to HQ connects a helper to your phone's HQ." · "Helpers your phone starts here show up once helpers here are on." · "{2.1.12} · connected to your phone's HQ" · "{1.0.105} · not connected to your phone's HQ" · "Try it needs helpers on here. Start them on This PC first." |

## 6. Behaviour rules

1. **Version from the package file first.** Read `package.json` for npm installs (for example
   `@anthropic-ai/claude-code/package.json` beside the program's link), the binary's own metadata where it
   has one; nothing runs.
2. **Otherwise one sandboxed run** of the program's version line: no network, nothing typed into it
   (stdin closed), a 2 s limit (as `hq-core`'s machine probe), in the same sandbox real work uses
   (`crates/hq-sandbox`). Two at a time; a waiting one says so.
3. **Cached by path, size and date** (modification time): opening the tab again runs nothing that has not
   changed. Check all again clears the cache.
4. **A program seen for the first time** shows its full path once: "new here: /Users/alex/.local/bin/agy".
5. **Refuse world-writable places** for any program HQ Hub would run or keep as a launch command.
6. **On Windows,** checks run inside Ubuntu through `wsl.exe` by its full path and skip the Windows folders
   WSL adds to PATH (no `/mnt` paths).
7. **Update available** is looked up only with the "Look for helper updates" setting on (off by default,
   W45): it reaches the internet.
8. **Row order:** at work first (Needs your OK first); in each group, rows that need you, then Ready, then
   Not installed. Each row shows the first state that applies (`04-helper-detail.md` state table); the
   detail shows all.
9. **Title-link rows.** The name is the link to the detail; the one button sits beside it, never inside.
10. **The count everywhere is eleven helpers and one app,** in three groups: Coding helpers (9), General
    helpers (2) and Apps that use HQ (1).
11. **HQ's own runs** (from `/api/edge-worker`) show as "HQ is working on it" in At work here, and as
    "Done by HQ" once finished; they are never counted as helpers (B4).
12. **Lockdown:** Try it is off with the reason; checking still works (it starts no helper through the
    host).

## 7. Browser and platform notes

- macOS: programs under `~/.local/bin`, `~/.npm-global/bin`, `/opt/homebrew/bin`; the search list in
  `04-helper-detail.md`. A Dock-launched app has the system's short PATH; the login shell's PATH is read
  as the last step (`docs/DESKTOP.md`).
- Windows and Linux: future scope (W50).

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `crates/hq-desktop/src/helpers.rs` | The helper list, checks, cache, states. | NEW |
| `crates/hq-tools/src/harness_session/spec.rs` | Add display name, package file, install line, sign-in site, connect target per helper (or a sibling table the hub reads). | yes |
| `crates/hq-tools/src/harness_session/preflight.rs` | Reused for "Found today". | yes |
| `crates/hq-desktop/src/agents.rs` | At work here rows with display names. | yes |
| `apps/desktop/ui/index.html`, `ui/src/main.ts` | The Helpers tab (replaces the Claude tab, 73-80 and 400-420). | yes |
| `apps/desktop/src-tauri/src/commands.rs`, `build.rs`, `capabilities/control.json` | `helpers_list`, `helpers_check`; `claude_status` and `claude_change` fold into `helper_connect` and `helper_disconnect`. | yes |

## 9. Server, desktop or release changes

- Desktop: `helpers_list`, `helpers_check` (`05-commands-and-platforms.md`); the helper list data.
- Release: install lines checked in the release checklist (`docs/security/RELEASE_CHECKLIST.md` gains a
  line; W49).

## 10. Acceptance checks (each fails on today's build)

1. `cargo test -p hq-desktop helpers`: a fake `claude` with a `package.json` reports its version with no
   process spawned; a fake program without one is run once with no network and stdin closed, and is
   killed at 2 s. Today there is no check.
2. Opening the tab twice runs no program the second time when nothing changed (spawn counter).
3. At most two checks run at once.
4. A program in a world-writable folder is refused as a launch command.
5. The tab lists 11 helpers and 1 app in the three groups. Today the Claude tab lists paired hubs only.

## 11. Check on a laptop or phone

- macOS with Claude Code and Codex installed: first open asks; Check this computer; versions from package files;
  Activity Monitor shows no more than two helper processes during the check.

## 12. Open decisions it depends on

- **W42, W45, W46, W47, W49** (Recommended). **W50** (owner to confirm).
- Depends on: `04-helper-detail.md`, `05-commands-and-platforms.md`, `01-this-pc.md` (lockdown).
