# Changes outside the screens

Everything the phone and web specs depend on that is not a screen: server routes and behaviour in
`crates/hq-web` and friends, desktop app commands, release tooling and docs. Each line names the wave
that first needs it and the specs that use it. Build these in the wave they are listed for, with tests,
before the screens that call them. `resolutions.md` settles any conflict with this file.

## Server (crates/hq-web, hq-db, hq-agent, hq-llm)

| Change | Why | First needed | Used by |
|---|---|---|---|
| Web push carries no content and no answer buttons; notify_owner pushes its title only; push answer tokens retired; two kinds (Needs you, Updates) filtered per subscription; task, stuck-helper and server titles per resolutions.md B19; waiting items replayed once when a subscription or kind comes back (C-11) | R1 on the web (live bug) | Web wave 0 | web/wave-0 live bugs, rules-web.md 1 |
| `drive`, `goal` and `answer` routes require the HQ client header (`crates/hq-web/src/origin.rs`) | Live bug | Web wave 0 | web/wave-0 live bugs |
| Device sign-ins store browser, system, created and last used; expire after 90 days without use; "Keep signed in" | Sign-in safety | Web wave 1 | web Settings, rules-web.md 3 |
| A new sign-in creates a Needs you item with It was me and Sign it out | Phishing and lost devices | Web wave 1 (phone hub: phone wave 1, resolutions.md C-4) | web Inbox, Settings; phone Other devices |
| `GET /api/status/ai` (signed in): last good and failed call per AI service, failure kind, no prompt or answer text; the LLM backend chain records calls | D16 | Phone wave 0 / web wave 0 | Connection state on both |
| A starting flag HQ returns while it opens (so a browser can show "Starting HQ") | Connection states | Web wave 0 | rules-web.md 2 |
| Which edition is running (full, Lite, Mobile) | Lite shell | Web wave 2 | web routes and editions |
| Approvals carry a `loses` flag set by the runtime from the tool's category; the card gets the tool and its arguments verbatim (resolutions.md B1, phone too); an approval lapses after 24 hours (C-7) | N17, WB6 | Phone wave 1 / web wave 1 | approvals everywhere |
| Chat messages carry `source` (only "voice") and `client_id` in their meta (`chat_messages.meta`, `add_message_with_meta`); echoed on `turn_start`; returned from `/api/threads/{id}/messages` | D7, the assistant hand-off | Phone wave 3 / web wave 4 | assistant hand-off, web Chat |
| HQ drops a repeated `client_id` (messages, notes, tasks); the web outbox holds at most 100 items (WR30) | Outbox never sends twice (resolutions.md C-3) | Phone wave 4 / web wave 0 shared parts | outbox on both |
| `turn_end` gains `waiting_for_owner`; voice approvals start with "You said: …" | Assistant hand-off | Phone wave 3 | mobile/wave-3 hand-off |
| Resume a reply by message id; Stop over HTTP as well as the socket | Dropped links | Web wave 4 | web Chat |
| Restore routes for every Undo, with a 10-minute window: chat unarchive (and rename), Inbox item restore (dismiss is final today), spending limit, task archive (exists), cleared update; a restore past the window or after a change answers 409, shown as "Couldn't undo. It changed on another device, or too much time passed." | W23, WR7, N18, resolutions.md A6, C-1 | Phone wave 1 / web wave 1 | Undo on both |
| Notes: a save carries the version it started from; HQ answers "changed" (409) when it moved on; create is idempotent by client id | WB4, resolutions.md C-2, C-3 | Web wave 4 (phone notes too, phone wave 4) | web Notes, phone Notes |
| `/api/search` takes `folder` (a path prefix filtered before the limit; rejects `..`, a leading `/`, backslashes) | One search | Phone wave 4 / web wave 4 | both Search specs |
| Backup status, update status and diagnostics routes (the web's Backup row turns amber only when this route reports failed or overdue, resolutions.md B10) | Truthful rows | Phone wave 1 / web wave 1 | backup, Settings |
| A signed-in busy route (turn, backup, outbox flush, download running) for the idle gate before an install | R3 | Phone wave 0 | safe mode, updates |
| `hq vault moved`: clears device sign-ins, paired devices, connector tokens and push subscriptions on a moved phone | M1 | Phone wave 0 | data transfer |
| The start route takes an optional task field (start a helper on a task) | WR17 | Web wave 4 | web Helpers |
| Owner approvals and "Let HQ say yes" never allow a losing request | W15 | Web wave 1 | Chat, Helpers |
| Times sent as instants; one HQ time zone plus a display zone per browser | M11 | Web wave 2 | everywhere times show |
| Change events over the one socket instead of polling loops | Data budget | Web wave 2 | rules-web.md 9 |
| A minimum-client header, so an old page can say "A newer HQ is running. Reload to keep going." | Version skew | Web wave 2 | rules-web.md 7 |

## Desktop app (apps/desktop, crates/hq-desktop)

Every new command goes into `apps/desktop/src-tauri/src/commands.rs`, `build.rs` and
`capabilities/control.json` together; `crates/hq-desktop/tests/shell_config.rs` checks they match and
that no plugin is used.

| Change | Why | Wave |
|---|---|---|
| The HQ window proxies only to a forward the hub opened itself; a taken port says "Another program holds this port. HQ Hub won't send your sign-in there." (`desktop.rs` `upstream_addr`, hq-bridge `start_forwards`) | WB3, live bug | Web wave 0 |
| `open_main_window(path)` with an allow-list in `window.rs` | Open a helper from the hub | Web wave 3 |
| `host_log` (last lines in a sheet, keys blanked) | Open the log | Web wave 3 |
| `pick_program` (native chooser in Rust; refuses world-writable folders and programs that are not HQ or the helper) | Choose the program | Web wave 3 |
| `host_unlock` (runs `hq host unlock` locally) | W25 | Web wave 3 |
| `host_status` gains the start step, the usual start time, stopped-by-progress (resolutions.md A7), the lockdown's who, when and why, the helpers working now (Stop HQ is red only with some, B9), a restarting state and free space where `~/.hq` lives (C-12) | Connection states in the hub | Web wave 3 |
| Helpers tab: `helpers_check`, `helpers_list`, `helper_try`, `helper_try_stop`, `helper_connect`, `helper_disconnect`, `helper_restore`, `helper_signin` (fold `claude_status` and `claude_change` into these) | Helpers tab | Web wave 3 |
| A helper list the hub reads: program, display name, version source (package file first), install line, sign-in check, connect target | Helpers tab | Web wave 3 |
| System notices for Needs you items on that computer, sent from Rust (no plugin) | Notifications in the hub | Web wave 3 |
| productName "AgentHQ"; control window "HQ Hub"; HQ window "HQ"; the rename plan for Claude entries pointing at HQ.app (WR22) | Names | Web wave 3 |
| Bundle Clear Sans and the web styles locally (the CSP allows `font-src 'self'`) | One look | Web wave 3 |
| `open_in_browser` without `/usr/bin/open` for Windows and Linux (future scope, W50) | Cross-platform | Later |

## Phone app and Android release

| Change | Why | Wave |
|---|---|---|
| `res/xml/data_extraction_rules.xml` and the manifest entry | M1, D17 | Phone wave 0 |
| `:safe` process and the "Save notes" launcher shortcut | R3 | Phone wave 0 |
| The five notification channels created once at the hub's start | R1 | Phone wave 0 |
| Strings moved to resources from one catalogue (N19), with the literal lint (resolutions.md D-8) | Translation | Phone wave 0 |
| Test setup: Compose UI tests (Robolectric or instrumented), a screenshot test, an Android lint-checks module for the string rules | resolutions.md D-11 | Phone wave 0 |
| The phone's web pages load with `?embed=1` | resolutions.md C-9 | Phone wave 4 |
| `UpdateLogic.kt` hardcodes a real GitHub owner and repo: move it behind config | CLAUDE.md rule | Phone wave 2 |
| The signed channel file gains `seq`, `size`, `notes` (a map from language to up to three lines), `withdrawn` (a list of versionCodes), `rollout`, `switches` (with `save_copy` and `lock_screen_assist_off_models`), as resolutions.md D-3 shapes them; `scripts/release/android-channel.sh` writes them; `promote.yml` re-signs instead of copying; a withdraw workflow | R3, updates | Phone wave 0 (fields) / phone wave 2 (updates) |
| `UPDATE_PACKAGES_WITHOUT_USER_ACTION` only when D11's conditions are met | D11 | Later |

## Web app release

| Change | Why | Wave |
|---|---|---|
| Manifest name "AgentHQ", shortcuts Chat, Notes, Inbox; document title | Names | Web wave 2 |
| Redirects for every old route, including links stored in Inbox items and pushes, QR codes (`/vault#code=`), `#token=` login links, `/sessions?id=`, `/chat?thread=`, `/tasks?task=`, `/notifications` | Old links keep working | Web wave 2 |
| Heavy readers (PDF, spreadsheets, Word, code colouring) load on demand | Data budget | Web wave 2 |
| Install lines for helpers checked in the release checklist | W49 | Web wave 3 |

## Docs to update when the code lands

- `docs/DESKTOP.md`: new tabs, the connector's status on the web (W34), cross-platform notes, the Claude tab line that says it has no buttons.
- `docs/security/DESKTOP.md`: new commands, the proxy rule, system notices, unlock.
- `docs/WEB_PUSH.md`: no content and no buttons by default.
- `docs/HQ_MOBILE.md`: safe mode, data transfer, channels, the assistant setup path, 57 MB model.
- `docs/AGENT_HOST.md`: lockdown is lifted in HQ Hub on that computer (unchanged rule, new place).
- `CLAUDE.md` entry points, if new crates or major files appear.
