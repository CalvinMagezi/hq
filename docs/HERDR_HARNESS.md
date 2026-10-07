# Herdr harness (retired)

> Herdr was replaced by HQ's built-in host (`hq host`). This page still describes the
> harness-session tools and workflow, which are unchanged, but anything about the
> `herdr` CLI, `hq-herdr-gate` or `herdr.service` is historical. Current setup:
> [AGENT_HOST.md](AGENT_HOST.md) and [NATIVE_HOST_CUTOVER.md](NATIVE_HOST_CUTOVER.md).

Agent HQ runs and monitors coding agents (Claude Code, Codex, Cursor, Pi, OpenCode,
Copilot CLI, Kimi, Qwen, Antigravity) in [Herdr](https://herdr.dev). Herdr owns the
terminals, recognises which agent sits in a pane, and reports its state: `idle`,
`working`, `blocked` (waiting on a dialog or approval), or `done`. HQ drives it through the
`herdr` CLI. It replaced the earlier tmux-based runner, which could only guess state from
screen text.

A **host** is a machine running Herdr. `local` is the machine HQ runs on. Any other host is
reached over ssh, which is how HQ on a VPS can run and watch agents on a laptop whenever the
laptop is on and reachable over Tailscale.

## What HQ can do

| Tool | Effect |
|------|--------|
| `harness_session_spawn` | Start an agent on a host. Returns a `session_id`. |
| `harness_session_send` | Submit a prompt, or press `keys` (`down`, `enter`, `esc`, `ctrl+c`) to answer a dialog. |
| `harness_session_wait` | Block until the session goes `idle`, `done` or `blocked`. |
| `harness_session_logs` / `_status` / `_list` | Read output and live state. |
| `harness_session_stop` / `_resume` | Close the workspace, or continue a stopped session. |
| `herdr_hosts` | Which hosts exist and whether each answers right now. |
| `herdr_agents` / `herdr_read` | Read-only view of every agent on a host, including ones you started by hand. |
| `herdr_send` | Prompt (`text`) or press `keys` in an existing pane by pane id or agent name, including one you started by hand. `host` is required. |

The `harness_session_*` tools only change sessions HQ launched. An agent you started yourself
is visible and readable, and `herdr_send` is the one way to steer it: it checks the target
first, sends nothing to an unreachable host or a stale pane id, and refuses text while the
agent is blocked at a dialog (it returns the screen so the dialog can be answered with
`keys`). `hq sessions list|spawn|status|logs|send|stop|resume`
mirrors the tools, and `hq sessions spawn <harness> --host laptop --cwd <path on laptop>`
starts one on a remote host (`--cwd` is required there).

Each session gets its own Herdr workspace labelled `hq <label>`, opened without taking focus,
so it appears in your sidebar when you open `herdr` on that machine.

## Behavior worth knowing

- **Dialogs are not answered for you.** If an agent stops at a startup dialog, `spawn`
  returns the screen and types nothing. Claude Code's workspace-trust prompt defaults to
  "No, exit", so pressing Enter would quit it. Read the screen, then send `keys`. Only
  Antigravity, whose default is verified to be "trust", is accepted automatically.
- **A blocked agent alerts once.** The supervisor (every minute) raises an action-needed
  item when a session blocks, and again only if it blocks in a new state.
- **A finished agent alerts once.** When Herdr reports `done` (the agent finished a turn and
  is waiting), the supervisor posts the last screen to the relay, so the operator hears about
  completion without the session having to exit. `idle` (nothing new happened) does not alert.
- **An unreachable host is not an exit.** If the laptop sleeps or leaves the tailnet, its
  sessions stay `running` and are left alone. They resume being watched when it answers.
  `harness_session_stop` fails for such a session rather than pretending to stop it.
- **Prompts are confirmed, not just sent.** `agent prompt` writes the text and Enter
  together, but a TUI that is not listening yet can swallow the Enter and leave the text in
  its input box. `harness_session_send` and the initial prompt wait for the agent to start
  working, and if Herdr reports `agent_prompt_stalled` HQ presses Enter once more and says so
  in the result. Enter on an empty input does nothing, so this is safe when the agent had
  simply finished first. Read the output before resending anything by hand.
- Exit summaries use the last screen the supervisor captured while the agent was alive.

## Sessions that work on a task

`harness_session_spawn` takes an optional `task_id` (an id or a display id such as
`FR-053`). The task is then the durable record of the work, stored in the session's
`mission_id`, and `harness_session_list` with `task_id` shows every session launched for
it. The task must exist and not be complete, and that is checked before anything is
launched. `harness_session_link` attaches a session that was started without `task_id`;
linking a running session counts as a launch in the table below, and linking one that
already ended only records the link. Each event leaves a comment on the task, and the web Tasks view picks it up at
once. Comments never quote the agent's screen: pane text is untrusted input, and task
comments are read back as trusted, so they point at `harness_session_logs` instead.
Status moves only from the states listed here, and HQ never sets `complete`: the agent
saying it is done is not verification, so a person decides.

| Event | Task status | Notification |
|-------|-------------|--------------|
| Launched or resumed | `to_do`, `blocked` or `ready_for_review` becomes `in_progress` | none; the reply to the launch says it |
| New instruction sent | same as launch; commented only when the status moved | none |
| Working or idle, every sweep | unchanged, no comment | none |
| Blocked on a dialog | unchanged, comment | action-needed item, sent by the next value-bus run (every 5 minutes) |
| Finished a turn (`done`) | `in_progress` becomes `ready_for_review` | relay message, batched with other updates |
| Exited while no other session of the task is running | `in_progress` becomes `blocked` | relay message sent as an interrupt |
| Exited while another session of the task still runs | unchanged | relay message, batched |
| Stopped by hand | unchanged | none |

Each event is recorded right after the registry claim that makes it happen once
(`claim_state_alert`, or the running-to-exited update), and before the exit summary, so
the recorded event survives a daemon restart or a sweep cut off at its timeout. It is
recorded only once.

Offline behavior:

- **Laptop asleep or off the tailnet.** No event is produced and the task does not move. A
  session on that host still counts as running, so another session's exit does not block
  the task. When the host answers again, whatever happened meanwhile is reported on the
  next sweep. A `done` that was followed by an exit while the host was away shows up as
  the exit.
- **Daemon down.** Nothing is watched. On start the supervisor sweeps within a minute and
  reports anything that changed, once.
- **No chat to push to.** Interrupts and value-bus items wait in the relay's pending queue
  until a chat is known. Batched updates are dropped, but the task comments still hold
  them.

A session whose `mission_id` matches no task is treated as having none. Rows written by the
retired mission engine are like that.

## Handing work over from outside (`harness_session_handoff`)

A client that is not a web chat (an MCP client such as another agent) can start tracked work
in one call. `harness_session_handoff` takes `title`, `description`, `acceptance` (alias
`done_criteria`), `harness`, `cwd`, optional `host` (default local), `external_id`, `task_id`,
`prompt`, `space_id`, `initiative` and `drive`. It then:

1. Files an HQ task (acceptance criteria are appended to its description), or reuses the one
   `external_id` already names in that space, or works on `task_id`. The two are alternatives.
2. Starts one session linked to the task, with the task as its goal and `acceptance` as its
   definition of done. With no `prompt`, the agent is told the task title, description and
   acceptance.
3. Creates a web chat thread that owns the session (`owner_thread`), so ready-for-review,
   blocked and exited notices post there and Drive follows the usual gate and
   `herdr.drive_new_watches` (pass `drive: false` to opt out).

The result carries the task id and display id, session id, thread id and
`links.chat` (`/chat?thread=<id>`) and `links.task` (`/tasks?task=<id>`), and `handoff` says
what happened: `started`, `blocked_at_dialog` (the agent stopped at a dialog, so the prompt
was not typed and nothing is running yet; answer with `harness_session_send` keys) or
`existing_session`.

It is idempotent. The same `external_id` returns the same task (`task.deduplicated: true`),
and a task that already has a session its host still reports as running gets no second one.
A session marked running that its host no longer lists is stale and does not block a new one.
A host that cannot be reached when the task already has a running session is reported as
unknown, and no second session starts. If the launch fails (host unreachable, binary missing,
agent never started) the call errors, the unused thread is archived, no session row is written,
and the task stays so a retry reuses it. If the agent stops at a startup dialog instead, the
session stays registered and the task is set to `blocked` with a comment saying so (the comment
does not quote the pane, which is untrusted); the next instruction sent to the session moves it
back to `in_progress`. The check and the launch share one process-wide lock; two separate `hq`
processes sharing a database could still both start a session.

### Launch bounds, preflight and disconnects

A harness that cannot start used to leave the call waiting past an MCP client's roughly 60
second transport limit, with an orphan pane behind it. Now:

- **Preflight (local hosts, conservative).** Before anything is launched, HQ looks for the
  harness binary (`claude`, `agy`, `cursor-agent`, ... or the first word of a profile's
  `command`). It only does so when that word is a plain program name or an absolute path with no
  spaces, and the command has no `=`, `$`, `~`, quote, `&`, `;`, `|`, `<`, `>`, backtick or
  parenthesis. An env prefix (`CLAUDE_CONFIG_DIR=x claude`), `cd x && claude`, a quoted path or a
  `$VAR` is left to the shell and never blocks a launch. Search roots: the profile's `PATH`
  entries (literal `$PATH` and `~` entries skipped), then HQ's own `PATH` (never replaced), then
  `~/.local/bin`, `~/.bun/bin`, `~/.cargo/bin`, `~/.npm-global/bin`, `~/.volta/bin`,
  `~/.local/share/mise/shims`, `~/.asdf/shims`, `~/.local/share/pnpm`, `~/.claude/local`,
  `~/.nvm/versions/node/*/bin`, `/usr/local/bin` and `/opt/homebrew/bin`. A miss fails at once
  with `harness 'claude-code' is not installed on host 'local' (binary 'claude' not found on
  PATH; searched ...)`, listing the roots. Remote hosts are not preflighted: the gate does not
  expose their filesystem.
- **Bounded start.** The wait for the agent to leave `launch_pending` is `herdr.launch_bound_secs`
  (default 25, clamped to 5..=50 so it stays under the transport limit). If the last poll still
  shows `launch_pending` with an `unknown` status, or no agent, HQ reads the pane's last lines,
  closes the Herdr workspace and fails with the host, the harness, what Herdr reported and those
  lines. An agent whose last poll shows `launch_pending: false` is never closed. No row is
  written, so no session claims to be running. Any other failure between the workspace being
  created and the row being recorded (the agent lookup, the database write) also closes the
  workspace; if the close fails the error says so and names the workspace.
- **One launch per agent.** A second launch of the same agent name on the same host (a resume
  retried while the first is still waiting) is refused with "already in progress".
- **Disconnects.** `harness_session_handoff` and every launch (spawn, resume) run on their own
  task, so a client that drops the connection cannot skip the cleanup or leave a thread, a
  workspace or the process-wide lock behind. The bound above is what makes a real error likely
  to arrive before the client gives up. The one launch step that can still outlast the limit
  is accepting a vouched trust dialog (Antigravity), which waits up to 120 seconds.

`task_create` takes the same `external_id`: unique per space, 200 characters at most, a repeat
returns the existing task with `deduplicated: true`. The REST `POST /api/tasks` accepts it too.

### Refusing working directories

Every `cwd` must be an absolute path with no `..` component and none of `~`, `$`, a backtick
or NUL, so the string that was checked is the string Herdr receives.

A `cwd` shaped like a home directory is refused whichever host the session runs on, because
the agent stops at its folder-trust dialog there and nobody is watching: `/`, `/Users/<name>`,
`/home/<name>`, `/root`, `/var/root`, `/opt/hq`, the bare `/Users` and `/home`, and HQ's own
`$HOME`. Trailing and doubled slashes and letter case do not get past it. There is no
per-host home setting yet, so a home that is not shaped like one of these (say `/data/me`) is
only caught through `herdr.spawn_cwd_deny`.

`herdr.spawn_cwd_deny` is a list of path fragments. A `cwd` that contains any entry
(case-insensitive, trailing and doubled slashes ignored on both sides) is refused by
`harness_session_spawn`, `harness_session_handoff`, `harness_session_resume` and
`hq sessions spawn`, with an error naming the entry. It is empty by default. The match is on
the path as written, so a symlink into a denied directory is not caught, and a remote host's
paths are matched as text.

```yaml
herdr:
  spawn_cwd_deny:
    - /clients/acme
    - /private
  handoff_cwd_allow:       # empty means the handoff key is unrestricted
    - /srv/projects
```

### Scoping an MCP client

Give an external client `AGENTHQ_HANDOFF_API_KEY` instead of the full key. It reaches the
read tools (except `harness_session_logs`) plus `task_create`, `task_update`,
`task_comment_add`, `harness_session_spawn` and `harness_session_handoff` (see
`docs/security/WEB_AUTH.md`). It cannot stop, resume, relink, re-goal, send to or read the
output of sessions, delete tasks or write the vault. Send and logs are withheld because the
registry does not record which key started a session. It can also ask HQ's own chat agent a
read-only question with `hq_ask` (see `docs/MCP_ASK.md`), which is not a harness session. That
reply's session has the session-log, Herdr and file-reading tools removed, so the key cannot read
through HQ what it cannot call.

**This key is code-execution equivalent.** A spawned `claude-code` session runs with
permissions skipped, on any host and `cwd` the deny list allows. Set
`herdr.handoff_cwd_allow` to the directories the client may use: a call that arrived on the
handoff key must then have a `cwd` in or under one of them (the gateway marks those calls, a
client cannot set the marker). Left empty, treat the key as shell access to those hosts as
described under "Security model".

Keys sent to a session (`keys` on `harness_session_send`, `herdr_send` and the REST
endpoint) must be logical names matching `^[A-Za-z0-9][A-Za-z0-9+_-]*$`, at most 32
characters, so a key can never be read as a Herdr flag.

## Watching and driving from a web chat

A session launched or linked from a web chat is watched by that chat (`owner_thread`), and
`harness_session_watch(session_id)` makes a chat watch any session. The chat header shows
"Watching N", with each session's task, live agent status and a Drive switch
(`/api/threads/{id}/sessions`, `/api/harness-sessions/{id}/drive`, `.../unwatch`). On a
phone the same panel shows under the chat header with 44px controls, and the button counts
the sessions waiting on an answer.

A session no chat watched starts with Drive on (`herdr.drive_new_watches`, default true)
when it passes the goal gate below; without a usable goal it starts observation-only. Spawn, link and watch take `drive=false` to start with updates only, and on a session the
chat already drives it stops driving. Every other case starts or stays off, so a tool can
never turn Drive back on once the user switched it off:

- A chat watching a session again keeps its switch where the user left it.
- A session taken over from another chat starts with Drive off.
- A turn that has read untrusted content (pane text, the web, mail, vault notes) starts its
  new watches with Drive off: governance marks those calls (`untrusted_turn`).
- A turn the session driver started never starts a new watch driving.
- A turn in a chat that `hq_ask` opened never starts a new watch driving, and neither does a
  handoff made from one.
- `drive=true` on spawn, link and watch is refused. The one way for HQ to turn Drive on is
  `harness_session_mode` (below), which the gate and an untrusted turn both override.

Sessions already watched when this default shipped keep their stored switch; nothing is
backfilled.

A watched session sends nothing to Telegram, Discord or the value bus. The supervisor turns
each finished turn, block and exit into a durable wake on the row (`pm_wake`), and the web
server's session driver (`crates/hq-web/src/session_driver.rs`) answers it in the chat:

- **Watching only.** A short update names the session, what happened and where its task
  stands. It never quotes the screen.
- **Driving.** HQ runs a turn in the chat as the session's project manager: it reads the
  session, then sends the next instruction, approves a permission prompt, resumes an exited
  session, or leaves a working one alone, all toward the linked task's goal. It asks you in
  the chat instead of acting when the step is outside the task, destructive, touches
  credentials, deploys to production unless the task says so, or spends money. It never
  marks the task complete. A driven session also gets a check-in every
  `herdr.driver_checkin_minutes` (default 30) while it runs and its host answers, to catch
  an agent that is stuck. After 48 driver turns in a day a session drops back to updates.
  A chat that is archived or deleted lets go of its sessions, whose events return to the
  relay. The limits in "Limits on driving" below stop a session that keeps being nudged.
  These turns are ordinary chat replies tagged with the session, so their tool calls show
  exactly what HQ sent. Stop ends one like any reply.

A wake is claimed before it is acted on, and a chat with a reply already running is retried
on the next pass (every minute, or at once when the supervisor writes a wake in the same
process). If the web server is not running, wakes wait on the row. Driving approves prompts
after reading pane text, which can carry instructions planted by content the agent read;
turn it on only for sessions whose work you would approve yourself.

### Limits on driving

What wakes a driven turn: a finished turn, a block or an exit that the supervisor turns into a
wake (`pm_wake`), and a check-in every `herdr.driver_checkin_minutes` while the session runs.
What used to stop it: the model deciding the goal was met, and 48 driver turns a day. A model
that judges an unreachable definition of done (a physical-device test, say) as unmet kept
sending another wrap-up instruction, each answered by a finished turn that woke it again.
These limits do not depend on the model deciding to stop. Each one switches Drive off, posts
one notice in the chat (fixed text, never pane text), records a `drive_off` event with actor
`guard` in `harness_session_events`, comments on the linked task, and keeps the reason on the
row (`drive_off_reason`, in the sessions payload and shown under the Drive switch):

| Limit | Default | Setting |
|-------|---------|---------|
| Instructions (text prompts and resume prompts) the driver sends one session, counted since the user last switched Drive on. Claimed in one conditional write before the send, so parallel calls cannot overshoot, and handed back if the send fails. `harness_session_send` and `harness_session_resume` in a driver turn refuse past it, and `herdr_send` is removed from driver turns so it cannot go around it | 8 (1 to 100) | `herdr.driver_nudge_budget` |
| Key presses (permission dialogs, menus) the driver sends one session, on the same terms but a separate, larger allowance so a task with many dialogs is not cut off | 40 (1 to 500) | `herdr.driver_key_allowance` |
| Finished turns in a row that show no new tool activity on the screen the supervisor stored (`Bash(...)`, `Update(...)`, `Ran ...` lines). A tool line the last turn did not show, or one shown more times, counts as work, so re-running a test is not a stall. A finished turn that follows something other than a driver instruction (you typing in the chat) is not judged. A harness that prints none is never judged stalled; the budget covers it | 3 (2 to 20) | `herdr.driver_no_progress_limit` |
| The linked task is complete, or is not in progress at a check-in while the agent is not working (a working agent means someone is typing in the pane). A finished turn itself moves the task to ready for review, so that is not a stop | none | none |
| The session exited or was stopped (Drive goes off in the registry write, and the exit is reported as an update) | none | none |
| Running sessions HQ drives at once. A default-on watch past it starts observing and says so, and `harness_session_mode` from HQ cannot go past it (the user can); a session the user already switched on is never switched off by it | 3 (1 to 50) | `herdr.max_driven_sessions` |
| Running sessions started by an MCP client with no chat (`harness_session_spawn`, `harness_session_handoff`), recorded in `harness_sessions.origin` | 3 (1 to 50) | `herdr.max_mcp_started_sessions` |
| Running sessions started from a chat an `hq_ask` created, including by handoff | 2 (0 to 20) | `herdr.max_ask_spawned_sessions` |
| Full-mode `hq_ask` questions waiting at once | 2 (1 to 10) | `herdr.max_full_asks` |

Known harmless prompts are not left to the model. Claude Code's optional feedback survey
(`How is Claude doing this session? (optional)` with `0: Dismiss`) used to stall a driven loop
until a person pressed `0`. The supervisor (`harness_session::dismiss`, called from
`session_supervisor.rs` on its minute sweep) now dismisses it by text match, not judgement:

- Both exact lines (the title and `1: Bad  2: Fine  3: Good  0: Dismiss`) must sit within the last
  12 non-empty lines of the screen, so the same words in older scrollback are ignored. The screen
  is read again just before the key goes out, so a survey that already closed gets no stray `0`.
- The key is the constant `0` (a bare `0` works with `agent send-keys` on a real claude-code
  session). A tail that shows a dialog needing a human is never touched: `do you trust`, `do you
  want to`, `would you like`, `permission to`, `requires permission`, `enter to select`, `esc to
  cancel`, `(y/n)`, `allow this`, `no, exit` and `yes, proceed` all veto. The word "permission"
  alone does not, because Claude's bypass-mode footer (`bypass permissions on`) always sits in
  the tail. A prompt without a recognised dismiss option is never answered, and trust dialogs
  still default to "No, exit" and are reported.
- Nothing is typed while Herdr reports the agent `working` (the text would be the agent's own
  output), and nothing is typed when the input line below the survey holds any text. The input
  line is recognised by its glyph: Claude Code's `❯` (U+276F), `>`, or either inside a `│ … │`
  box. A ghost suggestion looks like a draft and cannot be told apart from screen text, so it
  counts as one and the survey is left for a person.
- After a dismissal the supervisor remembers a hash of the screen tail
  (`harness_sessions.last_dismiss_tail`). If the next sighting has the same tail, the key did
  nothing: HQ types nothing more, records a `dismiss_cap` event ("prompt did not close") and
  posts the same single notice as the cap. Together with the draft check, a survey lookalike
  can cost at most one stray `0`. Switching Drive on clears the hash.
- Host calls are bounded: at most one screen re-read and one key send per session per sweep, each
  with a 3 second timeout. The screen is read again just before the key goes out, so a survey
  that already closed gets no stray `0`. A send Herdr refused is handed back; a timeout or lost
  connection keeps the claim spent (the key may have landed, and a second `0` could reach the
  input box).
- A finished turn is never silenced by a dismissal: its alert and driver wake fire as usual,
  even if Herdr changes the session's state after the keypress. Only the blocked alert the
  survey itself caused is skipped.
- Only sessions with Drive on and a watching chat. An observe-only session is never typed into.
- Dismissals have their own counter (`harness_sessions.dismissals`, refilled when the user
  switches Drive on) and cap of 5. They spend neither the instruction budget nor the key
  allowance. The sixth sighting sends nothing, records a `dismiss_cap` event and posts one notice.
- Each dismissal is a `prompt_dismissed` event with actor `guard`; the linked task gets one
  comment per session, not one per dismissal.

A second finished turn with no instruction sent since the driver last answered one is posted as
a plain update, not driven. Anything other than the driver sending the session something (the
user typing in the chat, the Sessions page) re-arms that. Notices never create a wake.

Only the user switching Drive on (the panel switch or `POST .../drive`) refills the budgets and
clears the stall streak. A driver turn or an `hq_ask` reply cannot turn Drive on with
`harness_session_mode`, and an HQ-initiated switch does not refill anything: if HQ switches Drive
back on after a budget stop, the next wake switches it off again.

Driver turns (the session driver, not sub-agent follow-ups) may not start, hand off, attach or
link sessions, and cannot type into a pane except through the metered tools. Attach and link are
also refused in `hq_ask` replies. Sessions started over the handoff key never start driven. After a guard stops Drive, the
session keeps running untouched and later events arrive as ordinary updates.

`herdr.drive_new_watches` stays `true`: a handoff or spawn whose goal and definition of done pass
the gate starts driven, bounded by the limits above. Set it to `false` for every new watch to
start observing.

### Sessions HQ spawned cannot start more sessions

Every pane HQ launches has `HQ_SESSION_ID=<session id>` in its environment. An MCP client in
that pane can pass it on, and the gateway then marks the call as coming from an HQ-spawned
session (the marker only adds restrictions, so the caller cannot gain anything by setting or
omitting it). A marked call to `harness_session_spawn` or `harness_session_handoff` is refused,
and `hq_ask` is refused in `mode: full` (`read_only` still works), with "HQ-spawned sessions
cannot start further sessions or full-mode asks; ask the owner".

- **HTTP `/mcp`**: the client config sends header `x-hq-session-id` filled from the variable.
  In the Claude Code MCP entry for `agent-hq` (verified against Claude Code 2.1.x on 2026-10-04:
  it expands the variable in HTTP headers, and a blank value arrives as an empty header):
  `"headers": {"x-hq-session-id": "${HQ_SESSION_ID:-}"}`. A blank value means no marker. A live
  session started by HQ with this entry was refused both `hq_ask` in full mode and
  `harness_session_spawn`. Add the header to every Claude profile's `agent-hq` entry
  (`~/.claude.json`, and `.claude.json` in each `CLAUDE_CONFIG_DIR`); HQ does not edit client config.
- **stdio `hq mcp`**: reads `HQ_SESSION_ID` from its own environment when the client passes it on.

A marked call also cannot use `harness_session_resume` or `config_manage`, and can type
(`harness_session_send`, `herdr_send`) only into its own session.

**This depends on that client config, which HQ does not write, so containment fails open
without it** (see `TECHDEBT.md`). HQ does not pass a per-pane `--mcp-config`. Without the marker
these caps still hold, whatever the client says about itself: a watch past
`herdr.max_driven_sessions` does not drive; a session started from a chat that `hq_ask` created
never starts driven, and is capped at `herdr.max_ask_spawned_sessions`; sessions an MCP client
starts with no chat are capped at `herdr.max_mcp_started_sessions`; full-mode asks wait at most
`herdr.max_full_asks` at a time; an `hq_ask` reply loses `config_manage`.

### The Sessions page

`/sessions` in the web app lists every registry row with its task, host, goal, live agent
status and whether its host answers (an unreachable host shows "host unreachable", never
"exited"). Picking one shows the pane text (refreshed every 3s), a prompt box, the keys that
answer a dialog, an Adopt button, and the command that opens Herdr on that machine
(`herdr`, or `ssh <host> -t herdr` for a remote host; the page notes that this assumes the host
name is an ssh alias on your machine). `ctrl+c` sits apart from the other keys and needs a
second tap. A task's drawer lists its sessions and links here. All of it sits behind the same
web auth as the rest of `/api`, and `send` and `adopt` under `/api/harness-sessions/` must
also carry `X-HQ-Client` (the web app sends it; scripts must add it, or get a 403). `drive`, `goal` and
`unwatch` predate the header and stay open to cached PWAs that do not send it yet. Errors the
endpoints do not recognise are logged and answered generically, never with ssh or Herdr stderr.

- `GET /api/harness-sessions?task_id=&status=&host=` is `harness_session_list` plus the task
  and goal. `GET /api/harness-sessions/{id}` is one row, live.
- `GET /api/harness-sessions/{id}/screen?lines=N` (default 80, at most 500) reads the pane
  while the agent runs and returns the last stored snapshot after it ends (`source`). A live
  read is one Herdr call (one ssh round trip on a remote host). Concurrent reads of one
  session share a single in-flight call, and a live result is reused for about a second;
  a failed read or a stored-snapshot answer is never reused, and a send, stop or resume
  drops what is held. A session the registry no longer lists as running is answered from its
  snapshot without calling Herdr. A live answer also carries `herdr_source` (`recent-unwrapped`
  or `visible`; null for a snapshot). While an agent is working, Herdr refuses a `recent` read
  of more than about 25 lines with `agent_not_idle`; `HerdrHost::read` then retries that one
  error once as `--source visible` with the same line count, so the page and the supervisor
  stay live instead of falling back to a stale snapshot. Other errors (`agent_not_found`, an
  unreachable host) are not retried, writes never take this path, and idle agents keep the
  deep `recent-unwrapped` read. The supervisor was left on the same read rather than moved to
  `--source detection`: `detection` is Herdr's own classifier input with no documented wrapping
  or depth contract, and the stored snapshots and progress hashing assume unwrapped text. The page waits for each response before polling
  again and backs off (up to 30s) while requests fail.
- `POST /api/harness-sessions/{id}/send` takes exactly one of `text` or `keys`, with the same
  rules as `harness_session_send`: text is refused with a 409 while the agent waits at a
  dialog, and a session that is not running or whose host is down is refused too. Each send
  is a `sent` row in `harness_session_events` (key names, or the length of the text, never
  the text itself).
- `POST /api/harness-sessions/{id}/adopt` with an optional `thread_id` makes that chat watch
  a session nobody watches, or creates a chat named "Session: <label>" when omitted. It
  starts observation-only. Repeating it, or naming the chat that already watches it, changes
  nothing; a session another chat watches is a 409 until that chat unwatches it.

### Goal and definition of done (the drive gate)

HQ steers a session only while it can state what the session is for and how anyone could
tell it is finished. Both live on the session row (`goal`, `done_criteria`, migration 063),
for tracked-task and ad hoc sessions alike. Set them with `goal` and `done_criteria` on
`harness_session_spawn`, with `harness_session_goal`, or from the web panel
(`POST /api/harness-sessions/{id}/goal`). A session spawned or linked with a `task_id` and no
goal takes the task's title and description as its goal; the definition of done always has to
be stated.

The gate (`hq_db::harness_drive_gate`) is deterministic, no model involved. Each field must be
non-empty, not a placeholder (`TBD`, `when done`, `it works`, ...), and have minimum substance
(goal: 4 words and 20 characters; definition of done: 3 words and 15 characters), and the
definition of done must not just repeat the goal. When the gate fails, HQ stays
observation-only and the refusal names what is missing, in the tool result, the REST 409 and
the Watching panel. Editing the goal so it no longer passes switches a driven session to
observing on the spot, and the driver re-checks before every turn. The driver prompt carries
the goal and definition of done. A session that exits or goes idle has not thereby met its
goal: the task moves to review at most, and a person completes it.

Every goal change, drive change, refusal and attach is a row in `harness_session_events` with
the goal and criteria in force at the time (`registry::list_events`).

### Attaching and switching mode mid-chat

- `herdr_agents` lists what Herdr runs, including agents started by hand.
  `harness_session_attach(agent, host)` starts tracking one and makes the chat watch it,
  observation-only. An agent already tracked is reused, and another chat can take it over
  (Drive off). An unreachable host or a missing agent is an error and records nothing. Family
  guests cannot attach.
- `harness_session_mode(session_id, mode)` switches HQ between `drive` and `observe` for a
  session the chat watches. `observe` always works and takes effect at once: the driver reads
  the row before every turn, so at most the step already in flight finishes. `drive` needs a
  live agent (an ended session or an unreachable host is refused, since HQ cannot confirm
  what it would steer), a passing gate, and a turn that has not read untrusted content. The
  Drive switch in the panel is the same mode change.
- Switching mode never touches the agent: it is not paused or stopped. `harness_session_stop`
  is the only tool that ends it, and it is never called as a side effect of a mode change.
  After a resume the row keeps its goal, so Drive can be turned on again.

## Custom harness profiles

`herdr agent start` runs the agent CLI Herdr knows by name, so a wrapper script (one that
selects an account, sets a config directory, or adds flags) cannot be launched that way. A
profile in `config.yaml` names such a launcher and borrows everything else from a built-in
harness:

```yaml
herdr:
  harness_profiles:
    my-claude:                 # the name callers pass as `harness`
      base: claude-code        # resume, trust and token behavior come from this harness
      command: my-claude-wrapper   # typed into the pane's shell; must end up running claude
      args: ["--dangerously-skip-permissions"]   # optional; replaces the base args on a fresh spawn
      env:                     # optional; set for the pane's shell, so for the CLI too
        CLAUDE_CONFIG_DIR: /home/you/.claude-work
```

`harness_session_spawn`, `hq sessions spawn` and session-mode dispatch all accept the profile
name, and `harness_session_resume` finds it again from the stored session. For a profile with
a `command`, HQ opens the workspace with `env`, types the command and its quoted arguments into
the pane (`pane run` submits with Enter in the same write), waits for Herdr to recognize the
agent, then names it so status, send, wait and the supervisor address it like any other
session. If no agent appears within two minutes the spawn fails with the pane's screen and
the workspace is closed; the usual cause is a command that is not on the `PATH` of a
non-interactive shell on that host, or a wrapper that does not end up running the base CLI.
`command` is typed as written (a leading `~` expands) and its arguments are quoted for a
POSIX shell. A profile without `command` launches the base CLI through Herdr with the profile's
`args` and `env`. A resume uses the base harness's resume arguments. A profile may reuse a
built-in name to change how that harness starts.

A separate Claude Code account needs no wrapper: point `CLAUDE_CONFIG_DIR` at that account's
config directory and Herdr starts `claude` in it. Write the path out in full, since `env`
values are passed through as written and `~` is not expanded:

```yaml
    claude-work:
      base: claude-code
      env: { CLAUDE_CONFIG_DIR: /Users/you/.claude-work }
```

The spawn result's `harness` field names the profile the session was started with. An unknown
name fails before any workspace is created, listing the known harnesses and profiles.

Profiles run shell commands on the host, so they belong to whoever can edit `config.yaml`.
The gate on a remote host needs no change: it already allows `pane` and `agent`, which cover
`pane run` and `agent rename`.

## Local setup (the machine HQ runs on)

Install Herdr and make sure its server is running. On a Linux server, run it as its own
service so restarting HQ does not stop running agents:

```bash
curl -fsSL https://herdr.dev/install.sh | HERDR_INSTALL_DIR=/usr/local/bin sh
sudo cp deploy/herdr.service /etc/systemd/system/ && sudo systemctl enable --now herdr
```

`deploy/herdr.service` runs `herdr server` as the `hq` user with the same hardening as
`hq.service`. Every pane inherits that unit's environment, so agents there see the LLM provider
keys (`openrouter.env`) and the GitHub token (`gh.env`); the Google Workspace credentials
(`gws.env`) are deliberately not loaded into it. On a workstation, opening `herdr` once starts the server.

Optional config in `config.yaml` (all defaults shown):

```yaml
herdr:
  binary: herdr            # binary on this machine
  session: null            # named Herdr session; null is the default session
  default_host: local
  launch_bound_secs: 25    # how long a launch waits for the agent (5-50)
  command_timeout_secs: 30
  driver_nudge_budget: 8        # instructions the driver may send one session (1-100)
  driver_no_progress_limit: 3   # finished turns without new tool activity before Drive stops (2-20)
  max_driven_sessions: 3        # running sessions HQ drives at once (1-50)
  max_ask_spawned_sessions: 2   # running sessions hq_ask chats may start (0-20)
  max_mcp_started_sessions: 3   # running sessions MCP clients with no chat may start (1-50)
  driver_key_allowance: 40      # key presses the driver may send one session (1-500)
  max_full_asks: 2              # full-mode hq_ask questions waiting at once (1-10)
```

## Adding a remote host (for example a laptop)

Run these steps once. `hq-host` is the machine running HQ, `laptop` runs Herdr.

1. **Install Herdr on the laptop** and open it once so its server runs.
2. **Install the gate on the laptop.** It is a forced command that runs only the `herdr`
   CLI, taking its arguments as a JSON array on stdin so no shell parses prompt text:

   ```bash
   install -m 755 scripts/hq-herdr-gate ~/.local/bin/hq-herdr-gate
   ```

   It allows `agent`, `pane`, `workspace`, `tab` and `status`, and refuses `server`,
   `session`, `machine` and the `--machine`/`--remote` flags.
3. **Create a dedicated key on `hq-host`**, as the user HQ runs as:

   ```bash
   sudo -u hq ssh-keygen -t ed25519 -N '' -C 'hq herdr-laptop' -f /opt/hq/.ssh/herdr_laptop
   ```
4. **Authorize it on the laptop**, one line in `~/.ssh/authorized_keys`, pinned to
   `hq-host`'s Tailscale address and to the gate:

   ```
   restrict,from="<hq-host-tailscale-ip>",command="/home/you/.local/bin/hq-herdr-gate" ssh-ed25519 AAAA... hq herdr-laptop
   ```

   `restrict` turns off ptys, port forwarding and agent forwarding. The laptop needs SSH
   Remote Login enabled (macOS: System Settings, General, Sharing).
5. **Trust the laptop's host key** from `hq-host` so ssh never prompts:

   ```bash
   sudo -u hq sh -c 'ssh-keyscan -H <laptop-tailscale-ip> >> /opt/hq/.ssh/known_hosts'
   ```
6. **Tell HQ about it** in `config.yaml`:

   ```yaml
   herdr:
     hosts:
       laptop:
         ssh: you@<laptop-tailscale-ip>
         identity_file: /opt/hq/.ssh/herdr_laptop
   ```
7. **Check it** from `hq-host`:

   ```bash
   printf '%s' '["agent","list"]' | sudo -u hq ssh -i /opt/hq/.ssh/herdr_laptop \
     -o IdentitiesOnly=yes you@<laptop-tailscale-ip> hq-herdr-gate
   ```

   then `herdr_hosts` from any HQ session should show `laptop` as reachable. To exercise HQ's
   own ssh path (not just ssh by hand) run the opt-in diagnostic, which also proves prompt text
   reaches Herdr as a plain argument:

   ```bash
   HQ_TEST_HERDR_SSH=you@<laptop-tailscale-ip> HQ_TEST_HERDR_KEY=/path/to/key \
     cargo test -p hq-tools herdr::tests::real_ssh -- --ignored --nocapture
   ```

To revoke access, delete that one line from `authorized_keys`.

### Connection reuse (`herdr.ssh_multiplex`)

Every Herdr call on a remote host is one ssh command, and a fresh Tailscale ssh handshake
costs seconds, so HQ keeps one connection per host open between calls (`ControlMaster=auto`,
`ControlPersist=120`, sockets in `~/.hq/run/ssh`, mode 0700, one per host). It also sets
`ServerAliveInterval=15` and `ServerAliveCountMax=2`, so a master whose laptop went to sleep
exits within about 30s instead of holding later calls until their deadline. The key,
`IdentitiesOnly`, `BatchMode`, the forced command and the gate are unchanged: sshd still runs
your `authorized_keys` command for every session on the shared connection.

```yaml
herdr:
  ssh_multiplex: true   # default; false opens a fresh connection per call
```

The control socket is a bearer credential for the gated key: anything that can open it can
run gated Herdr calls as that key without the key file, so it sits inside the same boundary
as the key. The directory is 0700 and holds nothing else; keep `identity_file` mode 0600 and
owned by the HQ user.

If the socket directory cannot be made private or its path would be too long for a unix
socket, HQ logs a warning and uses plain connections. If ssh itself reports a control socket
problem before a session opens, a read-only call (`status`, `agent get`, `agent list`,
`agent read`) is retried once on a fresh connection. A write (send, prompt, launch) is never
retried, since the gate may already have run it. A master that hangs
rather than failing is bounded by the normal command timeout. Run HQ with
`RUST_LOG=hq_tools::herdr=debug` to log the elapsed milliseconds of every Herdr invocation.

## Security model

The gate limits what the key can ask for, but Herdr can run arbitrary commands inside panes
by design (`pane run`, and any agent it starts can run code). Treat the key as shell access
to the laptop as you, scoped to Herdr's own surface and to one source address. Consequences:

- Keep the private key readable only by the HQ user and out of backups you do not control.
- Anyone who can call HQ's `harness_session_*` tools can start an agent on the laptop, and
  anyone who can call `herdr_send` can type into any agent pane there, including ones you
  started by hand. Do not expose those tools to callers you would not give a shell.
- Herdr's API is a local unix socket and its documentation describes no separate
  authentication, so access control is the socket's file permissions plus the ssh key above.

## Troubleshooting

| Symptom | Likely cause |
|---------|--------------|
| `herdr_hosts` shows `unreachable` with `Permission denied` | Key not in `authorized_keys`, wrong `from=` address, or `identity_file` unreadable by the HQ user. |
| `unreachable` with `Host key verification failed` | Run step 5. |
| `unreachable` with `timed out` | Laptop asleep, off the tailnet, or Remote Login disabled. |
| `gate_denied` | The call used a subcommand the gate does not allow. |
| `agent_not_ready` at spawn | Agent is at a dialog. Read the screen from the spawn result and answer with `keys`. |
| Spawned Claude replies `Login expired` | The user account on that host is not logged in to that CLI. |
