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

A session with no approval store (`owner_approval: None` in `NativeHqHooks`) never asks: until it
reads outside text its calls run as they would with `governance.ask_owner: false`. That is why the
turns in the "No" rows are limited before they start instead. Once such a session has read outside
text (it is tainted, `PROMPT_INJECTION.md`) it never starts, briefs or types into a coding agent,
and an unattended one (`/watch` firings, Drive and follow-up turns, marked
`NativeHqHooks::unattended`) refuses every call that would ask in a chat, leaving an item in the
owner's inbox saying what it left for them. A watch can only be set by the owner: `/watch` and `resume` are
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
- other machines and agents: `host_add`, `host_send`, `agent_send_message`, and starting a coding agent (`harness_session_spawn`, `harness_session_handoff`, `agent_delegate`), which is as strong as `bash`. Steering an agent already started (`harness_session_send`, `harness_session_answer`) does not ask in a clean session; see below for a tainted one
- exports to a path: `convert_from_markdown`, `vault_export` and `vault_export_pdf` when given `output`
- the owner's other accounts: `google_workspace` and every remote MCP server (`remote_mcp:`)
- the owner's phone calendar: `calendar_update_event` and `calendar_delete_event`. Unlike the rest of
  this list they ask in every permission mode but bypass, and a clean chat is no exception
  (`always_ask_reason` in `tainted_calls.rs`, wording from `hq_tools::calendar::describe_modification`).
  The card shows the exact arguments (the event link, its title and start as listed, the new
  values) and a line such as "If approved, it would change the event "Dentist" on the owner's phone
  calendar: start to 2026-02-11T16:00." The digest covers every argument, so an approval is for this
  event and this change only. With no approval store they do not run. Creating an event
  (`calendar_create_event`) still asks only once the turn is tainted.

Reads never ask, and neither do notes and tasks, which the owner can undo from the vault and the
task history.

Once the session has read outside text, a few more calls ask, in every permission mode but bypass
(`tainted_reason` in `crates/hq-agent/src/governance/tainted_calls.rs`): `harness_session_send`,
`web_fetch` of a host the owner did not name in this chat or of an address carrying more than 200
bytes of path and query, and `web_search` with a query over 150 characters. A tainted turn may type
into or answer only a coding agent its own web chat watches. The reasoning is in
`PROMPT_INJECTION.md`, rules 4 and 5. This list is narrower than the claude.ai connector's (`CONNECTOR_CONFIRM_TOOLS` in
`crates/hq-mcp/src/gateway.rs`), which also guards reads because its caller is remote.

## How an approval is bound

- The request is a value item whose dedup key is `owner-approval:<chat>:<tool>:<digest>`. The chat
  comes from the runtime (`web:<thread>`, `relay:<platform>:<chat>`), never from the model. The
  digest is SHA-256 of the arguments with keys sorted and the runtime's `_hq_*` keys left out
  (`hq_core::approval_key::args_digest`, shared with the connector).
- A call that acts on stored text its arguments do not carry is bound to that text too
  (`hq_tools::approval_note::stored_text` and `bound_digest`). Today that is
  `harness_session_handoff` with no `prompt` of its own, which types the task's title, description
  and acceptance criteria into the agent: the request quotes that prompt as it stands, its hash is
  folded into the digest, and the text is read again just before the call runs. If the task was
  edited after the owner approved, nothing runs and the next call asks about the new text. The
  connector's gate does the same.
- The owner's approval is signed with the install's approval key over the item id and the dedup
  key, so a database write cannot move it to another call.
- An approval lets exactly one call through and lasts 24 hours. A decline lasts as long and is
  reported to the model, which is told not to retry the same call.
- An answer names exactly one request. The web inbox and the phone answer by the item's full id
  (`hq_db::value_items::answer_by_id`), and an approval there is signed whether or not the item had
  been delivered yet. A request already answered, expired or spent answers 409 and is left as it
  was, so approving again never re-arms a spent approval. The short refs a chat button or
  `hq queue approve` take are prefixes, and one that matches more than one waiting request is
  refused instead of approving them all.
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

## Fields on an approval item (for the apps)

`GET /api/notifications` puts these on `metadata.approval`, all set by the runtime from the call:

- `loses` and `verb`: from `hq_core::losing::tool_verb`.
- `consequence`: one plain sentence (`hq_core::losing::consequence`). It follows the tool's name
  (a fixed table: `vault_delete` says the note goes to the Trash and can be restored for 30 days,
  `task_delete` says it cannot be undone), the host the runtime parsed from a fetch ("HQ will open
  example.com and read the page."), or the verb found in a shell command HQ is asked to run. It
  never follows an `action` or inner `tool` argument the model supplied, and it is absent when HQ
  cannot say it truly (the card then shows the request itself).
- `site`: the host of a `web_fetch` call.
- `site_approvals`: how many times the owner answered "Allow once" for that site, across chats
  (`hq_db::fetch_sites::allowed_once_for_host`, table `site_allowed_once`). Only inbox answers
  write it, so a helper session's prompts never count; "Allow all" and a decline never change it.
  It is display only: the apps offer "Allow all for this site" once it reaches two, but the
  server accepts `approve_site` at any count.
