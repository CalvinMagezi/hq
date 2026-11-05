# Web wave 3 · 04 · HQ Hub: one helper, Try it, sign-in and connections (WB7, M6)

Boards: **AlignedHubHelperDetail** ("one helper": a helper that works, what Try it does, signing in a
helper) and **AlignedHubHelperDetail2** (Undo, install, a refused program, Claude Desktop, the state
table, the exact rules), AlignedReview WB7 and M6. Follows `../../rules-web.md` section 6 (6.5 Try it,
6.6 helper checks) and `../../rules-shared.md` sections 3 and 4 (D8 for spending). Where this spec and
the rules differ, the rules win.

## 1. Finding and the review items it closes

- **WB7 (blocker). Try it answers a helper's trust prompt.** The old hub board said Try it "answers yes,
  as HQ does for Antigravity". Trust prompts default to No and are reported, never answered, except where
  `spec.rs` marks a helper verified. Try it follows `spec.rs` exactly and reports a trust prompt as "Needs
  you on this computer".
- **M6.** The version's source shown ("from its package file"), the full path once; "Choose the program"
  refuses world-writable places.
- **D8 ask.** Hermes and OpenClaw use "Not now" · "Ask 1 question", two equal buttons, since it uses their
  plan.
- **M2.** Remove the connection in ink, red only in a dialog. **W48:** removing a connection acts at once
  with Undo.

## 2. Before (as built, verified at `e5f9d5f`)

- No helper detail in the hub today. The Claude tab adds or removes a paired HQ for Claude Code and Claude
  Desktop (`crates/hq-desktop/src/desktop.rs:240-258` `claude_status`, `claude_change`).
- `crates/hq-tools/src/harness_session/spec.rs:218` the only `trust_pattern`, Antigravity ("trust the
  contents"); `:251` and `:268` `one_shot` for Hermes and OpenClaw.
- `docs/AGENT_HOST.md`: a coding agent's trust dialog defaults to "No, exit", so blocked launches are
  reported, never auto-answered (also `CLAUDE.md`, "Coding-agent runtime").
- `apps/desktop/src-tauri/src/commands.rs:91-103` `open_in_browser()` opens only the app's own proxy
  address (`window.rs:40` `browser_target`), with `/usr/bin/open`.
- The board gives Try it a 25 s wait, "HQ's own launch limit". `../../decisions.md` WR31 settles it:
  Try it waits up to `agent_host.launch_bound_secs` (25 s by default,
  `crates/hq-tools/src/agent_host/mod.rs`), read from config, not a constant of its own.

## 3. Decision

**A row's name opens the detail: the program, its version and where that came from, the HQs it can reach,
Check again and Try it. Try it starts the helper exactly as real work does, in one fixed folder, never
answers a trust prompt `spec.rs` does not answer, and says what data it may use. Every result stays until
the next change.**

Rejected:

- **Answer every trust prompt in the Try it folder** (the old board). It would teach owners that HQ says
  yes to trust, which it never does for real work.
- **A red Remove the connection.** It comes straight back with Undo, so it acts at once in ink.

## 4. Screens and states

Same window and frame. A back link "All helpers", the name as the title, a line "Coding helper · version
2.1.12" (or "General helper", "App that uses HQ"), the state pill, "Checked at 14:30." [Check again].

- **D1 · Ready, connected, tried (Claude Code).** Facts: "Version · 2.1.12, from its package file";
  "Program · ~/.local/bin/claude"; "Last tried · Today at 14:32, it worked". "Connected to HQ": "HQ on this
  PC · Claude Code can use your notes, tasks and helpers · Connected" [Remove the connection] (quiet ink);
  "Pixel 8, your HQ · Through the pairing". "Try it": "Worked at 14:32. Claude Code started in the Try it
  folder in 3 s, was ready, then closed. It got no prompt." [Try it again] (the one primary). Technical
  details: "Program: claude, at ~/.local/bin/claude"; "Version: read from
  @anthropic-ai/claude-code/package.json, nothing run"; "Connections: ~/.claude.json, entries agent-hq
  (this computer) and hq-pixel-8 (paired)"; "Try it: through HQ's host, the sandbox real work uses, folder
  ~/.hq/desktop/try-it, self-updater off".
- **D2 · Trying it, where HQ answers the trust prompt (Antigravity).** Pill "Trying it" (blue). "1.4.0,
  from its package file", "New here: /Users/you/.local/bin/agy". "Trying Antigravity in the Try it folder
  · 6 s" / "It asked whether to trust the folder. HQ answers yes for Antigravity, as it does for real
  work." / "No prompt is sent. Antigravity may contact its own service to check your sign-in." [Stop
  trying]. "Antigravity can use your notes, tasks and helpers · Connected. Restart Antigravity to see HQ."
  It gives up after HQ's own launch limit (`agent_host.launch_bound_secs`, 25 s by default, WR31), and
  says so.
- **D3 · A trust prompt HQ does not answer (Cursor).** Pill "Needs you on this computer" (amber). "Try it
  stopped on a question" / "At 14:31, Cursor asked whether to trust the Try it folder. HQ never answers that
  for Cursor." / "When HQ starts Cursor for real work, the question comes to your Inbox." / "To answer it
  here once, run Cursor in the Try it folder:" / `cd ~/.hq/desktop/try-it && cursor-agent` [Copy]. Facts:
  "2025.10.2, from a sandboxed check", "~/.local/bin/cursor-agent", "Today at 14:31, it asked a question".
  The question's words are kept only until the next try; Try it never saves a helper's screen.
- **D4 · General helpers ask first (OpenClaw, Hermes).** "General helper · version 2026.10.2", "2026.10.2,
  from its package file", "~/.npm-global/bin/openclaw", "Last tried · Not yet", "OpenClaw has no ready
  screen, so Try it asks it one short question.", "Connected to HQ · Not offered. HQ never connects itself
  into a general helper." The dialog: "Try OpenClaw?" / "Try it asks OpenClaw one short question. That uses
  a little of your OpenClaw plan." / [Not now] [Ask 1 question] (two equal outlined buttons). The result
  stays in the card: the answer's first line, and how long it took.
- **D5 · Signing in a helper (Qwen Code).** Pill "Needs sign-in". "Waiting for you to sign in · 9:12 left"
  / "Qwen Code is open in the Try it folder, on its sign-in screen. Copy its sign-in link and open it in
  your browser." [Copy sign-in link] [Open the sign-in page] [Stop] / "The page opens only on Qwen Code's
  own sign-in site. HQ Hub never saves the sign-in screen." Facts "0.2.1, from its package file",
  "~/.npm-global/bin/qwen", "Today at 14:31, it asked to sign in". "HQ can start Qwen Code and give it work
  without a connection. Connecting it, to reach HQ from inside it, is not available yet." Sign in… on the
  row starts the helper again in the Try it folder and keeps it open up to 10 minutes.
- **D6 · Remove the connection, with Undo.** "HQ on this computer · Removed at 14:40. Restart Claude Code to drop
  HQ. · Not connected" [Connect to HQ]; "Pixel 8, your HQ · Through the pairing · Connected" [Remove the
  connection]. Toast "Connection removed" · Undo. Undo restores the same entry, on this computer, within 10
  minutes. After that, or if the entry changed: "Couldn't undo. It changed on another device, or too
  much time passed." (`../../resolutions.md` A6). Only the
  entry HQ added is removed, by name and exact command; anything else in Claude Code's settings is left
  alone.
- **D7 · Not installed (GitHub Copilot).** Grey. "Not on this computer" / "Install it with this line in a
  terminal. Sign in once (run copilot, then type /login), then check again." / `npm install -g
  @github/copilot` [Copy] ("Copied" stays beside it, ink, until the next action) / [Its install page] /
  "It needs Node.js. HQ Hub never runs an installer itself." [Check again] (primary). Facts "Looked for ·
  Its program, in 14 places", "Last checked · Today at 14:30". Technical details: "Program: copilot";
  "Looked in: PATH, ~/.local/bin, ~/.bun/bin, ~/.cargo/bin, ~/.npm-global/bin, ~/.volta/bin, mise, asdf
  and pnpm folders, ~/.claude/local, ~/.nvm Node folders, /usr/local/bin, /opt/homebrew/bin, then your
  login shell's PATH".
- **D8 · Program not found, and a refused choice (Kimi).** Amber: "Its settings are here, its program is
  not" / "HQ starts Kimi's program, and it isn't in any place HQ looks. If you installed it somewhere else,
  choose it. Or install it:" / `uv tool install kimi-cli` / [Choose the program…] / refusal "Couldn't use
  that program. Anyone on this computer can change its folder. Choose one that only you can change." Technical
  details: "Looked for: kimi", "Settings found: ~/.kimi", "A chosen program is saved as HQ's launch command
  for Kimi (an absolute path)".
- **D9 · Claude Desktop, in "Apps that use HQ".** "App that uses HQ · version 1.0.9" / "Not connected to
  HQ" / "An app you open yourself. HQ does not start it, so there is no Try it. Connecting adds HQ to it.
  Restart it to see HQ." Failure on its row: "Couldn't connect. Claude Desktop's settings file has a
  mistake in it. Fix or remove it, then try again." Technical details: "Settings file: Claude Desktop's
  claude_desktop_config.json (macOS: ~/Library/Application Support/Claude, Windows: %APPDATA%\Claude)",
  "Entry agent-hq starts: ~/bin/hq mcp-serve", "Paired HQ entry starts: ~/bin/hq mcp-serve --remote
  pixel-8". One Connect to HQ per HQ it could reach. On Windows it connects through Ubuntu (needs
  building).

### Every state, and exactly what each action does

A helper can be several of these at once. The row shows the first that applies, in this order. The
detail shows all of them.

| State | Colour | The row says | Its one action | Today |
|-------|--------|--------------|----------------|-------|
| Checking | Blue, spinner | Reading its version | None yet | Needs building |
| Waiting | Grey | Waiting its turn | None yet | Needs building: two at a time |
| Not installed | Grey | Not on this computer | How to install | The search exists. Install lines need building |
| Program not found | Amber | Its settings are here, its program is not | Choose the program… | Needs building, and `pick_program` |
| Check failed | Amber | Couldn't read its version in 2 s | Check again | Needs building |
| Needs sign-in | Amber | Its sign-in screen showed | Sign in… | Needs building, from what Try it saw |
| Needs you on this computer | Amber | Its last Try it stopped on a question | See what it asked | Needs building. The host already reports a blocked start |
| Not connected to HQ | Amber | Not connected to HQ yet | Connect to HQ | Exists for six helpers and Claude Desktop (`hq mcp status`), and paired HQs in Claude (`hq claude`) |
| Update available | Grey | 0.58.0 · 0.60.1 is out | How to update | Needs building, behind a switch that is off: looking it up reaches the internet |
| Ready | Grey, ink words | Its version | Try it | The search exists. The version read needs building |
| Trying it | Blue, spinner | Trying it in the Try it folder · 6 s | Stop trying | Needs building, on the host HQ already uses |

## 5. Exact copy

Every string in section 4 is exact. Keys `hd_*`. The rules sentences:

| Key | String |
|-----|--------|
| `hd_try_rule` | "Starts the helper through HQ's host, with the sandbox and launch command real work uses, in one fixed folder (~/.hq/desktop/try-it, emptied after each try)." |
| `hd_check_rule` | "Version from the program's package file first. Otherwise one sandboxed run of its version line: no network, nothing typed into it, 2 s, two at a time, cached by path, size and date." |
| `hd_connect_rule` | "For HQ on this computer, what hq mcp install writes for that helper. For a paired HQ, what hq claude add writes (Claude Code and Claude Desktop only)." |
| `hd_signin_rule` | "Keeps the helper open in the Try it folder for up to 10 minutes. Copy sign-in link copies the link its screen shows. Open the sign-in page opens it only on that helper's known sign-in site." |

## 6. Behaviour rules

1. **Try it, exactly.** Start the helper through HQ's host, with the sandbox and launch command real work
   uses, in `~/.hq/desktop/try-it`, emptied after each try (so a trust answer given there lasts, WR23).
   Known self-updaters are off for it (Codex's update check, already off in `spec.rs`, and Claude Code's
   updater). Wait up to `agent_host.launch_bound_secs` (25 s by default, WR31); note whether its screen
   showed a sign-in or a question; close it.
2. **Trust prompts.** Answer one only where `spec.rs` has a `trust_pattern` (Antigravity). Report any other
   as "Needs you on this computer". Never send a prompt, except the one question for Hermes and OpenClaw, asked
   first with D8's two equal buttons.
3. **Try it needs HQ,** or helpers here in hub mode, running; with neither, it is off with the reason.
4. **Helper checks, exactly** (rules-web 6.6, `03-helpers-tab.md` rules 1 to 6). A chosen program in a
   place anyone can change is refused, and so is one that does not answer as that helper.
5. **Connect to HQ, exactly.** For HQ on this computer, what `hq mcp install` writes for that helper. For a paired
   HQ, what `hq claude add` writes (Claude Code and Claude Desktop only). Remove takes out only that entry,
   by name and by its exact command or address, and Undo puts the same entry back within 10 minutes. Not
   offered for general helpers.
6. **Sign in, exactly.** Keep the helper open in the Try it folder for up to 10 minutes (WR24). Copy
   sign-in link copies the link its screen shows. Open the sign-in page opens it only on that helper's
   known sign-in site; any other address is refused. Nothing from the screen is saved; a helper's saved
   sign-in files are never read.
7. **Results stay** in the card until the next try or check.
8. **The full path** shows the first time a program is seen, then the short form.

## 7. Browser and platform notes

- macOS: Claude Desktop's file is `~/Library/Application Support/Claude/claude_desktop_config.json`;
  Windows `%APPDATA%\Claude` (future scope).
- `open_in_browser` uses `/usr/bin/open` (macOS only); the sign-in opener needs the same per-site check on
  every system.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `crates/hq-desktop/src/helpers.rs` | Detail data, Try it runner over the host (its wait is `agent_host.launch_bound_secs`, WR31), sign-in session, connection add, remove and restore. | NEW |
| `crates/hq-tools/src/harness_session/spec.rs` | Sign-in sites per helper; self-updater flags. | yes |
| `crates/hq-desktop/src/claude.rs`, `crates/hq-bridge/src/claude.rs` | Folded into `helper_connect`; restore an exact entry. | yes |
| `crates/hq-cli/src/commands/mcp.rs` | Targets for Codex, Pi, Kimi and Qwen (needs building). | yes |
| `crates/hq-desktop/src/window.rs` | The sign-in opener's per-helper allow-list. | yes |
| `apps/desktop/ui/src/main.ts`, `ui/index.html` | The detail page. | yes |
| `apps/desktop/src-tauri/src/commands.rs`, `build.rs`, `capabilities/control.json` | `helper_try`, `helper_try_stop`, `helper_connect`, `helper_disconnect`, `helper_restore`, `helper_signin`, `pick_program`. | yes |

## 9. Server, desktop or release changes

Desktop: the commands above (`05-commands-and-platforms.md`). HQ: none beyond the host's existing blocked
start report. Settings file: "Look for helper updates" (off), the Try it folder.

## 10. Acceptance checks (each fails on today's build)

1. `cargo test -p hq-desktop helpers::try_it`: a fake helper whose screen shows a trust prompt is reported
   as "Needs you on this computer" for every helper but Antigravity, and no key is sent. Today there is no Try
   it.
2. Try it for Hermes asks first; without the yes, no process starts.
3. Remove then Undo restores the same entry byte for byte; after 10 minutes Undo answers the too-late line.
4. Open the sign-in page with a link to another site is refused.
5. The detail shows "from its package file" for a version read without running the program.

## 11. Check on a laptop or phone

- macOS with Claude Code: Try it: "Worked at …"; Activity Monitor shows it close.
- With Cursor installed: Try it reports the trust question and offers the line to copy.

## 12. Open decisions it depends on

- **W46, W48, WR23, WR24, WR31** (Recommended). **WR21** (the native chooser). `../../resolutions.md` A6.
- Depends on: `03-helpers-tab.md`, `05-commands-and-platforms.md`, `../wave-1/03-undo.md`.
