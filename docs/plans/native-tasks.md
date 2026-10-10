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
and two nullable summary columns on `tasks`. (Migration 080 later extended the log to every
status, see "Event log, validation and paging".) Every move into `in_progress` or
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

## Event log, validation and paging (TSV2 WS1)

Migration `080_task_event_log` widens `task_events` (rebuilt, because SQLite cannot widen a
CHECK) and adds `tasks.completed_at`. Old events keep their ids and times; their new
`from_status` and `to_status` stay NULL (unknown, not guessed).

- **Every transition is logged.** A move into any of the five statuses appends one event
  (`entered_to_do`, `entered_in_progress`, `entered_blocked`, `entered_ready_for_review`,
  `entered_complete`) with the status it came from and went to. A write that does not change the
  status records nothing. `work_started_at` and `first_ready_for_review_at` are still stamped
  once. `completed_at` is the latest completion and is cleared when the task reopens.
- **Status and priority are validated** in `hq-db`, so MCP, REST and the harness supervisor share
  one rule. An unknown value fails with the allowed values in the message (REST answers 400).
  Only a status or priority a write actually sets is checked, so a legacy row can still be edited.
  There is no transition table on purpose: the board moves cards freely and any status is
  reachable from any other. Gating belongs to the work lease (WS2), not to a table.
- **Lists are paged.** `list_tasks` takes `limit` (at most `MAX_LIST_LIMIT`, 500) and `offset`,
  and `count_tasks` returns the total for the same filter. MCP `task_list` and `GET /api/tasks`
  reply with `total`, `offset` and `has_more`; MCP defaults to 100 rows per page. The web client
  follows `has_more`, so counts and the active total no longer stop at 500. Row errors while
  reading a page now fail the call instead of silently dropping rows.
- **Update and delete are one transaction.** `hq_db::tasks::in_write_tx` takes the write lock,
  runs the closure and commits, or joins a transaction the caller already holds. The MCP and REST
  update paths read the previous task, apply the patch, apply dependency changes and compute
  unblocked tasks inside one, so a failed dependency change undoes the status change.
- **Notifications follow tags that were added.** An update mails only the routing tags the write
  introduced (`hq_tools::tasks::added_tags`). Creating a task still mails all of its tags.

Rollback of 080 (SQLite 3.35+). The events table keeps the wider CHECK and the two extra columns,
which older code ignores, so only the new column needs to go, then delete the
`080_task_event_log` row from `schema_version`:

```sql
ALTER TABLE tasks DROP COLUMN completed_at;
```

## Work leases (TSV2 WS2)

A work lease is one agent session holding a task for a stretch of time. It is the session lock,
the source of time-on-task and the record of which session did the work, for any MCP client and
for sessions HQ spawns. Migration `081_task_work_sessions` adds the table and `actor` and
`work_session_id` columns on `task_events`; old events keep NULLs.

- **Tools** (full-key and tasks-scoped callers; restricted audiences and launched sessions are refused;
  on the tasks scope a lease is named `mcp:tasks/<name>`, see `docs/security/WEB_AUTH.md`):
  `task_claim` (task, actor, optional harness, session_ref, host, cwd, branch, takeover) returns a
  lease token once, moves a waiting task to in_progress and leaves a comment. `task_heartbeat`
  keeps it alive. `task_release` ends it, with an optional status and summary.
- **Attribution.** Pass the token as `lease` on `task_update`, `task_comment_add` and
  `task_create`. The lease's actor then wins over any `author`, `created_by` or `actor` text, and
  events carry the actor and lease id. A wrong or ended token is an error, never an anonymous
  write. Launched sessions proven by their own token keep their existing identity.
- **One holder.** A task another session holds is refused with who holds it and since when.
  `takeover` ends that lease as `superseded`. Claiming again with the same actor and `session_ref`
  replaces your own lease, so an agent that lost its token can carry on; without a `session_ref`
  two agents sharing a name cannot replace each other and need `takeover`. Claims are limited per
  actor (`CLAIMS_PER_WINDOW`). Labels are single-line, capped and stripped of zero-width and
  bidirectional characters, and the names HQ writes under (`hs-...`, `harness-session`,
  `unknown`) are refused, because the task thread is read back as trusted text. A release summary
  is left as a quoted block (`> ...`) so it cannot pass for a line HQ wrote.
- **Time.** An external lease that goes silent for `tasks.lease_ttl_secs` (default 900) ends at
  its last heartbeat, so a crashed session adds no phantom time. A spawned session's lease has no
  ttl: `mission::record` opens it on launch, resume, a new instruction or linking a running
  session, and closes it when a turn finishes (`released`) or the session exits, is stopped or
  stalls at launch (`session_ended`). Closing does not need a heartbeat, so a daemon restart
  loses nothing, and any read that checks leases also closes one whose session is gone or no
  longer running, so a missed exit cannot hold a task forever. A session holds at most one open
  lease (a unique index), and `mission::record` runs in one transaction.
  `tasks.lease_ttl_secs` is never taken below `MIN_LEASE_TTL_SECS` (60).
- **A stale token cannot rewrite the task.** A lease that already expired can still report where
  the work stands, and its summary is left on the thread, but its status change is applied only
  while nobody else holds the task and nothing else has moved it since the lease ended
  (`status_applied` in the reply says which).
- **Attribution is per task.** A lease names who is acting on any task, but an event is recorded
  under the lease (`work_session_id`) only on the lease's own task.
- **`tasks.require_lease`** is `off` (default), `warn` or `enforce` and applies only to agents
  calling `task_update` over MCP when they start a task. The web board never asks a person for a
  lease. `warn` adds a warning to the reply, `enforce` refuses and says how to claim. Settings are
  read when the tools are built, so a change applies on the next start.
- **Reading.** `task_get` returns `held_by` and the 20 latest `work_sessions`;
  `GET /api/tasks/{id}/work-sessions` returns up to 50 for the web UI. Neither ever returns a
  token or its hash. A restricted audience's `task_get` has `work_sessions`, `held_by` and the
  events' actor fields removed, since they carry host, directory and branch.
- **Not here yet.** Reading the transport's `clientInfo` (see `TECHDEBT.md`), the resume packet
  and staleness sweep (WS5), and time summaries (WS3).

Rollback of 081 (SQLite 3.35+), then delete the `081_task_work_sessions` row from
`schema_version`:

```sql
DROP TABLE task_work_sessions;
ALTER TABLE task_events DROP COLUMN actor;
ALTER TABLE task_events DROP COLUMN work_session_id;
```

## Time on a task (TSV2 WS3)

Three sources, kept apart because they answer different questions. Anything unknown is `null`,
never a guess.

- **Leased time** is the union of the task's work lease intervals, so two sessions working at
  once count once. An external lease counts up to its last heartbeat; a spawned session's open
  lease counts up to now.
- **Status time** is read from the event log: seconds in each of to_do, in_progress, blocked and
  ready_for_review since creation (time in complete is not time spent). It is `null` for a task
  with any event from before the full log (migration 080), because the gaps cannot be filled
  honestly. `time_to_start` is creation to the first in_progress. `cycle_seconds` is that first
  start to `completed_at`, only for a complete task that recorded both.
- **Estimate** is `estimate_minutes` (1 to `MAX_ESTIMATE_MINUTES`, one year), settable on create
  and update over MCP, REST and the web forms. `variance_minutes` is leased minutes minus the
  estimate. A parent also reports a `subtasks` rollup: leased seconds, summed estimates and how
  many sub-tasks have one.

Read it with `task_get` (`time`), `GET /api/tasks/{id}/time`, `task_time_report` (MCP),
`GET /api/task-time-report?days=` and `hq task time [days]`: leased hours, completed tasks, mean
cycle time and actual over estimate per initiative, and leased hours per agent. A completed task
with no recorded start is counted in `unknown_tasks`. Only the part of each lease inside the
window counts, per initiative and per agent (two sessions of one agent at once are one agent's
time). Reading time closes leases that went silent first, so `live` and the totals never include a
session that is gone. A malformed `estimate_minutes` (a string, 30.5) is an error over MCP, never
a silent clear or drop.

The first claim of a task with no start date stamps today as its start date, so the timeline can
place it; an existing date is kept, and a task already past due is left unscheduled because a
start after the due date is invalid. The timeline draws a thin bar under each planned bar for the
days work leases covered (`GET /api/work-sessions?days=`).

**Timestamps stay UTC `YYYY-MM-DD HH:MM:SS` on the wire.** Every client already reads them as UTC
(`parseSqliteUtc` in the web app), and changing the format would break them for no gain. The web
shows times in the viewer's timezone with its abbreviation. Dates (`start_date`, `due_date`) are
whole UTC calendar days; "today" is the viewer's local date. `parseDay` now rejects a date that
does not read back the same (`2026-02-30`), which used to roll over to March 2.

Migration `082_task_estimate` adds one nullable column. Rollback (SQLite 3.35+), then delete the
`082_task_estimate` row from `schema_version`:

```sql
ALTER TABLE tasks DROP COLUMN estimate_minutes;
```

## Typed links (TSV2 WS4)

Migration `083_task_links` adds `task_links`: a task's link to a vault note, chat thread, session,
commit, pull request, URL or another task, with a direction (`origin` for what it came from,
`produced`, or `related`), an optional label and who added it. The `Source:` line in a promoted
note's description stays for people; the link is what code reads.

- **One thing is one row.** A `ref` is checked and normalised per kind before it is stored: a note
  is a vault-relative path collapsed to one spelling (`./a//b/` is `a/b`; no absolute path, drive
  letter, `..` or backslash), thread and session refs are plain ids, a commit is a sha or
  `owner/repo@sha`, a pull request is `owner/repo#123` (a github.com pull request URL is accepted
  and normalised to it; `#007` is `#7`), repositories are lowercased and are exactly `owner/repo`
  with no `.` or `..` part (a browser would resolve those to another repository), a URL is http or
  https without spaces, and a task link stores the other task's internal id. Adding a link twice
  returns the first. A task keeps at most 100.
- **Both directions.** `task_link_add`, `task_link_remove` and `task_link_list` (by task, or by
  `kind` and `ref` to find the tasks that link to a note or a thread). `task_get` returns `links`.
  `task_create` takes a `links` array, and a malformed link means no task is created.
  `GET /api/tasks/{id}/links` and `GET /api/tasks/linked?kind=&ref=` serve the web.
- **Recorded without being asked.** A task created from HQ's own web chat gets that thread as an
  `origin` link (the chat's toolset is built with its thread), a task created by a launched session
  gets that session, and `task_create_from_note` links the note. Promoting a note that already
  started tasks says so in `already_linked`.
- **Only HQ records origins of chats and sessions.** A `chat_thread` or `session` link an agent
  writes is stored as `related` even if it asks for `origin`, so an injected turn cannot claim a
  conversation it was not in.
- **Hidden from restricted audiences.** Their `task_get` has no `links`, their `task_create` takes
  no `links` and its reply drops `links` and the advice block (which names other tasks), and the
  link tools are denied to them.
- **Web.** The drawer lists a task's links with icons and opens each (note, session and task inside
  the app, the chat thread through `/chat?thread=`, pull requests and commits on GitHub, URLs only
  when http or https). A note's page lists the tasks that started from it.

**Deterministic advice on create.** `task_create` returns, never blocking and never writing:
`similar_open_tasks` (open tasks that read like the new one: score at least 0.45 on the existing
TF-IDF, tag and initiative score, with the shared terms as evidence; its parent, sub-tasks,
siblings and dependencies are left out because they are related on purpose, and open and completed
lookalikes are ranked apart so many finished ones cannot hide an open one) and, when no estimate was
given, `suggested_estimate` (the median leased time of similar completed tasks, the middle of the
two when there is an even number, rounded to five minutes, only when at least two have real leased time). `similar_to_text` scores text that is not
yet a task against the live tasks the way the stored index would, so it needs no index write and
no LLM call.

Rollback of 083, then delete the `083_task_links` row from `schema_version`:

```sql
DROP TABLE task_links;
```

## Durable and long-horizon work (TSV2 WS5)

Migration `084_task_durability` adds the columns and tables below. Nothing here changes a task's
status on its own: staleness is reported, never acted on.

- **Why a task is blocked.** `blocked_reason`, `waiting_on` and `blocked_since` (one line each, at
  most 500 characters). Over MCP, moving a task to blocked needs a `blocked_reason`, and releasing
  a lease as blocked takes the summary as the reason. The web board and REST do not insist, since
  a person dragging a card should not be stopped. All three clear when the task leaves blocked,
  and a reason on a task that is not blocked is refused.
- **Long-horizon mode.** `long_horizon` on a task means a harness session finishing a turn or
  exiting no longer moves it to ready_for_review or blocked (`mission::target`). Starting work
  still moves it to in_progress, the comment is still written and the session's lease still
  closes. Set it on create or update.
- **Checkpoints, the resume packet.** `task_heartbeat` and `task_release` take a `checkpoint`
  (`summary`, `next_step`, `open_questions`, `files`); a release with a summary and no checkpoint
  uses the summary. `task_claim` returns the latest as `resume` and `task_get` as `checkpoint`,
  labelled as notes from an earlier session rather than instructions. Text is cleaned (no control or
  invisible characters), capped at 2000 characters, at most 50 files, and the latest 50 per task are
  kept. The drawer shows it as "Where it left off".
- **Removing archives.** `task_delete`, `DELETE /api/tasks/{id}` and the web delete button archive:
  the task is hidden from lists and counts but keeps its comments, events, leases, links and
  checkpoints, and live leases on it end. `task_restore` and `POST /api/tasks/{id}/restore` bring
  it back with the sub-tasks archived at the same moment, and a sub-task cannot return before its
  parent. `task_list archived=true` and the web Archived tab list them. Only an archived task can be
  purged (`task_delete purge=true`, `DELETE ...?purge=true`), which removes everything it holds.
  Each archive, restore and purge writes a `task_audit` row that outlives the purge. An archived
  task cannot be changed or claimed, does not block anything, and frees its `external_id`, so a retry
  after a deliberate delete makes a new task.
- **Stale tasks.** An in-progress task with no live lease and no write, comment or lease heartbeat
  for `tasks.stale_after_hours` (default 72) is stale. `task_stale` lists them each with a suggested
  action and why (review the sub-tasks, close or release a task untouched for 30 days, resume one
  with a checkpoint, otherwise release), `task_list stale=true` filters to them, `task_get` has a
  `stale` flag, `GET /api/tasks/stale` serves the web (a "stale" chip on cards and the drawer), and
  the daemon's `task-stale-digest` posts one web-only item a day. None of them changes a task.
- **Epics are initiatives.** Nesting stays one level. A large piece of work is its own initiative
  with a task per workstream and sub-tasks under those, as this epic is filed. `initiative_progress`
  and `GET /api/initiatives/{id}/progress` roll it up: tasks by status, share complete, summed
  estimates, worked time and stale count.
- **Archived tasks stay out of everything.** They do not launch or report sessions (a session
  already running on one stops recording to it), take sub-tasks or dependencies, appear in the
  graph, block anything, or count in progress, time or link lookups. A restricted audience's
  `task_get` also omits the checkpoint, which names files and actors, and a restricted chat cannot
  attach a watch to a task. A late release from a lease that no longer holds the task writes no
  checkpoint, so it cannot replace the resume point a newer session left. A blocked reason is
  judged by what would be stored (a lone control or zero-width character is no reason), and a
  blocked task's reason cannot be cleared over MCP, only replaced or the task unblocked.
- **Watches can follow a task.** `watch_create task_id=...` links a recurring watch to an open task:
  each changed result is left on the task as a quoted note (`watch-<ref>`, at most one per 30
  minutes, and never counted as work for staleness), the watch's end is noted too, and the watch
  stops by itself once the task is complete, archived or gone, telling its chat.

Rollback of 084, then delete the `084_task_durability` row from `schema_version` (SQLite 3.35+):

```sql
DROP TABLE task_checkpoints;
DROP TABLE task_audit;
DROP INDEX IF EXISTS idx_tasks_archived;
ALTER TABLE tasks DROP COLUMN blocked_reason;
ALTER TABLE tasks DROP COLUMN waiting_on;
ALTER TABLE tasks DROP COLUMN blocked_since;
ALTER TABLE tasks DROP COLUMN long_horizon;
ALTER TABLE tasks DROP COLUMN archived_at;
ALTER TABLE background_turns DROP COLUMN watch_task_id;
```

Archived tasks must be restored or purged before rolling back: the old code would show them.

## The agent protocol (TSV2 WS6)

Any agent, whatever harness it runs in, can use tasks as its headquarters. Migration
`085_task_assignees` and the tools below make the loop short and self-explaining.

- **Assignees, apart from tags.** A task has `assignees` (agents or people; at most 20) and
  `tags`. An assignee says who the task is for, and is mailed when added; a tag says what it is
  about. While `tasks.route_tags` is true (the default) a tag that names an agent mailbox still
  routes to it, so nothing existing breaks. `task_routing_audit` lists each mailbox with its open
  tagged tasks, assigned tasks and `tagged_but_not_assigned`, changing nothing. Assign those, then
  set `tasks.route_tags: false` and tags are purely topical. A name that is both is mailed once,
  and an edit mails only who it newly added. Claiming a task assigned to someone else works, with
  a warning.
- **Finding work.** `task_list` takes `assignee` (your queue), `search` (every word, in the title,
  description or display id), `updated_since` and `sort` (`updated`, `created`, `priority`), and
  replies with `as_of`, a time a second before the read, to pass as the next `updated_since`: a
  change may be seen twice, never missed. `task_next {actor}` picks the most urgent `to_do` task
  assigned to you (or nobody, with `include_unassigned`) whose dependencies are done and that
  nobody holds, then soonest due, then oldest, and claims it in the same write, so two agents asking
  at once never get the same task. It answers like `task_claim`, or `task: null` and why.
- **Bulk.** `task_create_many` (up to 100) takes `tasks`, `defaults` and `external_prefix`; a later
  item refers to an earlier one as `@key` in `parent_id`, `depends_on` or a task link, and the
  prefix makes the call safe to repeat. `task_update_many` takes `updates` and `defaults`. Each item
  runs through the single tool, so every rule and notification is identical; a bad item fails alone
  and is reported.
- **Telling an agent how.** Three channels, none needing setup:
  1. the MCP server `instructions` every client receives on connect carry a short "Working on
     tasks" block (present exactly when the task tools are);
  2. the first task read or write from a name that holds no lease gets one `hq_task_protocol`
     note in the reply, once per name per process, never to a launched session or a lease holder;
  3. `hq task install-skill [claude|codex|cursor|all] [--dry-run] [--force]` writes the generic
     `hq-tasks` skill (the source is `crates/hq-cli/assets/skills/hq-tasks/SKILL.md`) into each
     agent's skills directory (`~/.claude/skills`, `~/.codex/skills`, or a Cursor rule), updating
     HQ's own copy in place and leaving one a person edited unless `--force`. With no name it
     installs only for agents found on the machine.
- **Tool text.** Every task tool description is at most 800 bytes and says what to call next. A
  malformed argument is an error that says what to pass, never a silent no-op.
- **Acceptance.** `crates/hq-mcp/tests/fresh_agent_task_loop.rs` drives the real gateway as a
  client that knows nothing: connect, find work, claim, comment, heartbeat with a checkpoint,
  link, be refused a blocked update without a reason, release blocked, then a second agent resumes
  from the checkpoint. It also checks that a wrong move is answered with what to do. A release with
  only a summary becomes the new resume point but keeps the last checkpoint's next step, open
  questions and files.

Real-harness acceptance (run against a throwaway vault over HTTP `/mcp`, no skill installed, prompt
"you are <name>, use the hq MCP server to find the work assigned to you and do it"):

- Claude Code with a small model: `task_next`, `task_comment_add`, `task_release` and done. One
  stumble, `task_next` refused a call that named only `assignee`; it now takes that as the actor too.
- Codex: first run found nothing because HQ answered the `notifications/initialized` notification
  with a JSON body, which Codex's client rejects; HQ now returns an empty `202 Accepted`. Codex
  also needs its per-server tool approval set for non-interactive use (`default_tools_approval_mode`).
  With both, it found the task through `hq_discover`/`hq_call`, did the work, and released it.
- OpenCode (a free hosted model): one error on an empty `task_next`, recovered from the message alone,
  then claimed, commented and released as `ready_for_review`.
- Copilot CLI: completed the loop but marked the task `complete`. It never received the server
  instructions (they need `--allow-all-mcp-server-instructions`) and discovery truncates tool
  descriptions, so the claim reply's `next` now states the status rule itself.
- Cursor agent (project `.cursor/mcp.json`): claimed, commented and released as `ready_for_review`.
- With `AGENTHQ_API_KEY` set (unkeyed calls refused): Claude Code, with the `hq-tasks` skill placed in
  its project, and Codex (key via `bearer_token_env_var`) both finished the loop and ended at
  `ready_for_review`. Codex needs stdin closed (`< /dev/null`) when scripted.
- Auth on, via each tool's own header setting: Cursor agent (`--model auto`; its default model needs a paid
  plan) and OpenCode ended at `ready_for_review`. Copilot CLI finished the loop but again chose `complete`
  even though the claim reply says to use it only for verified work; HQ states the rule and does not
  enforce it.
- Multi-session handoff on one three-step task, one step per session, lease TTL 60s: Claude Code did
  step 1 and left a checkpoint naming step 2; Codex, with no shared memory, read it and did step 2;
  an agent then claimed the task and vanished; a claim by another agent was refused with the holder's
  name and the way out; after the lease expired OpenCode took it, did step 3 and released it
  `ready_for_review`. The record shows six leases (the dead agent's two as `expired`) and 83 seconds of
  leased time against 297 seconds in progress. A task stays `in_progress` after a lease expires and
  shows as stale instead, by design.
- Not run: Gemini (not installed here). The longest job was minutes, not days.

Rollback of 085, then delete the `085_task_assignees` row from `schema_version`:

```sql
DROP TABLE task_assignees;
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
