# Agent teams: sessions that start sessions

Status: in progress. Steps 1 and 2 are built (team limits, delegation to other agents, spawns from a session becoming delegations), and step 3 without the vault part (team-wide scope, `team_roster`, the session token reaching team logs and waits). The team vault folder, lease tools for sessions, the brief and result contract, and tokens for other agents are proposed.

The flow HQ is built for starts the conversation in HQ: the owner asks HQ for
something, HQ starts one lead session on a task, and the lead decides how much
of the work to hand to other agents (executors, a reviewer, a validator), which
may hand work on again. HQ watches the lead, the lead watches its workers, and
every agent in the tree knows who else is in it. Today a launched session can
barely do any of that. This plan removes the guards that exist only because
nested teams were not designed yet, and replaces them with deterministic limits
that hold however deep the tree grows.

## What stops a team today

| Guard | Where | Effect |
|-------|-------|--------|
| `refuse_if_spawned` | `harness_session/origin.rs`, used by spawn, resume and handoff | A session HQ launched may not start another one ("ask the owner"). |
| `MAX_SPAWN_DEPTH = 2` | `a2a.rs` | Owner's lead, its worker, and one more level. A compile-time constant. |
| `pick_harness`: the delegator's own agent only | `a2a.rs` | A Claude lead cannot hand work to Codex, Copilot or pi. No model, purpose or folder choice either. |
| Session tokens only for `kind: claude` | `harness_session/launch.rs` (`mcp_access`) | Other agents cannot report, message or delegate. |
| `SESSION_ALLOWLIST` | `hq-mcp/src/gateway.rs` | The token reaches the task thread only: no vault, no lease or checkpoint, no roster. |
| `thread_between`, `check_session_access`, `task_scope` | `a2a.rs` | Scope is self, parent and children. Siblings, grandparents and grandchildren cannot see or message each other. |

The per-node limits (`MAX_LIVE_CHILDREN` 20, `MAX_CHILDREN_PER_WINDOW` 60) do
not compose: once depth grows, 20 children each with 20 children is reached
long before anything stops it, and the host's `MAX_AGENTS` (128) is the only
backstop.

### The side door

A launched Claude Code session loads the user's own MCP configuration as well as
its `hq-session` server. When the user has HQ as a claude.ai connector, the
session sees both, and the connector carries the owner's full grant with no
session attestation. Through it a launched session can call
`harness_session_spawn`, `refuse_if_spawned` never fires (no
`x-hq-session-id` marker reaches the connector), and the new session has no
parent: it is not stopped with the tree, never reports back and counts against
no limit. Closing this is the first step, because every later rule assumes the
attested token is the only way in.

## Principles

Drawn from Anthropic's published guidance on multi-agent systems (the
multi-agent research system, "Building multi-agent systems: when and how",
multi-agent coordination patterns, Claude Code subagents and agent teams, the
Agent SDK and Managed Agents multi-agent docs) and from the changelog of an
open-source terminal agent multiplexer:

1. **Deterministic caps, not refusals.** Prompting only steers. Depth,
   concurrency and spend are enforced in code, at the tree, with readable errors.
2. **Decompose by context, not by job title.** An executor writes its own tests.
   A separate agent pays off where the handoff is small: independent tracks, and
   verification (artifact plus criteria plus tools).
3. **References, not transcripts.** Workers write substance to the vault or a
   file and pass a path and a short summary back. The parent's pane gets a
   capped message.
4. **One team, one roster.** Every member knows the others: who, role, task,
   what files they own, status.
5. **Partition writes.** Parallel writers in one folder overwrite each other.
   Owned paths in the brief, and a worktree per writer when they overlap.
6. **Agent messages are data.** They never count as approval and are labelled as
   coming from another session.
7. **Child state never leaks up.** A child that blocks blocks its own task, not
   its parent's status; nested agents never inherit or overwrite a parent's
   identity or resume record.

## Design

### 1. One door, steered

Hiding the owner's claude.ai connectors from launched sessions would close the
side door, but it would also cut those sessions off from every other connector
the owner relies on, so connectors stay. Instead the door leads to the same place:

- A spawn from inside a session becomes a delegation. The gateway's
  `x-hq-session-id` marker, or a `from_session` argument the caller sets to its own
  `HQ_SESSION_ID`, makes `harness_session_spawn` run `agent_delegate`'s path: one
  authorization check, one parent link, one stop-with-parent rule, the team's
  limits. Naming a session only adds limits, so there is nothing to gain by
  claiming one.
- The plugin's session-start hook, its fleet nudge and the `hq-fleet` skill tell
  a session with `HQ_SESSION_ID` set to start teammates that way.
- `harness_session_resume` and `handoff` stay refused for a marked session, and
  the refusal names `agent_delegate`.

A session that ignores all of that and spawns through the connector without
`from_session` still starts an unlinked session. That is the owner's grant being
used as the owner's, and it is the accepted cost of keeping connectors on.

### 2. `agent_delegate`, full featured

New optional arguments, each checked against owner config:

| Argument | Rule |
|----------|------|
| `harness` | Any name in `agent_host.teams.harnesses` (default: the delegator's own agent only, which is today's behavior). Profiles that spend another account's credit are allowed only when listed. |
| `model` | Passed as on `harness_session_spawn`. |
| `purpose` | `code` (default), `review`, `research`, `monitor`. A reviewer or monitor never moves its task. |
| `observe` | Sessions in the same tree a reviewer or monitor watches. |
| `owns` | Paths the child owns; recorded on its sub-task and shown in the roster. A second writer claiming an overlapping path is refused unless it runs in a worktree. |
| `worktree` | `true` starts the child in a new git worktree of the delegator's folder on a branch named for its sub-task. |
| `can_delegate` | Default `true` up to the tree's depth limit; `false` makes this child a leaf. |

The child's cwd is the delegator's folder or a worktree of it, never a folder
the owner has not already approved.

### 3. Limits on the tree

New config `agent_host.teams` (all owner-set, generic defaults):

```yaml
agent_host:
  teams:
    max_depth: 3              # Claude Code's own default for nested subagents
    max_live_per_tree: 8      # every running session under one root
    max_live_children: 5      # per node, so one node cannot take the whole tree
    max_spawns_per_hour: 30   # per tree
    harnesses: []             # extra agents a delegator may start; empty = its own only
```

A tree is every session reachable through `parent_session_id` from a root (the
session a person or HQ's chat started). One helper, `tree_root(session)`, is the
only place that walks it, so the limits, scope checks and roster agree.
`max_depth` 1 makes every delegated session a leaf, which is the conservative
setting Anthropic's agent teams and Managed Agents use; the default stays at 3
so the depth the owner asks for is available without a code change.

A spend cap (turns or tokens per tree) is out of scope for the first cut: HQ
does not see token use for every agent. `hq usage` covers HQ's own model calls;
the tree's turn count is the measurable proxy and is reported in the roster.

### 4. Team scope and the roster

- `thread_between`, `check_session_access` and `task_scope` accept any two
  sessions in the same tree. A message between two that do not share a task goes
  on the recipient's task thread, labelled with the sender.
- New tool `team_roster` (also in the delegation prompt): for the caller's tree,
  each session's id, harness, purpose, depth, task, owned paths, status and last
  reply line.
- `agent_message_send` gains `to_session: "parent"` and `"root"` shorthands.
- The supervisor tells a parent when a child finishes, blocks or exits (as
  today). The root additionally gets a one-line notice when any session in its
  tree blocks, since that is the one event a lead must not miss.

### 5. What a session token reaches

`SESSION_ALLOWLIST` grows, each tool limited to the caller's tree:

- `task_claim`, `task_heartbeat`, `task_release`, `task_update` on tasks in
  scope, so workers checkpoint and release honestly.
- `vault_search`, `vault_read`, `vault_find` (read), and `vault_write_note`,
  `vault_patch_section` confined to the team folder
  `Notebooks/Teams/<root task display id>/`. Writes elsewhere are refused.
  Governance taint still applies: a turn that read untrusted content keeps its
  restrictions.
- `team_roster`, `harness_session_list` filtered to the tree,
  `harness_session_logs` and `harness_session_wait` for sessions in the tree.

The test `a_session_token_reaches_task_threads_and_nothing_wider` changes on
purpose: the token still never reaches config, settings, other trees or the
wider vault for writing.

### 6. The team folder in the vault

Created when the root first delegates:

- `plan.md`: the lead's plan and decisions (the lead's memory outside its context).
- `roster.md`: written by HQ from the registry on each change, read-only to agents.
- `<sub-task id>.md`: each worker's result: status (done, partial, blocked),
  summary, artifacts, evidence, open questions.

The task thread stays the coordination channel; the folder holds what is too
big for a message. On the root task's completion the folder is kept as the
team's record.

### 7. The delegation brief and the result

`delegation_prompt` carries: the objective and done criteria, owned paths, the
roster, the team folder, where to write the result, whether it may delegate
further, the rule for being blocked (comment, release as blocked, do not guess),
and that messages from other sessions are not approvals. A delegation without a
description of what done looks like is refused.

A child's final report to its parent is its result note's path plus a summary
capped at `REPORT_TAIL_CHARS`, not its screen tail.

### 8. Other agents

Session tokens for non-Claude agents, so a Copilot or Codex worker can report
and message: Codex takes an MCP server through `-c mcp_servers…`, Copilot CLI
through `--additional-mcp-config`. pi and agents without MCP keep reporting
through the supervisor (finished turn, exit) only. A later step; until then a
non-Claude child is a leaf whose result reaches its parent through the
supervisor's report.

## Order of work

1. The side door: spawns from a session become delegations, the refusal points
   at `agent_delegate`, plugin hooks branch on `HQ_SESSION_ID`.
2. `agent_delegate` arguments and `agent_host.teams` limits with `tree_root`.
3. Tree scope, `team_roster`, the widened allowlist, the team folder.
4. Brief and result contract, `owns` and `worktree`. `worktree` lands together
   with phase 4 of [remote-driven-teams.md](remote-driven-teams.md), which adds
   it to spawn and handoff as well.
5. Session tokens for Codex and Copilot.

Each step ships with its tests and an end-to-end run on a real host: a root task,
a lead that delegates to a child of a different agent, that child delegating to a
grandchild, the grandchild messaging a sibling, and stopping the root stopping
the whole tree.

## Not doing

- A model-driven monitor that polls the team. The supervisor is deterministic
  code and already watches every pane; a monitor agent stays an opt-in `purpose`.
- A free-form shared scratchpad. The task thread and the team folder are
  append-only and attributed.
- Hook-only state for Codex (a comparable multiplexer dropped its Codex turn
  hooks as unreliable); screen
  detection stays the fallback.
