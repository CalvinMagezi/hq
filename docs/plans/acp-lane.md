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

1. A pane kind in `hq-host` that owns a `Connection` under the process sandbox and answers the same
   `agent.*` calls as a pty pane.
2. A transport field on harness specs and profiles; `hq doctor` runs `initialize` only.
3. HQ MCP injection through `mcpServers`, and edited files into task activity.
4. Move the lanes over one at a time, keeping the pty path as the fallback.
