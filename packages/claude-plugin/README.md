# Agent HQ plugin for Claude Code

Keeps a Claude Code session working through your [Agent HQ](https://github.com/CalvinMagezi/hq): a health check and task count when a session starts, a fleet-first nudge, a lease reminder when a turn ends, and skills for tasks, other agents and the vault.

The HQ tools themselves (`hq_discover`, `hq_call`) come from one of two places:

- a claude.ai connector for your HQ (`docs/CLAUDE_CONNECTOR.md`), which Claude Code loads by itself, or
- the [`agent-hq-mcp`](../claude-plugin-mcp/README.md) plugin from the same marketplace.

Use one, not both, so a session does not load the same tools twice.

## Install

```
/plugin marketplace add CalvinMagezi/hq
/plugin install agent-hq@agent-hq
/plugin install agent-hq-mcp@agent-hq    # only without a claude.ai connector
```

Claude Code asks for two values:

- **HQ MCP URL**: your HQ's MCP endpoint, for example `https://hq.example.com/mcp`.
- **HQ API key**: the MCP API key set on your HQ (`AGENTHQ_API_KEY`, see `docs/VPS_AGENT_CONNECT.md`). It is stored as a sensitive value. The hooks use it to count tasks; without it they still run the health check.

## What you get

| Piece | What it does |
|-------|--------------|
| Session-start hook | Checks `<url>/health`. If HQ answers, it reminds the agent to use tasks, the fleet and the vault, and prints how many tasks are in progress, blocked and to do. If not, it warns the agent to tell you instead of working around it. After a compaction it reminds the agent to send a task heartbeat. |
| Fleet nudge | The first `scout` or `Explore` subagent launch in a session is sent back once with a pointer to HQ harness sessions. Launching it again goes through. |
| Lease reminder | When a turn ends while the session holds a task it claimed and has not released, the agent is asked once to release it or send a checkpoint. It asks again only after another claim or release. |
| `hq-tasks` | Claim a task, keep a lease, checkpoint, release with an honest status. |
| `hq-fleet` | Hand bounded work to other coding agents with `harness_session_spawn`. |
| `hq-vault` | Search before writing, update existing notes, keep knowledge in the vault. |
| `hq-status` | Run `/agent-hq:hq-status` for a summary of health, hosts, agents and tasks. |

The nudge and the reminder need `python3`; without it they do nothing. They keep small marker files in the system temp directory under `agent-hq-plugin/`.

`skills/hq-tasks/SKILL.md` is a copy of `crates/hq-cli/assets/skills/hq-tasks/SKILL.md`. A Rust test fails if they differ.

## Validate

```
claude plugin validate ./packages/claude-plugin
claude plugin validate ./packages/claude-plugin-mcp
claude plugin validate .
```
