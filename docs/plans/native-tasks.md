# Native task management

**Date**: 2026-09-21 (migration), updated 2026-09-22 (vault bridge)
**Topic**: The built-in task system, when to use it instead of a
vault note, and the note-to-task bridge.

This is the doc that `crates/hq-tools/src/tasks/`, `crates/hq-db/src/tasks.rs`
and `crates/hq-web/src/tasks_api.rs` point to as "the design".

## Shape

Spaces > Folders > Initiatives > Tasks > Comments, an `Initiative` is a project or work area, not just a bucket. Tags are the filter/routing dimension: tagging
a task with an agent id (`"hq"`) is how work gets assigned, since
agents don't have real accounts.

`hq-db::tasks` is the single write surface. Both the MCP tool layer
(`hq-tools::tasks`) and the web REST layer (`hq-web::tasks_api`) call its
functions directly and never call each other. Two default Spaces
(`personal`/`professional`, `id == slug` for these two only) are seeded by
`crates/hq-db/sql/044_tasks.sql` via `INSERT OR IGNORE`, so they exist in
every vault including a fresh one; every other Space gets a generated id
distinct from its slug.

Task creation is a direct tool in both directions — unlike Missions (chat-flow
only), agents and humans both call `task_create` directly, and a tagged agent
is notified via its mailbox (`hq_core::mailbox::notify_tagged_agents`) the
moment the task is created.

## Vault vs. Tasks

The vault and the native task system store two different things, and picking
the wrong one is the main confusion this doc exists to prevent:

- **Vault note** — knowledge, reference material, research findings, meeting
  notes, memory. Nothing here expects to be tracked to completion or assigned
  to anyone; it's for consulting later.
- **Task** — an actionable work item that belongs in the Tasks UI: it needs a
  Space > Folder > List, a status (`to_do`/`in_progress`/`blocked`/
  `ready_for_review`/`complete`), and, via tags, an owner.

Rule of thumb: capturing information → `vault_write_note`. Creating something
that should show up in the Tasks UI and get tracked to completion →
`task_create`. This same rule is stated once in `_system/SOUL.md` (every
harness reads it at session start) and cross-referenced from both tools'
descriptions (`task_create` and `vault_write_note`) so it surfaces wherever an
agent discovers either tool, not just at session start.

## Promoting a note into a task: `task_create_from_note`

The web vault UI's note view has a "Convert to Task" button that opens the
chat panel with a pre-filled (not auto-sent) prompt asking hq to promote the
open note. When sent, the agent calls the `task_create_from_note` MCP tool
(`crates/hq-tools/src/tasks/`), which:

1. Reads the note (`hq_vault::VaultClient::read_note`) for its title, tags,
   and content.
2. Unless the caller forces a destination (`space_id`/`folder`/`initiative`
   params), builds the list of existing Lists (Space > Folder > Initiative)
   and picks one:
   - **Heuristic first** (the incumbent, zero-config behavior): match the
     note's tags, containing vault folder name, and title words (5+ characters)
     against existing List/Folder names. Ambiguous (more than one match) or no
     match at all falls through. The title check matters more than it might
     look: a note's own heading is often the strongest signal even when its
     folder and tags don't match anything (e.g. a note titled "AcmeCorp
     Platform & Infrastructure Research Findings" matches a List named
     "AcmeCorp — Platform & Infra" purely on shared title words).
   - **Jev as an optional sharpening layer**: if the `task_placement` decision
     site (see `docs/architecture/decisions.md`) is enabled, it gets a shot at
     picking a better match or confirming there's no fit, but a disabled site,
     a timeout, or an abstain all fall back to the heuristic result — never a
     hard dependency, so the feature works with zero decisions config.
3. If nothing fits, creates a new List named after the note's containing
   folder. A brand-new Space is only created when no Space exists at all
   (vanishingly rare in practice, since `personal`/`professional` are always
   seeded) — filing a new List under an existing Space is far less disruptive
   than guessing wrong and inventing Spaces on every miss.
4. Creates the task with a title/excerpt drawn from the note and a trailing
   `Source: vault note <path>` line linking back to it.

See `docs/architecture/decisions.md`'s `task_placement` row and its
"Adding a consumer" section for how the Jev call itself is structured. To
actually turn Jev on for this site (it's off by default, same as every new
site), add to `~/.hq/config.yaml`:

```yaml
decisions:
  enabled: true
  routes:
    - endpoint: https://openrouter.ai/api/alpha/decisions
      model: typesafe/jev-1.13
      credential_env: OPENROUTER_API_KEY
  sites:
    task_placement: { mode: enforce }
```

...with `OPENROUTER_API_KEY` set in the environment `hq start` runs under, then
restart (`decisions:` is read once at daemon startup). Without this, the
button still works — every note gets placed by the heuristic above alone.

## Sub-tasks, dependencies and views (2026-09-24)

Migration `053_task_hierarchy_and_dependencies` is purely additive: two
nullable columns on `tasks` (`parent_task_id`, `start_date`) and a
`task_dependencies` table. Existing tasks keep their ids, tags and dates.

- **Sub-tasks** nest one level. A sub-task always lives in its parent's
  initiative and draws its display id from the same sequence. A task with
  sub-tasks cannot become a sub-task, and deleting it needs `cascade`.
- **Dependencies** are finish-to-start and soft. Cycles and self-links are
  rejected (recursive CTE in `add_dependency`). Nothing blocks a status change:
  `blocked_by` lists open blockers, `task_update`/`PATCH` return a `warnings`
  array when a blocked task is started or finished, and completing a task
  mails the tagged agents of every task it just unblocked
  (`hq_tools::tasks::notify_unblocked`). The manual `blocked` status is
  separate and means blocked on something outside the system.
- **Dates** are validated as `YYYY-MM-DD`, with `start_date <= due_date`.
- **Shared adapter helpers**: `hq_tools::tasks::{task_json,
  task_json_with_warnings, apply_dependency_changes, unblocked_by_transition,
  notify_unblocked}` are used by both the MCP tools and `hq-web`, so the JSON
  shape and notifications cannot drift.
- **Web** (`/tasks?view=list|board|timeline`): the board moves cards with the
  claim-safe `expected_status` and reverts on a 409. The timeline drags bars to
  move, drags edges to resize, and draws dependency arrows (rose when a task
  starts before its blocker ends). Unscheduled tasks sit in a tray and can be
  dragged onto the grid. Both views use `usePointerDrag` (pointer events, with
  long-press on touch) instead of HTML5 drag-and-drop.

## Lifecycle timestamps (FR-068)

Migration `062_task_lifecycle_events` adds an append-only `task_events` table
and two nullable summary columns on `tasks`. Every move into `in_progress` or
`ready_for_review` inserts one event (`entered_in_progress`,
`entered_ready_for_review`) in the same `BEGIN IMMEDIATE` transaction as the
status change in `hq_db::tasks::update_task`. `work_started_at` and
`first_ready_for_review_at` are set once (`COALESCE`), so a reopen or retry adds
events but never overwrites the first timing. `hq_db::tasks::list_task_events`
returns a task's events oldest first.

- **Format**: UTC `YYYY-MM-DD HH:MM:SS` from SQLite `datetime('now')`, the same
  as `created_at`. The web UI labels them UTC.
- **Unknown, not backfilled**: tasks that predate the migration have no events
  and NULL summaries. A status write that does not change the status (a
  repeated `in_progress`) records nothing, and a lost claim
  (`expected_status` mismatch) rolls back with no event.
- **Surface**: `Task` JSON (MCP `task_list`/`task_get`, REST list and PATCH)
  carries both summaries; `task_get` adds `lifecycle_events`; REST has
  `GET /api/tasks/{id}/events`. The detail drawer shows the two summaries.
- **Validate**: `cargo test -p hq-db tasks::` (repeated transitions, reopen,
  concurrent claims, legacy unknown, UTC format) and
  `migrations::tests::task_lifecycle_migration_keeps_existing_rows_unknown_and_rolls_back`.
- **Rollback** (SQLite 3.35+; the data is only timing, no other table reads it),
  then delete the `062_task_lifecycle_events` row from `schema_version` so the
  migration can be re-applied later:

```sql
DROP TABLE task_events;
ALTER TABLE tasks DROP COLUMN work_started_at;
ALTER TABLE tasks DROP COLUMN first_ready_for_review_at;
```

## Task relationship graph (FR-069)

`task_related` (crates/hq-tools/src/tasks/tools_graph.rs) answers "what else is connected to this task". The
logic lives in `crates/hq-db/src/task_graph.rs`; storage is two derived tables from migration 063
(`task_graph_nodes`, `task_graph_edges`). It never touches the vault or memory graph and does not
depend on hq-memory, so a failure in either cannot affect the other.

**Two kinds of link, always returned in separate lists.**

- `explicit`: parent, subtask, depends_on, dependent. Read live from `tasks.parent_task_id` and
  `task_dependencies`, so they are never stale. These are the only links that mean "declared".
- `inferred`: deterministic similarity, with a score and evidence. Each result says why: `shared_terms`,
  `shared_tags`, `same_initiative`, `text_similarity`, `tag_overlap`. Inferred links are hints and are
  labelled as such in every result.

**Scoring.** `0.6 * tfidf_cosine(title x3 + description) + 0.3 * rarity-weighted tag overlap + 0.1 * same
initiative`, kept when at least one term or tag is shared and the score is 0.2 or more, top 10 per task.
A tag on every task has weight zero, so routing tags like `hq` never link anything. Pure numbers are not
terms. No LLM calls, no new dependency.

**Freshness.** Each indexed task stores a fingerprint of initiative, title, description and tags. Every
`task_related` call runs `sync`: deleted tasks are pruned with their edges, tasks whose fingerprint
changed are re-derived (at most 200 per call, the rest reported as `stale_remaining`). Editing a task is
the correction path: a wrong link disappears when the text that caused it changes. Tasks have no merge
operation; a merged-away task is a deleted one. Untouched pairs keep the corpus statistics they were
scored with, so scores drift slowly as the corpus grows; `rebuild=true` drops both tables and re-derives
everything. Comments are not indexed.

**Bounds and fallback.** One hop only, at most 25 inferred links, at most 50 explicit links. When the
index has no inferred neighbour, or `sync` fails (a `warning` is returned), the result carries
`fallback`: recent tasks from the same initiative.

**Lifecycle timestamps (FR-068).** Evaluated and not used as a score feature. Only tasks moved after
migration 062 have `work_started_at`, and in a single-operator system close start times mostly reflect
when an agent was scheduled, not topical relatedness. The graph is complete without them.

**Access.** Read-only tool in the `tasks` category, same governance as `task_get`. Results return ids,
display ids, titles, statuses and the shared terms and tags only, never descriptions or comments. The only
write is to the derived index tables.

**Quality check.** `retrieval_quality_on_labeled_fixture` in `task_graph/tests.rs` holds 12 tasks in 3
labeled clusters plus 3 distractors and asserts precision@2 >= 0.8 and recall@2 >= 0.8 (currently 1.000
and 0.889). The fixture is small and hand-built, so it guards regressions; it is not a claim about
accuracy on the real backlog.

Rollback: `DROP TABLE task_graph_edges; DROP TABLE task_graph_nodes;`
