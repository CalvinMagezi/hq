---
noteType: guide
fileName: vault-structure
version: 1
---
# Vault Structure Guide

The vault is a directory of markdown files with YAML frontmatter. Every file is human-readable and editable. This guide explains what goes where.

## Directory Layout

```
.vault/
├── _system/                  # Agent identity and configuration
│   ├── SOUL.md               # Agent operating principles (read every session)
│   ├── MEMORY.md             # Persistent facts and goals
│   ├── PREFERENCES.md        # User preferences and custom instructions
│   ├── HEARTBEAT.md          # Daemon action queue (processed every 2 min)
│   ├── CONFIG.md             # Runtime configuration table
│   ├── CAPABILITIES.md       # Detected tools and integrations
│   ├── ONBOARD.md            # Onboarding progress tracker
│   └── guides/               # Reference documentation (this file is here)
│
│
├── _threads/                 # Conversation continuity
│   ├── active/               # Ongoing conversation threads
│   └── archived/             # Past conversations (searchable)
│
├── _plans/                   # Multi-step execution plans
│   ├── active/               # In-progress plans
│   └── archive/              # Completed/abandoned plans
│
├── _agents/                  # Agent role definitions
├── _mailboxes/               # Messages between agents and the relay
│
├── _approvals/               # Human-in-the-loop gates
│   ├── pending/              # Awaiting user decision
│   └── resolved/             # Decided (approved/rejected)
│
├── _logs/                    # Agent session logs
├── _usage/daily/             # Token usage tracking
├── _embeddings/              # Vector embedding cache
├── _agent-sessions/          # Session state files
├── _moc/                     # Maps of content (auto-generated indexes)
├── _templates/               # Note templates
├── _data/                    # SQLite database (vault.db)
├── _orchestration/           # Orchestration traces
│
└── Notebooks/                # User-facing knowledge
    ├── Memories/             # Personal/agent memories
    ├── Projects/             # Project-specific notes
    ├── People/               # One note per person — see People Notes below
    ├── AI Intelligence/      # AI research and findings
    ├── Insights/             # Analysis and insights
    └── Diagrams/             # Generated diagrams
```

## Frontmatter Convention

Every vault note should have YAML frontmatter:

```yaml
---
noteType: notebook          # system-file, guide, job, task, plan, notebook, etc.
fileName: my-project-notes  # Kebab-case identifier
version: 1                  # Increment on major rewrites
tags: [project, active]     # Searchable tags
created: 2026-03-22         # ISO date
---
```

## People Notes

`Notebooks/People/` holds one note per person — the vault's answer to "who is
this and how are they connected," so that doesn't have to be reconstructed
from mailbox history each time. Full convention lives in
`Notebooks/People/README.md`; the essentials:

- One note per person, filename `firstname-lastname.md`. Organisations go in
  `People/Organizations/`, not mixed in with people.
- Every claim carries a provenance tag: `[C]` stated by the operator and
  confirmed, `[P]` public record with a source, `[I]` inferred by HQ and
  **not** confirmed — never silently upgrade an `[I]` to a `[C]`.
- Sources section at the bottom of every note. No claim without a trail.
- Corrections are recorded, not overwritten — a wrong version stays visible
  with a dated correction rather than disappearing.
- A note holding only a name is labelled a stub, not dressed up as a profile.

A human profile does not belong under `Agent-Wiki/entities/`, which is scoped
to organisations and agents — link to and from those notes instead of
duplicating what they already record.

## Key Rules

1. **System files are sacred.** Agents read `_system/` at session start. Don't delete them.
2. **Plans are shared state.** Active work lives in `_plans/active/`; agents hand off text through `_mailboxes/`.
3. **Notebooks are for users.** Agents write here, but the content should be user-facing.
4. **Underscore dirs are infrastructure.** Everything in `_*` is managed by HQ. `Notebooks/` is the user's space.
