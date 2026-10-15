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

## Packets built from a task

Give `context_packet` (or a sub-agent's `context_need`) a `task_id` and the need is built from the
task, in priority order:

1. the task's own record, rendered in a `<task_record>` block: description, acceptance criteria
   (the lines after an `Acceptance` label or heading), status, owner, the unfinished tasks it waits
   on, a recorded blocker, what the last session left open, and its pull request, commit, URL,
   session and task links (listed, not fetched)
2. the vault notes it links (`via="task_link"`)
3. notes recorded as written under the task or by the sessions it is linked to
   (`via="task_produced"`, from the provenance store)
4. explicit open questions, and the conflicts the packet already flags

Explicit `refs` and `queries` come first and are never displaced. With no queries of its own, the
task's title is searched as plain keywords. The task record is charged to the packet budget (at most
half of it; the least important parts are trimmed first), so a packet never exceeds `budget_chars`.

If the task cannot be read, or search or the graph index fails, the packet falls back to the explicit
references and the task's direct links and says so as a gap; a packet with any gap is labelled
`Completeness: partial`. Task text is written by people and agents, so it is fenced and escaped like
note text and the reader is told it is data, not an instruction.

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

## Who wrote it, and whether anyone reviewed it

Each entry also carries what the provenance store knows about the exact version it quotes,
shown as attributes on the `<vault_note>` block and as a `Provenance:` line inside it:

```
<vault_note id="S1" path="..." ... actor="relay" source_kind="agent" task="tk-1" session="s-1"
            provenance="recorded" review="unreviewed" reviewed_by="unknown">
```

| `provenance` | Meaning |
|---|---|
| `recorded` | a write of this exact version was recorded |
| `unattributed` | it was recorded but the writer could not say who it was |
| `changed_outside_tools` | the note has recorded writes, but this text is not one of them (edited in an editor or by a shell); its author is not known |
| `no_record` | nothing was ever recorded for the note |
| `unavailable` | no provenance store to ask (no database, or one from before the store existed) |
| `outdated` | the note changed after the packet was built, and a refresh stopped crediting the earlier author |

What is not known renders as `unknown` (and `review="none"` when there is no record to review).
It is never filled in from an edit time or from which agent was active. `review` is one of
`unreviewed`, `reviewed`, `accepted`, `disputed`, `superseded`, with `reviewed_by` beside it.

Reading rules, repeated in the packet itself: `unknown` is not a sign of trust; `unreviewed`,
`disputed` and `superseded` sources are claims to verify and never settled fact; even `accepted`
is one person's judgement at one time and not proof the text is current. An edit date is still
not proof of validity, and provenance does not change a source's freshness verdict. Packets saved
before this existed read back as `unavailable`. The fencing and escaping of note text are
unchanged, and provenance values are stripped of quotes and angle brackets before they go into the
block header.

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
