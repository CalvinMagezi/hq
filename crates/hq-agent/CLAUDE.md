# Agent sessions, sub-agents and governance

Notes for working under `crates/hq-agent/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Agent session | `crates/hq-agent/src/session/mod.rs` |
| Sub-agent dispatch | `crates/hq-agent/src/agents/service.rs`, `crates/hq-agent/src/agents/tool.rs` |
| Context fold (long chats as append-only summary blocks over `chat_messages` plus a raw tail, `context_fold:` config), chat history search and restore, the in-turn compactor and tool-output spill | `crates/hq-agent/src/fold/` (`mod.rs` view, cut, fold and distill, relations; `rules.rs` the summary rules), `crates/hq-db/src/thread_folds.rs` and `chat_search.rs` (`sql/115_thread_folds.sql`, `sql/116_chat_search.sql`), `thread_history` in `crates/hq-web/src/ws/mod.rs`, `fold_blocks` in `crates/hq-relay/src/native_run.rs`, `crates/hq-tools/src/session_search.rs` (`session_search`, `history_restore`), `crates/hq-tools/src/spill.rs` (`tool_output_read`), `crates/hq-agent/src/session/context.rs`, `crates/hq-agent/tests/fold_recall.rs` and `fold_graph.rs`, `docs/plans/context-fold.md` |
| Context engine | `crates/hq-agent/src/context/engine.rs` |
| Folder allow for a chat (per-chat, eight hours, from the approval card) and the nudge after three approvals | `crates/hq-agent/src/governance/folder_scope.rs` (what a grant may name and cover), `FolderCover` and `OwnerApproval::folder_cover` in `owner_approval.rs`, `owner_decision` in `governed_tool.rs`, `BashSettings::confined_to` in `crates/hq-agent/src/bash_sandbox.rs`, `crates/hq-db/src/owner_folder_grants.rs` (`sql/137_owner_folder_grants.sql`), `approve_folder` in `crates/hq-web/src/notifications_api.rs`, `chat_folders_api.rs`, `apps/hq-web/src/components/chat/ChatApprovals.tsx` and `SessionsPanel.tsx`, "Folder allow" in `docs/security/OWNER_APPROVAL.md` |
| Bash sandbox, prompt-injection policy | `crates/hq-agent/src/bash_sandbox.rs`, `crates/hq-agent/src/governance/` (`taint.rs`, `secrets.rs`, `egress.rs`), `docs/security/BASH_SANDBOX.md`, `docs/security/PROMPT_INJECTION.md` |
| Orchestrator role (`governance.orchestrator_role`) | `crates/hq-agent/src/builder.rs` (`SessionRole`, `ORCHESTRATOR_REMOVED_TOOLS`), `crates/hq-agent/src/session/role_denial.rs`, `crates/hq-tools/src/github.rs`, `docs/security/ORCHESTRATOR_ROLE.md` |
| Owner approval in chat sessions (`PermissionMode::Default`, `governance.ask_owner`): a call that changes things outside the vault and tasks waits for the owner to approve exactly it in their inbox | `crates/hq-agent/src/governance/owner_approval.rs` (`needs_owner`, `OwnerApproval`), `GovernedTool::owner_decision` in `governed_tool.rs`, wired in `crates/hq-web/src/ws/mod.rs` and `crates/hq-relay/src/native_run.rs`, `hq_db::value_items::approval_gate`, `hq_core::approval_key::args_digest`, `docs/security/OWNER_APPROVAL.md` |

## Rules

- **Execution**: sub-agents run through the unified `spawn_subagents` tool
  (`SpawnSubagentsTool`) over `AgentService` (single, parallel, race and graph
  modes, in process). It is the only sub-agent substrate.
  `agent_send_message` and `agent_read_inbox` are plain vault-mailbox tools for
  handing text to `relay` or another local mailbox id.
- **Tool hint**: `crates/hq-agent/src/tool_hint.rs` adds one "Tools that may fit this message (not
  loaded yet; call tool_search to load): ..." line to the per-turn context note when the message
  clearly needs a deferred tool. It ranks with `hq_tool_api::tool_rank`, offline, from the message
  and the not-yet-loaded catalogue only: never usage, history or adoption data, and never the system
  prompt or the tool list (the cached prefix). Keep it that way; a hint that depended on what the
  person had used would break `docs/security/ADOPTION_DATA.md`. Shown and followed hints are
  counted in `hq_db::tool_routing`.
