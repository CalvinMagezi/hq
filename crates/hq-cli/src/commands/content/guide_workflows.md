---
noteType: guide
fileName: workflows
version: 1
---
# Common Workflows

This guide covers the most useful HQ workflows for day-to-day use.

## Basic Chat

```bash
hq                    # Start interactive chat (default command)
hq chat -m anthropic/claude-sonnet-5  # Chat with a specific model
```

The agent loads your vault context (SOUL, MEMORY, PREFERENCES, pinned notes) and responds with full awareness of your knowledge base.

## Sub-Agent Execution

For complex tasks, HQ spawns sub-agents in-process with the `spawn_subagents` tool (single, parallel, race or graph runs); they report back to the session that spawned them. Long-lived coding agents (Claude Code, Codex and others) run as harness sessions over the host: `hq sessions`.

## Vault Operations

```bash
hq vault stats      # Note count, disk usage
hq vault list       # List recent notes
hq vault tree       # Directory tree view
hq vault read <path>  # Read a specific note
hq search "kubernetes deployment"  # Full-text search
```

## Memory Management

```bash
hq memory show      # Display SOUL + MEMORY + PREFERENCES
hq memory facts     # Just the facts from MEMORY.md
hq memory soul      # Just the SOUL.md content
hq memory context   # Full system context (what agents see at session start)
```

## Tasks

Multi-step work is tracked as native tasks (task_create, task_list, task_update) and shows up in the web app's Tasks view.

## Daemon

The background daemon handles scheduled work:

```bash
hq start daemon     # Start the background daemon
hq daemon status    # Check if it's running
hq daemon logs      # View daemon output
hq daemon stop      # Stop it
```

## Monitoring

```bash
hq status           # System overview
hq health           # Diagnostic check
hq ps               # Running processes
hq logs             # Recent logs (journald on a systemd host)
hq logs -n 100      # Last 100 log lines
hq logs -f          # Live log tail
hq logs --errors    # Error lines only
hq usage            # LLM cost and tokens by model
```

