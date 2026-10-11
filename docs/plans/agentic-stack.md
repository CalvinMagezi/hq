# The Agentic Stack: general-purpose agents that pull work

Status: implemented on the branch this document arrived with. The first cut of
general-purpose agents (Hermes, OpenClaw) ran them as interactive panes that HQ
typed into. This plan replaces the slowest part of that with a model where a
named agent in your stack pulls tasks assigned to it and runs each one as a
single, bounded, sandboxed job. The interactive panes stay for people.

## Where the first cut choked the agents

- HQ drove agents with real APIs through a terminal. Hermes has `hermes chat -q`
  for one-shot runs; OpenClaw's automations run `openclaw agent --message` in an
  isolated session. Screen-scraping a TUI that has no state rule file left every
  session as `unknown`: no wait, no finished alert, messages typed in blind.
- HQ took itself away from the agent, then re-fed it by hand. A general-purpose
  agent gets no HQ token, so the supervisor pasted untrusted screen text into its
  prompt on every observed event. That is both the injection surface we worry
  about and a token cost per event.
- NanoClaw is the wrong shape for a pane. It is a Node process that runs Claude
  Agent SDK sessions in containers, driven by WhatsApp and a polling loop. There
  is no interactive CLI to drive. It is dropped.
- Purpose was enforced only at spawn. Nothing stopped a general-purpose actor
  from pulling a coding task through `task_next`.

## What was borrowed

**Paperclip** (open-source "zero-human company" orchestration): tickets with
goal ancestry, agents that wake on an event (assignment, mention) or a
heartbeat, atomic task checkout tied to a per-agent budget that pauses the agent
at its ceiling, and humans as the board with pause and approval. Its known pain
is that timer heartbeats burn tokens, so event triggers come first. Most of this
already existed here under other names: tasks with spaces, initiatives,
sub-tasks and checkpoints are the tickets; `claim_next` is the atomic checkout;
assignees are the event trigger; `/watch` is the heartbeat; Drive on and off,
stop and action-needed items are the board.

**The Kola Academy Agentic Freelancing Virtual Internship** (September 2026
cohort), whose language this document and the code use instead of an org chart:

- *Agentic Stack*, not org chart. Your stack is the set of specialists you
  direct. "Specialists, not clones": drafting, research, client comms, workflow
  ops. HQ's config block is `stack:` and each entry is a *specialist*.
- *Outcome orchestration, not task execution.* "You're the account, not a pair
  of hands. Judgment is the job." A specialist's run ends in `ready_for_review`,
  never `complete`; a person signs off.
- *The orchestratable-task test.* Hand off work whose first draft is mostly
  mechanical, that you have done twice before, where a mistake is cheap to
  catch; keep the review. Tasks assigned to a specialist are exactly that class.
- *Blast radius.* "Split planning from execution. Scope your shared context.
  Limit what you haven't tested: read-only until you trust it." A specialist
  runs in the process sandbox, with its own state directory, no HQ token, and
  reaches only the hosts you allow. HQ reads its output as untrusted text.
- *Trigger, links, output; checkpoints where the chain waits for you.* The
  trigger is an assignment, the run is the link, the output lands on the task.
  The chain stops for a person at review, and at the four non-negotiables the
  course names (money, a first message to a new client, legal advice, low
  confidence), which is why a specialist never sends anything on HQ's behalf.
- *Definition of done, documented test run.* A task's description is the brief
  and the definition of done; the run's output is kept with the session so the
  reviewer can read the actual result, not a summary of it.

## The shape

```yaml
stack:
  specialists:
    - name: hermes                 # tasks assigned to "hermes" are its queue
      harness: hermes              # a built-in general-purpose harness or a profile
      purposes: [research, general] # its lane; code is never accepted
      cwd: ~/stack/hermes          # where runs start; default: stack-<name> in the host's HQ folder
      host: native                 # optional; any paired host
      run_minutes: 30              # a run longer than this is stopped
      runs_per_day: 24             # budget in the only currency we can measure
```

1. **Trigger.** Assign a task to the specialist's name, in the web board, by
   `task_update`, or from chat. The dispatcher (daemon task `stack-dispatch`,
   every minute) asks the task queue for the next task assigned to that name:
   open, dependencies done, nobody holding it, start date reached, most urgent
   first. The same query `task_next` uses, with one extra clause: a
   general-purpose specialist is never offered code-shaped work (a task tagged
   `code`, or linked to a commit or pull request).
2. **Run.** One run per specialist at a time. The dispatcher builds a brief from
   the task (display id, title, description, initiative, the resume point from
   its last checkpoint, and what to print at the end) and launches the harness's
   one-shot command in the host's sandbox: `hermes chat -q "<brief>"`,
   `openclaw agent --message "<brief>"`. A profile can replace the command for
   an install that differs (`one_shot_args`, with `{brief}`). The session is an
   ordinary harness session with origin `stack`, so the Workbench, `hq sessions`
   and `harness_session_logs` show it like any other.
3. **Output.** The process exiting is the completion signal. The supervisor
   keeps the final screen, closes the pane, and records `RunEnded` on the task:
   lease released, status `in_progress` to `ready_for_review`, with a comment
   that points at the session's logs. A run that exits non-zero is recorded as
   `RunFailed`, which blocks the task as a crashed session does; a run is never
   resumed. A run past `run_minutes` is stopped. A launch that fails leaves one
   comment and backs off for half an hour, and the next task in the queue runs
   meanwhile.
4. **Budget.** `runs_per_day` is counted from the session registry. Harness
   token usage is collected after the fact only for the coding agents whose
   transcripts HQ can read, so money is not a currency HQ can enforce for
   these agents yet; runs and minutes are.
5. **Fire now.** `hq stack run <specialist> <task>` starts a run for one task
   without waiting for a sweep (the task does not need to be assigned). `hq
   stack list` shows each specialist, its lane, what is running and the day's
   count.

What did not change: the interactive general-purpose sessions, the purpose rules,
monitor and review semantics, observers. What HQ still does not do: give a
specialist a token or MCP wiring, or let it move a task to complete.

## Why not the OpenClaw backend experiment

OpenClaw's gateway speaks the OpenAI chat-completions API, so a `backends:`
entry could let `spawn_subagents` use it with no pane at all. That works only
when HQ can reach the gateway: OpenClaw on the same machine as HQ, or exposed
over a private network such as a tailnet with its bearer token. HQ on a VPS
driving an OpenClaw on a laptop cannot, and the gateway's own
token-on-localhost model is not meant to be exposed. The host-based run above
already crosses machines, because a paired host (`hq host join`) runs the pane
where the agent is installed. The backend route stays an option for the
same-machine case and needs no code.

## Follow-ups, in order of value

1. State rule files for Hermes and OpenClaw in the host, so interactive sessions
   get `idle` and `working` and the finished-turn alert.
2. Let a specialist read HQ. A scoped session token (`task_get`, comments, logs
   of the sessions it was given) would let a reviewer pull what it needs instead
   of receiving pasted screens. Needs the MCP config writer in the host to know
   each agent's config format.
3. Cost per specialist, once the usage collector can read Hermes's and OpenClaw's
   own transcripts.
4. A `task_next` caller on the tasks scope could pass its category, so a
   non-HQ-launched general-purpose agent gets the same no-code clause.

The launch arguments for Hermes and OpenClaw are the ones their documentation
gives and were not run against a live install here.
