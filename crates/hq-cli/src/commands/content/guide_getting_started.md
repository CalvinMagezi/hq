---
noteType: guide
fileName: getting-started
version: 1
---
# Getting Started with Agent-HQ

Welcome to Agent-HQ. This guide covers the essentials for your first session.

## What Just Happened

Running `hq install` created your vault, a structured directory of markdown files that serves as your agent's knowledge base, task queue, and memory store. Everything is local, plain-text, and version-controllable.

## Your First Commands

```bash
hq                  # Start chatting (default command)
hq health           # Check what's configured and what needs attention
hq onboard          # Interactive walkthrough to set up integrations
hq status           # See vault stats and running services
```

## Core Concepts

### The Vault
Your vault is a directory (default: `~/.vault`) containing:
- **System files** (`_system/`) that configure agent behavior
- **Notebooks** (`Notebooks/`) for your knowledge and projects
- **Agent infrastructure** (`_plans/`, `_mailboxes/`) for agent coordination

### LLM Router
HQ has a built-in LLM router that handles all agent sessions. It supports multiple providers:
- **OpenRouter**: Routes to any model (Claude, GPT, Gemini, open-source)
- **Anthropic**: Direct API access to Claude models
- **Google AI**: Direct API access to Gemini models
- **Ollama**: Local model inference

Configure your API keys via `hq env` or `hq onboard`.

### The Daemon
A background process that:
- Processes heartbeat actions every 2 minutes
- Embeds new notes for semantic search
- Monitors system health
- Runs scheduled tasks (memory consolidation, vault maintenance). Recurring prompts use `/watch` in chat or the `watch_create` tool

Start it with: `hq start daemon`

## Next Steps

1. Run `hq onboard` to configure API keys and integrations
2. Run `hq` to start your first conversation
3. Check `_system/guides/` for detailed documentation on specific features
