---
name: hq-wayfinder
description: Chart work too big for one session as a map (an initiative plus a vault note) of decision tasks, then resolve them one at a time until the way is clear. Use when asked to plan, scope or find the way through a large, foggy effort, or to work the next task on an existing map. Plans decisions; it does not build.
hints: [wayfinder, wayfinding, decision map, map note, frontier, fog of war, decision tasks, big effort]
provenance:
  mintedBy: hq
---

# Wayfinding: charting work bigger than one session

Use this when a loose idea is too big for one agent session and the way to the end is not visible yet. You chart the way as a map, then resolve its decision tasks one at a time until the route is clear. This is planning: each task resolves a decision, and the map is done when nothing is left to decide before someone goes and builds it. Wanting to just do the work usually means you have reached the edge of the map and should hand off. If the map's Notes say otherwise, follow the Notes.

If the whole journey fits one session, there is no fog and no map is needed. Say so and ask how to proceed.

## The map

A map is an initiative (`initiative_create`) plus one vault note, linked from the initiative. The note is an index: it gists decisions and points at the tasks that hold the detail. A decision lives in one place, its task.

```
## Destination
<what reaching the end looks like: the spec, decision or change. One or two lines.>

## Notes
<domain, skills to consult, standing preferences for this effort>

## Decisions so far
- [<task title>](<link>): <one-line gist of the answer>

## Not yet specified
<fog: questions you can tell are coming but cannot phrase sharply yet>

## Out of scope
<work ruled beyond the destination, with the reason. Never graduates.>
```

Name the destination first, with the owner. It sets the scope of everything else.

## Decision tasks

Each task in the initiative is one question sized to a single session, tagged `wayfinder:<type>`. Write the description with the hq-task-brief skill (Question, Acceptance, Out of scope). The type is in the tag, not the description.

- `wayfinder:research` (AFK): read docs, APIs or local knowledge to surface a fact a decision waits on.
- `wayfinder:prototype` (HITL): make a cheap, rough artifact for the owner to react to. Link it, do not paste it.
- `wayfinder:grilling` (HITL): a conversation, the default type. Question one thing at a time and wait for the owner's answers.
- `wayfinder:task` (AFK or HITL): manual work that must happen before a decision can be made (sign up, get access, move data). It unblocks a decision; it is not the destination.

HITL tasks resolve only through a live exchange with the owner. Never answer for them, and do not claim one when the owner is not in the session.

Create the tasks you can specify now with one `task_create_many`, using `@key` in `depends_on` to wire the order (see hq-task-brief). Everything you cannot yet phrase sharply stays in Not yet specified. Test: can you state the question precisely, even if you cannot answer it yet? Then it is a task. Otherwise it is fog.

## The frontier

The frontier is the work takeable now: tasks in the initiative with `status` to_do and an empty `blocked_by`. `task_list` rows carry `blocked_by`, so list the initiative's tasks and filter client-side.

`blocked_by` holds only blockers that are not complete (and not archived); a completed blocker drops out on its own. Checked in `crates/hq-db/src/tasks/rows.rs`: a dependency is listed in `depends_on` always, and in `blocked_by` only while its status is not complete. So "blocked_by is empty" and "every depends_on task is complete" are the same test. `depends_on` holds internal ids and `blocked_by` holds display ids, so do not compare the two lists directly. A complete task can still list an open blocker, which is normal.

Skip any frontier task another session has claimed. `task_claim` takes a lease, and that lease is the claim.

## Working a map

Never resolve more than one task per session, except research tasks, which can be resolved together.

1. Read the map note once. Do not load every task body.
2. Pick the task: the one the owner named, otherwise the first frontier task. `task_claim` it before any work.
3. Resolve it as its type. Read the tag, not just the description. Fetch related closed tasks on demand.
4. Record it: post the answer with `task_comment_add`, then `task_release` with `ready_for_review`. Append one line to Decisions so far in the map note with `vault_patch_section`. If you made an asset, link it with `task_link_add`.
5. Graduate fog the answer has made specific into new tasks (create, then wire) and clear it from Not yet specified. If a task turns out to sit past the destination, close it and note it in Out of scope, not in Decisions so far. If the decision invalidates other tasks, update or remove them.

Other sessions may work the same map, so re-read before you write to the note.

## Research on cheap lanes

Research tasks need no owner, so hand them to a cheap lane: `harness_session_spawn` an agent on pi or agy with `task_id` set to the research task and `purpose: research`. They resolve in parallel while you continue. Take the results back to the map the same way as any other resolution.

## Refer by title

In everything the owner reads, name a task or map by its title, never by a bare id. A wall of ids is unreadable. A title can wrap its link.

Adapted from the wayfinder skill in mattpocock/skills (MIT, Matt Pocock).
