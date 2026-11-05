# Web wave 3 · 05 · HQ Hub: every command and capability change, the docs, and Windows and Linux as future scope

Boards: **AlignedHub3** ("What HQ Hub needs built: commands and capabilities", "Also changes", "Settings
file", "Docs that are stale"), **AlignedFlow** ("Needs new commands"), **AlignedNotesNote2** (a save dialog
for the HQ window), AlignedReview ("Server, desktop and release changes no board named", D03, D07, D08,
D09, D12), Drift desktop 03, 08, 09, 12. Follows `../../rules-web.md` section 6. Where this spec and the
rules differ, the rules win.

## 1. Finding and the review items it closes

- **D03 (Drift desktop 03), partly closed.** A file picker is not allowed by the pinned capability and
  no-plugin tests; no command named.
- **D07 (Drift desktop 07).** No way to the agent: rows are not links and `open_main_window` takes no path.
- **D08 (Drift desktop 08).** No open-file command; `open` is macOS only.
- **D09 (Drift desktop 09).** The Dock name is the bundle's; renaming moves HQ.app, which Claude entries
  point to.
- **D12 (Drift desktop 12), not closed.** `docs/DESKTOP.md` says the Claude tab "has no buttons yet", and
  calls the connector retired, against W34.
- **Review "Desktop" list:** open the HQ window at a path; open the log; a file picker inside the pinned
  capability list; a bundle rename plan; a start timeout; bundled fonts for the stricter CSP.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/desktop/src-tauri/src/commands.rs:24-46` the handler list; commands at lines 48 to 156:
  `startup_warning`, `host_status`, `host_start`, `host_stop`, `host_agents`, `settings_get`,
  `settings_save`, `open_main_window` (84, no path), `open_in_browser` (91, `/usr/bin/open` at 94),
  `pairing_start`, `pairing_stop`, `pairing_begin`, `pairing_cancel`, `pairing_snapshot`, `pairing_submit`,
  `pairing_decline`, `pairing_revoke`, `claude_status`, `claude_change`.
- `apps/desktop/src-tauri/capabilities/control.json`: the same as `allow-*` permissions plus
  `core:event:allow-listen` and `-unlisten`, for the window `control` only; the web window has none.
- `apps/desktop/src-tauri/build.rs`: lists the commands for the permission build.
- `crates/hq-desktop/tests/shell_config.rs:119` "only the control window has a capability and it grants
  exactly the app commands"; `:183` "the shell has no plugin and so no login item, shell access or
  updater"; `:49` the CSP is strict and names no remote origin; `:204` both windows are made in code with a
  navigation guard and no new windows; `:243` the local network reason.
- `apps/desktop/src-tauri/tauri.conf.json`: `productName` "HQ", targets `["app"]`, CSP `font-src 'self'`,
  `style-src 'self'`.
- `crates/hq-desktop/src/window.rs:12-50` `app_page_allowed`, `main_page_allowed`, `browser_target`.
- `crates/hq-desktop/src/settings.rs:23-32` the settings file fields.
- `docs/DESKTOP.md`: tabs Host, Devices and Claude; the Claude tab line; "macOS" only.

## 3. Decision

**Every new action is a command added in three places at once, `commands.rs`, `build.rs` and
`capabilities/control.json`, with the `shell_config.rs` test updated in the same change. Only the control
window gets them. The HQ window keeps no capability, and the app stays plugin-free.**

### The commands

| Command | What it is for | Capability |
|---------|----------------|------------|
| `open_main_window(path)` | Open the HQ window at a helper, an Inbox item or Settings › Other devices (D07) | Same permission, `allow-open-main-window`. The path is checked against a short list in `window.rs`: `/helpers/{id}`, `/inbox/{id}`, `/settings/devices`, `/chat`, `/`. |
| `host_log` | Open the log: the last lines in a sheet, keys blanked (D08) | New: `allow-host-log` |
| `pick_program` | Choose the HQ program, or a helper's, in a native chooser (D03) | New: `allow-pick-program`. Made in Rust, no plugin. Refuses world-writable places. Pasting a path is the fallback (WR21). |
| `host_unlock` | Allow helpers again, on this PC only (WB5) | New: `allow-host-unlock`. Runs `hq host unlock` locally. |
| `host_status` | Start step, usual start time, stopped by progress, the lockdown's who, when and why | Existing permission, more fields |
| `helpers_check`, `helpers_list` | The Helpers tab: package-file versions, sandboxed checks, two at a time, cached | New: `allow-helpers-check`, `allow-helpers-list` |
| `helper_try`, `helper_try_stop` | Try it, through the host, in one fixed folder | New: `allow-helper-try`, `allow-helper-try-stop` |
| `helper_connect`, `helper_disconnect`, `helper_restore` | Connect to HQ for any helper `hq mcp install` knows. Remove with Undo for 10 minutes. | New, and `claude_status` and `claude_change` fold into them |
| `helper_signin` | Copy sign-in link, and open it only on that helper's own sign-in site | New: `allow-helper-signin`. `open_in_browser` keeps its own-address rule. |
| (event) notice | A system notice for Needs you on this PC, Open only | No command. A native notice from Rust, since no plugin is allowed. |

### Also changes

- **The proxy refuses a forward HQ Hub did not bind** (`desktop.rs`, `from_config.rs`), shipped in wave 0.
- **Names:** `productName` "AgentHQ" and the control window titled "HQ Hub" (`tauri.conf.json`,
  `main.rs`). The bundle rename plan (WR22, owner to confirm): on first open after the rename, HQ Hub
  rewrites only the Claude entries it added that point inside the old HQ.app, by name and exact command,
  and says so once.
- **`open_in_browser` calls `/usr/bin/open`,** so it works on macOS only. Windows and Linux need their own
  opener (future scope).
- **Fonts and styles:** bundle Clear Sans and `aligned-ui.css` locally (the CSP allows `font-src 'self'`).
- **A start timeout** by progress in `host.rs` (`01-this-pc.md` rule 2).
- **A save dialog for the HQ window** (export from the phone's HQ) needs a new desktop command inside the
  pinned list; until then Export is off in the HQ window with the reason (WR13).
- **The proxy adds `x-hq-window: hub`** on responses so the web app knows it is inside HQ Hub
  (`../wave-1/06-settings-groups.md`), and marks a gone upstream with `x-hq-proxy: upstream-gone`
  (`../wave-0/02-shared-parts.md`).

### Settings file

New fields in `~/.hq/desktop/settings.json`: "Look for helper updates" (`look_for_helper_updates`, off),
the Try it folder (`try_it_dir`, default `~/.hq/desktop/try-it`), the usual start time
(`usual_start_secs`). Saved with `settings_save` as today.

### Docs that are stale

- `docs/DESKTOP.md`: tabs Host, Devices and Claude become This PC, Devices and Helpers. Its Claude row
  calls the claude.ai connector retired, against W34. "macOS" only. The Claude tab line says it "has no
  buttons yet"; it has Add and Remove per paired hub (#339) and moves to Helpers.
- `docs/security/DESKTOP.md`: the new commands, the proxy rule, system notices and the unlock.

Rejected:

- **A Tauri dialog or notification plugin.** The no-plugin rule (`shell_config.rs:183`) keeps the shell
  free of an updater, a shell and login items; a native chooser and notice in Rust are small.
- **Give the HQ window a capability** (for downloads or the log). Any web page it shows could then call it.

## 4. Screens and states

No screens of their own: each command serves a screen in `01-this-pc.md` to `04-helper-detail.md`.

### Windows and Linux (future scope, W50, owner to confirm)

- **One control window for macOS, Windows (helpers inside Ubuntu on WSL2) and Linux,** drawn the same.
  "This PC" on every system.
- **Windows:** helpers run inside Ubuntu (WSL2); HQ itself on the phone, or all of HQ in Ubuntu with
  `hq start all` (`docs/WINDOWS.md`). `wsl.exe` is called by its full path
  (`C:\Windows\System32\wsl.exe -d Ubuntu-24.04 -- ~/bin/hq host serve`). Helper checks inside Ubuntu skip
  the Windows folders WSL adds to PATH (no `/mnt` paths). Claude Desktop runs in Windows; Connect to HQ
  makes its entry start HQ inside WSL2. Copy: "HQ Hub never starts anything when you sign in to Windows."
- **Linux:** the same window; the opener is `xdg-open`; notices through the desktop's notification
  service from Rust.
- **Lite:** HQ Hub is not part of HQ Lite.
- **Needs building:** HQ Hub is a macOS bundle today (targets `app`). Windows and Linux builds, starting
  through `wsl.exe`, an opener per system and a native chooser per system are new.

## 5. Exact copy

No new owner-facing strings beyond the screen specs. Command errors map to the catalogue's hub reasons
(`../wave-0/02-shared-parts.md`).

## 6. Behaviour rules

1. **Three lists, one change.** A command missing from any of `commands.rs`, `build.rs` or
   `control.json` fails `shell_config.rs`.
2. **Control window only.** No command is reachable from the HQ window (its webview label has no
   capability); `open_main_window(path)` takes only paths on the allow-list.
3. **No plugin.** The chooser (`NSOpenPanel` on macOS through `objc2`, later the system dialogs on Windows
   and Linux) and the notice (`UNUserNotificationCenter` on macOS) are Rust.
4. **Refuse world-writable** paths in `pick_program` (the file or any parent writable by others), and a
   program that does not answer as HQ (`hq --version` answering `hq <version>` within five seconds) or as
   the helper.
5. **`host_unlock`** runs the local `hq host unlock` with no arguments from the web; it never reaches a
   remote host.
6. **`helper_restore`** restores only the exact entry `helper_disconnect` removed, within 10 minutes and
   only if the settings file did not change since.

## 7. Browser and platform notes

- macOS 12 is the minimum (`tauri.conf.json`). `UNUserNotificationCenter` needs the bundle to be signed
  for notices to show reliably; ad hoc builds may need the owner to allow notices in System Settings.
- The control window's CSP forbids remote fonts, so Clear Sans ships in `ui/dist/fonts/`.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/desktop/src-tauri/src/commands.rs` | Every command above; `open_main_window(path)`. | yes |
| `apps/desktop/src-tauri/build.rs` | The same list. | yes |
| `apps/desktop/src-tauri/capabilities/control.json` | New `allow-*` permissions. | yes |
| `apps/desktop/src-tauri/tauri.conf.json` | `productName` "AgentHQ". | yes |
| `apps/desktop/src-tauri/src/main.rs` | Titles; the HQ window opened at a path (`show_main`, 161). | yes |
| `crates/hq-desktop/src/window.rs` | The path allow-list; the sign-in site allow-list. | yes |
| `crates/hq-desktop/src/notice.rs` | The native notice. | NEW |
| `crates/hq-desktop/src/chooser.rs` | The native chooser and its refusal rules. | NEW |
| `crates/hq-desktop/src/helpers.rs` | Helpers commands (`03-helpers-tab.md`, `04-helper-detail.md`). | NEW |
| `crates/hq-desktop/src/proxy.rs` | `x-hq-window` and `x-hq-proxy` headers. | yes |
| `crates/hq-desktop/src/settings.rs` | New fields. | yes |
| `crates/hq-desktop/tests/shell_config.rs` | Updated lists; a test that the HQ window has no capability still passes. | yes |
| `apps/desktop/scripts/build-ui.ts` | Copy the stylesheet and fonts. | yes |
| `docs/DESKTOP.md`, `docs/security/DESKTOP.md` | As above. | yes |

## 9. Server, desktop or release changes

All desktop. Release: the bundle rename (WR22) with the Claude entry rewrite; the desktop release
workflow (`.github/workflows/desktop-release.yml`) builds `AgentHQ.dmg` once renamed.

## 10. Acceptance checks (each fails on today's build)

1. `cargo test -p hq-desktop --test shell_config`: the capability grants exactly the new list and the
   web window has none. Today the list lacks every new command.
2. `open_main_window("/helpers/h-31")` opens the HQ window there; `open_main_window("https://example.com")`
   is refused. Today the command takes no path.
3. `pick_program` refuses `/tmp/hq` (world-writable `/tmp`). Today there is no chooser.
4. `productName` is "AgentHQ"; the control window is titled "HQ Hub".
5. `docs/DESKTOP.md` names the tabs This PC, Devices and Helpers.

## 11. Check on a laptop or phone

- macOS: from a system notice for a helper that needs you, Open: the HQ window opens on that helper.
- Choose the program with the native chooser; Open the log shows the last lines with a key blanked.

## 12. Open decisions it depends on

- **W50, WR22** (owner to confirm). **WR13, WR20, WR21** (Recommended). **OD-W3**.
- Depends on: `01-this-pc.md` to `04-helper-detail.md`, `../wave-0/01-live-bugs.md` (the proxy).
