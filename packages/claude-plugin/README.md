# Agent HQ plugin for Claude Code

Connects a Claude Code session to your [Agent HQ](https://github.com/CalvinMagezi/hq): the HQ MCP server, a health check when a session starts, and skills for tasks, other agents and the vault.

## Install

```
/plugin marketplace add CalvinMagezi/hq
/plugin install agent-hq@agent-hq
```

Claude Code asks for two values:

- **HQ MCP URL**: your HQ's MCP endpoint, for example `https://hq.example.com/mcp`.
- **HQ API key**: the MCP API key set on your HQ (`AGENTHQ_API_KEY`, see `docs/VPS_AGENT_CONNECT.md`). It is stored as a sensitive value.

## What you get

| Piece | What it does |
|-------|--------------|
| MCP server | Adds HQ's `hq_discover` and `hq_call` tools to the session. |
| Session-start hook | Checks `<url>/health`. If HQ answers, it reminds the agent to use tasks, the fleet and the vault. If not, it warns the agent to tell you instead of working around it. After a compaction it reminds the agent to send a task heartbeat. |
| `hq-tasks` | Claim a task, keep a lease, checkpoint, release with an honest status. |
| `hq-fleet` | Hand bounded work to other coding agents with `harness_session_spawn`. |
| `hq-vault` | Search before writing, update existing notes, keep knowledge in the vault. |
| `hq-status` | Run `/agent-hq:hq-status` for a summary of health, hosts, agents and tasks. |

`skills/hq-tasks/SKILL.md` is a copy of `crates/hq-cli/assets/skills/hq-tasks/SKILL.md`. A Rust test fails if they differ.

## Validate

```
claude plugin validate ./packages/claude-plugin
claude plugin validate .
```
