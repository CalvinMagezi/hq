# ACP lane

Status: first slice. The `hq-acp` crate exists and is tested; the host does not use it yet.

## Why

`hq-host` reads an agent's state from its terminal screen: rule files per agent, a debounce so a
flicker is not taken for a change, dialogs parsed from the screen, prompts typed as keystrokes. Each
new agent needs a rule file, and a change to an agent's interface breaks it. Agents that speak the
Agent Client Protocol (ACP, JSON-RPC over stdio) report the same facts directly: a turn started or
ended and why, the dialog it waits at with its options, the files a tool touched, token use, and
structured failures.

## What is here

`crates/hq-acp` turns an agent's message stream into the facts the host exposes in `AgentInfo`
(`state`, `done`, `state_seq`, `pending`, `pending_seq`). It is synchronous, to sit beside the pty
code, and tolerant: agents differ in field names, so messages stay `serde_json::Value` until a small
typed view reads the few fields HQ needs. The official Rust SDK was not used: it is async, strict
about message shape (agents were seen sending extra and renamed fields), and pulls a runtime into a
crate that has none.

- `wire` parses and builds JSON-RPC lines.
- `update` reads `session/update` notifications and tool calls, gathering file paths from every place
  an agent puts them.
- `dialog` reads `session/request_permission` into options (with `remembers()` for the "always"
  kinds that only the owner may pick) and builds the answer.
- `tracker` follows a session: working, blocked, idle; the open dialog and its number; the edited
  files; how the turn ended (stop reason and usage, or a failure classed as auth, billing or other).
- `client` runs one agent process: handshake, new session, prompt, cancel, answer, and a `pump` the
  host calls from its own loop.

Tests: unit tests, a scripted agent process (`fake-acp-agent`) for the process paths, and recorded
sessions from Hermes, agy, Copilot CLI, Cursor and opencode (`tests/fixtures`, paths and menus
trimmed) replayed through the tracker, including cancel and a provider error.

## Observed differences the code absorbs

- Relative paths (Hermes) against absolute ones; `name` and `args` tool calls (agy) against `kind` and
  `locations` (Hermes, Cursor) against a diff in `rawInput` (Copilot).
- Permission options: Copilot offers "always allow", Hermes and agy do not; Cursor edits without asking.
- A provider failure arrives as a JSON-RPC error on the prompt; a missing login as `-32000` on
  `session/new`.
- Both `loadSession` and `load_session` spellings in capabilities.

## Next

1. `hq doctor` runs `initialize` for each ACP agent.
2. HQ MCP injection through `mcpServers`, and edited files into task activity.
3. Move the lanes over one at a time, keeping the pty path as the fallback.
4. Bring ACP agents back after a host restart with the protocol's session resume.

## Host pane kind (second slice)

`hq-host` can now start an agent with `transport: "acp"` (`agent.spawn`, `SpawnSpec::transport`). It
answers the same calls as a terminal agent, so nothing above the host changes:

- `state`, `done`, `state_seq`, `pending`, `pending_seq` and `agent_session_id` come from the
  protocol. Every state change is an event in the same log, so `events.poll` and the supervisor see
  ACP agents like any other. The rule reads `acp`.
- A permission request is a dialog: options numbered from 1, `remember` set for "always" kinds, and
  `decline` picks the agent's reject option (or cancels). The owner-only rule for remember options
  applies unchanged.
- `read` and `read_since` serve a transcript: the agent's text, `> prompt`, `tool: ...`, `ask: ...`
  and `end: ...` lines. Cursors work the same way.
- `prompt` queues behind a running turn (an ACP agent takes one prompt at a time) and is refused at a
  dialog. `esc` or `ctrl+c` cancel the turn; any other raw input is `unsupported`.
- The process runs under the same sandbox as a terminal agent, with pipes instead of a pty, in its own
  process group so stopping it stops what it started. A failed start returns the agent's own words
  (`Authentication required`, a provider error) with the stderr line that explains it.
- Not done: an ACP agent is not brought back after a host restart (the protocol's session resume is
  not wired), and the harness specs in `hq-tools` do not select the transport yet.

### Findings from running real agents through it

Under the open sandbox profile agy and Copilot CLI complete a turn (agent kinds `antigravity` and
`copilot` give them their state folders); Hermes completes one unsandboxed. Hermes does not work
under the sandbox, and the reason matters for the design: `hermes acp` is a thin client of a Hermes
gateway daemon (a websocket on a unix socket under `~/.hermes`) that it starts detached when none is
running, and the gateway, not the client, runs the tools. The sandbox stops the start, and a gateway
that already runs outside it would run the tools outside it. So Hermes over ACP is for hosts that do
not require a sandbox, and Hermes stays on the terminal transport where one is required.

Run a real agent through the host with `HQ_ACP_LIVE="<command>" cargo test -p hq-host --test
acp_live -- --ignored --nocapture` (see the file header for the sandbox switches).

## Choosing the transport (third slice)

A harness profile opts in with `transport: acp` (and optionally `acp_args`); see "Running an agent over
ACP" in `docs/AGENT_SESSIONS.md`. `HarnessProfileConfig` gained `transport` and `acp_args`, a profile
that names an unknown transport or combines ACP with a wrapper `command` is refused when it resolves,
and `LaunchRequest.acp_argv` carries the server command to `NativeBackend`, which sends
`agent.spawn` with `transport: "acp"`, skips the screen wait and the hook, MCP and policy
plumbing, and treats the agent as ready once the host has opened the session. `submit` over ACP never
presses Enter a second time: the prompt is delivered, not typed.
