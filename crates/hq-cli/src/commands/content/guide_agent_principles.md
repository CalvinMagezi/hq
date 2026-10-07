---
noteType: guide
fileName: agent-principles
version: 1
---
# Agent Principles Guide

How agents should behave when operating within an HQ vault.

## Identity

Each agent session has access to the vault's system files. The SOUL.md defines the agent's operating identity. Agents should read it at session start and follow its principles. Different harnesses (Claude, Gemini, OpenCode) share the same vault and the same identity.

## Memory Discipline

- **Read before write.** Always search the vault before creating new content. Duplicate notes are vault debt.
- **Update over create.** If a note on the topic exists, update it. Only create new notes for genuinely new topics.
- **Frontmatter always.** Every note gets YAML frontmatter. No exceptions.
- **Facts go to MEMORY.md.** When you learn something about the user or their projects, add it to `_system/MEMORY.md`.

## Task Management

- **Check context.** At session start, look at `_plans/active/` and `_system/HEARTBEAT.md`.
- **Use sub-agents for complex work.** Spawn sub-agents via the session API for tasks that need focused execution.
- **Log sessions.** Write a brief session log to `_logs/` when doing significant work.

## Delegation

When a task is better suited to a coding agent (Claude Code, Codex, Cursor, Pi, OpenCode):
1. Start it with `harness_session_spawn`, giving clear instructions, context, and the expected output.
2. Steer it with `harness_session_send` and check on it with `harness_session_status`.
3. The daemon's session supervisor reports when it finishes or blocks.
4. For an agent a person started by hand, find it with `host_agents` and prompt it with `host_send`.

## Safety

- **Never modify repository code** from within the vault. Output goes to the vault, not the repo.
- **Never expose secrets.** If you encounter API keys, tokens, or credentials, don't write them to notes.
- **Ask before external calls.** If a task requires sending data to an external service, confirm with the user first.
- **Respect approvals.** If a task is in `_approvals/pending/`, wait for user resolution before proceeding.

## Collaboration

When multiple agents are active:
- Share plans through `_plans/active/` and hand off text with `agent_send_message`.
- Don't overwrite another agent's in-progress work.
