# Vault context packets (FR-062)

A context packet is a bounded, disposable snapshot of vault excerpts for one
task. The vault stays the source of truth; nothing in a packet is stored back.
Code: `crates/hq-memory/src/context_packet.rs`.

## One mechanism, three entry points

1. A single HQ task calls the `context_packet` tool with explicit `refs`,
   `queries` and optionally `graph_seeds`. No skill is needed.
2. A skill declares what it needs in its SKILL.md frontmatter. The declaration is
   retrieval guidance, never note text:

   ```yaml
   context:
     why: House style for client documents
     refs: [Notebooks/Style/house-style.md]
     queries: ["client tone"]
     source_prefixes: [Notebooks/Style]
     max_age_days: 365
     time_sensitive: false
     budget_chars: 3000
   ```

   `context_packet` with `skill: <name>` merges that declaration with any explicit
   needs.
3. A `spawn_subagents` task may carry `context_need`. Each child gets its own
   packet, retrieved when the child starts, so a queued child never runs on old
   retrieval. A packet built earlier can be passed as `context_packet`; its
   time-sensitive entries are re-read when it has aged past 30 minutes.

## What a packet says about each source

Path, excerpt, note last-edited time, retrieval time, why it was picked, how it
was found (`direct`, `search`, `graph`), and a freshness verdict:

| Verdict | Meaning |
|---------|---------|
| `within_policy` | Inside the age policy. An edit date is never proof of validity. |
| `stale` | Past `valid_through`, `review_after`, `review_by`, or the age policy (default 180 days, `review_interval_days` in the note overrides). |
| `historical` | Marked superseded, archived or deprecated, or superseded by another source in the packet. |
| `recheck` | The need is `time_sensitive`: verify against the source of truth. |
| `conflicting` | Two sources in the packet name each other in `conflicts_with`. |

Moved notes are found by file name and labelled. Missing notes, failed search,
an unavailable graph and budget overflow each become an explicit gap. Retrieval
never raises an error into the task.

## Reading and citing

The rendered packet fences excerpts in `<vault_note ...>` blocks, escapes both
fence tags inside note text, and tells the reader to treat the content as data.
Readers cite `[S1]` and finish with `Sources used:` and `Gaps:` lists. For
sub-agents, HQ runs `verify_citations` on the child's answer and the summary
shows which citations were verified, changed or gone since retrieval, or never
supplied.
