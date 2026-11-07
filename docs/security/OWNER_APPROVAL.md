# Owner approval in chat sessions

In the default permission mode, a chat the owner drives asks the owner before a tool call that
changes things outside the vault and tasks. The call waits until the owner approves exactly that
tool with exactly those arguments in their inbox (web app, web push, phone app).

## Where it applies

| Session | Asks the owner |
|---------|----------------|
| Web chat, an owner turn | Yes |
| Telegram and Discord chat turns | Yes |
| Sub-agents a chat starts (`spawn_subagents`), at any depth | Yes, under the chat's approvals |
| `hq_ask` from an MCP client | No. The connector gate already asks for the calls it guards |
| `hq chat` in a terminal | Not yet (see below) |
| Daemon tasks, `/watch` firings, web chat Drive and follow-up turns, task verification | No |
| Coding agents already running in the host | No. Starting one asks (below); after that its dialogs follow its command policy (`docs/AGENT_SESSIONS.md`) |

A session with no approval store (`owner_approval: None` in `NativeHqHooks`) never asks: its calls
run as they would with `governance.ask_owner: false`. That is why the turns in the "No" rows are
limited before they start instead. A watch can only be set by the owner: `/watch` and `resume` are
refused for a Telegram guest and in the Discord family channel and its threads, `watch_create` is
denied to guests and asks the owner in a chat. Before every firing and every sub-agent follow-up
turn the relay checks again that the chat is the owner's, and stops a watch (or hands the
follow-up to the owner's inbox) when it is not.

Only `PermissionMode::Default` asks. `accept-edits` (the `workspace-write` preset), `read-only`,
`plan` and `bypass` behave as before. In Telegram and Discord, `/permission workspace-write` pins a
chat to a mode that does not ask. `governance.ask_owner: false` in `~/.hq/config.yaml` turns it
off everywhere.

## Which calls ask

`needs_owner` in `crates/hq-agent/src/governance/owner_approval.rs`:

- files and commands on the machine: `bash`, `write_file`, `edit_file`, `rollback_file`, `file_edit_batch`
- git and GitHub: `git_commit`, `git_pr`, `github_write`
- HQ itself: `config_manage` (except `get`), `self_update_install`, `self_update_rollback`, `skill_manage`, `slash_command_manage`, `task_delete`
- scheduling an unattended turn: `watch_create`. A watch firing runs later with the owner's tools
  and nobody to ask, so the model (or text it read) setting one up is asked about once, here
- other machines and agents: `host_add`, `host_send`, `agent_send_message`, and starting a coding agent (`harness_session_spawn`, `harness_session_handoff`, `agent_delegate`), which is as strong as `bash`. Steering an agent already started (`harness_session_send`, `harness_session_answer`) does not ask
- exports to a path: `convert_from_markdown`, `vault_export` and `vault_export_pdf` when given `output`
- the owner's other accounts: `google_workspace` and every remote MCP server (`remote_mcp:`)

Reads never ask, and neither do notes and tasks, which the owner can undo from the vault and the
task history. This list is narrower than the claude.ai connector's (`CONNECTOR_CONFIRM_TOOLS` in
`crates/hq-mcp/src/gateway.rs`), which also guards reads because its caller is remote.

## How an approval is bound

- The request is a value item whose dedup key is `owner-approval:<chat>:<tool>:<digest>`. The chat
  comes from the runtime (`web:<thread>`, `relay:<platform>:<chat>`), never from the model. The
  digest is SHA-256 of the arguments with keys sorted and the runtime's `_hq_*` keys left out
  (`hq_core::approval_key::args_digest`, shared with the connector).
- The owner's approval is signed with the install's approval key over the item id and the dedup
  key, so a database write cannot move it to another call.
- An approval lets exactly one call through and lasts 24 hours. A decline lasts as long and is
  reported to the model, which is told not to retry the same call.
- The call waits up to 60 seconds in the turn. If the owner has not answered by then, the model is
  told the call is waiting and to make exactly the same call again once the owner approves it.
- One chat may have at most 5 requests waiting. A web chat that has no thread yet shares the scope
  `web:new` with other new chats until its first turn creates one. Arguments too long to review in a message are refused.
- A store that cannot be read stops the call.
- The call itself (the tool and its arguments without the `_hq_*` keys) is kept beside the item
  (`owner_approvals`), so every app quotes it verbatim. Whether it loses something, and the verb
  the red "Allow {verb}" names, comes from the tool and its arguments through
  `hq_core::losing::tool_verb` (a tool or action named delete, remove, stop and the like; for
  `bash`, the commands it runs, `sudo` and `sh -c` included), never from the model's words.
- Clearing all updates never touches a request, and Undo never brings back a declined one.

## Not covered yet

- `hq chat` in a terminal: an inbox approval for your own terminal is awkward, so a terminal prompt
  is the better design. Tracked in `TECHDEBT.md`.
- A relay turn whose `vault.db` cannot be opened runs without asking (it logs a warning), because
  there is no inbox to ask through.
