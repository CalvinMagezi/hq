---
name: hq-task-brief
description: Write HQ tasks another agent can pick up cold. Describe behaviour and interfaces instead of file paths, split big work into vertical slices that each fit one session, order them with depends_on, and give every task testable Acceptance lines and an Out of scope list. Use when creating or breaking down tasks with task_create or task_create_many, or when asked to plan, split, spec or brief work.
hints: [task_create, task_create_many, depends_on, acceptance, out of scope, vertical slice, task brief]
provenance:
  mintedBy: hq
---

# Writing task briefs

A task outlives the conversation that made it. Whoever picks it up, an hour or a month later, starts from the task alone and explores the code fresh. Write for that reader. Create tasks with `task_create` (one) or `task_create_many` (a set).

## One brief, five parts

Put these in the description, in this order:

1. **Summary.** One line: what has to be true when this is done.
2. **Current and desired behaviour.** What happens today (for a bug, the broken behaviour), then what should happen, including edge cases and error conditions. Say what a person or caller sees, not which functions to edit.
3. **Key interfaces.** Name the types, tool names, config keys or signatures that change and why. Names survive refactors; file paths and line numbers do not, so leave those out. Quote a snippet only when it encodes a decision prose cannot, such as a schema or a state machine.
4. **Acceptance.** An `## Acceptance` section, one testable line each: a command to run, an output to see, a case that must pass. "Works correctly" is not a line. Each line must be checkable on its own. HQ's `task_create` wants this section, and the verifier reads it.
5. **Out of scope.** What looks related but must not change in this task. It stops gold-plating and tells the next agent where to stop.

## Slice vertically

Break big work into tracer bullets: each slice cuts a thin but complete path through every layer it touches (storage, logic, interface, tests), so it can be shown or verified alone. Do not make one task per layer.

- Size a slice to one working session. If it needs more, split it.
- Do any prefactoring first, as its own slice.
- A mechanical change that touches everything (a rename, a retyped shared symbol) cannot land as a slice. Add the new form beside the old, migrate callers in batches that each have a task, then remove the old form in a last task that depends on every batch.

## Wire the order with depends_on

Give each task the tasks that must finish first, and nothing more. In one `task_create_many` call, give every item a `key` and refer to earlier items as `@key`:

```
task_create_many {
  external_prefix: "<feature-slug>",
  defaults: { initiative_id: "<initiative>", tags: ["<area>"] },
  tasks: [
    { key: "store",  title: "...", description: "..." },
    { key: "api",    title: "...", depends_on: ["@store"], description: "..." },
    { key: "screen", title: "...", depends_on: ["@api"],   description: "..." }
  ]
}
```

`external_prefix` makes a retry safe: the same call creates nothing twice. Put blockers first in the list. Tasks with no `depends_on` can start at once, and the ones whose blockers are done are the frontier to work next. Show the breakdown to the owner and settle the granularity and the edges before you create anything large.

## Before you create

- Could someone with no access to this chat do it from the description alone?
- Does every Acceptance line name something you could check?
- Is anything in Out of scope that a reader might otherwise assume is included?

Adapted from the to-tickets and AGENT-BRIEF skills in mattpocock/skills (MIT, Matt Pocock).
