---
noteType: guide
fileName: memory-system
version: 1
---
# Memory System Guide

HQ's memory system gives agents persistent, structured recall across sessions. Memory is stored as plain markdown in the vault, making it transparent, searchable, and editable.

## How Memory Works

### System Files (Core Memory)

These files in `_system/` are loaded at every session start:

| File | Purpose | Agent Behavior |
|------|---------|---------------|
| `SOUL.md` | Operating identity and principles | Read-only reference, defines how the agent behaves |
| `MEMORY.md` | Key facts, active goals, session history | Read/write, agents add facts as they learn them |
| `PREFERENCES.md` | User preferences and custom instructions | Read-only during sessions, edited by user or onboard |
| `CONFIG.md` | Runtime configuration values | Read-only, agents respect these settings |
| `CAPABILITIES.md` | Available tools and integrations | Read-only, updated by `hq install` |

### Notebook Memory (Extended Memory)

Notes in `Notebooks/Memories/` store longer-form memories that don't fit in MEMORY.md:
- Project-specific context
- Detailed session summaries
- Research findings
- Decision records

### Embeddings (Semantic Memory)

The daemon automatically embeds vault notes for semantic search:
- New notes are queued for embedding on creation
- Batch processing: 10 notes per daemon cycle (every 10 minutes)
- Enables `hq search` with semantic similarity matching
- Embedding vectors stored in `_data/vault.db`

## Memory Lifecycle

### Ingestion
When an agent learns something new:
1. **Fact**: Short, key insight → add to `MEMORY.md` under "Key Facts"
2. **Goal**: Active objective → add to `MEMORY.md` under "Active Goals"
3. **Detail**: Longer context → create a note in `Notebooks/Memories/`
4. **Project-specific**: Context for a project → add to `Notebooks/Projects/<project>/`

### Consolidation
Over time, memory is consolidated:
- The memory consolidator merges redundant facts
- Old session summaries are compressed
- Completed goals are archived
- The daemon runs it on a schedule. `hq memory stats` shows when it last ran

### Forgetting
Not everything should be remembered forever:
- Stale facts (no longer true) should be removed from MEMORY.md
- Completed project notes can be archived
- The forgetter can prune low-value memories based on age and relevance

## Best Practices

1. **Keep MEMORY.md focused.** It's loaded every session, so keep it to high-value facts. Move details to Notebooks.
2. **Tag memories.** Use frontmatter tags so they're discoverable via search.
3. **Date your memories.** Include creation dates so staleness is visible.
4. **Let agents write.** Don't manually manage every memory. Agents are trained to maintain MEMORY.md.
5. **Review periodically.** Run `hq memory show` occasionally to prune outdated facts.
