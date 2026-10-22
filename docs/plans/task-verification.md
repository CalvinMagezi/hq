# Task verification: completion without a person in the queue

Status: approved 2026-10-22 (see "Decisions"). Work streams 0 to 6 are built and described for
operators in `docs/TASK_VERIFICATION.md`. Not built: WS7 (rollout on a live instance), WS8 (sweeps),
WS9 (briefing and queue health) and WS10 (web and phone surfaces). Where the build differs from this
text: the report and the audit commands are `hq verify` (not `hq tasks verification`); an inconclusive
verdict that waits on the worker returns the task to `to_do` like a fail, rather than blocking it
(decision 4); and `waiting_for` is stored, so a restart forgets nothing.

One person cannot review every task that several agents finish. This plan takes
the owner out of per-task review and replaces it with a gate that moves a task
to `complete` only when evidence the worker could not write says it is done,
and that measures its own accuracy so the owner can trust it without reading
each result.

## Why this is needed

Observed on one working instance, 2026-10-21:

- About half as many tasks sat in `ready_for_review` as had ever been completed,
  and 60% of those had not been touched for a week. `notify_ready_for_review`
  emits one `ActionNeeded` item to the owner for every one of them, so the queue
  grows at the rate agents finish, and review capacity does not.
- `ready_for_review` is doing three jobs: "finished", "paused, waiting for the
  owner" and "I stopped". A parent task sat there with four of seven sub-tasks
  open and its own exit criterion unmet.
- Completed tasks often carry no link to a pull request or commit, no session
  and no checkpoint, so nothing records that any check ran. Several tasks had
  identical-second status changes across different tasks (a bulk flip).
- `complete` is reachable from any status by anyone. Completing a task mails the
  agents on every task it unblocks, so a false `complete` starts dependent work
  on something that is not done.
- The acceptance label that feeds skill outcomes counts review to complete as
  accepted whoever completes it, so a self-approved task teaches the skill
  flywheel that it worked.

## A principle this plan reverses

Several places state that HQ never marks a task complete and a person signs off:
`crates/hq-tools/src/harness_session/mission.rs` and `stack.rs`,
`crates/hq-tools/src/subagent_runs.rs` (the `subagent_run_review` description),
`docs/plans/agentic-stack.md`, `docs/AGENT_SESSIONS.md`, `docs/PERSONAL_AGENTS.md`,
and the web chat driver's prompt in `crates/hq-web/src/session_driver.rs`
("Never mark the task complete; the user verifies that"), which a test pins.
This plan changes that: HQ completes a task when a verdict passes, and the
person is authority over policy, not a reader of every result. Those places must
change together (work stream 6). The `hq-tasks` skill is softer ("do not mark a
task complete unless you verified it") and needs rewording, not reversing.

## What this is not

- Not "an LLM looked and said fine". A model reading a model's summary shares its
  blind spots and agrees too easily. Trust here comes from observed facts and
  from a measured error rate, not from a reviewer's opinion.
- Not a guard for irreversible actions. A deploy, a payment or a deletion needs
  approval before it happens. Completion review runs after the fact. Keep the
  existing approval path for those and do not lean on this gate.
- Not zero humans. The owner stays at three points: the policy (once), a small
  weekly audit sample, and the few exceptions the gate cannot settle. That is
  tens of items a week instead of a queue of hundreds, and the audit is what
  keeps a closed loop of agents from drifting.

## Principles

1. **Evidence the worker cannot author.** A worker's summary points at an
   artifact; it is never evidence. Evidence is a fact HQ observed or an
   independent system produced.
2. **Criteria fixed before the work.** The task's acceptance criteria and checks
   are recorded at creation or claim and hashed. After a claim they can be added
   to, never removed or weakened; a change resets verification.
3. **Independence from identity HQ issued.** Free-text actor names are not
   identity. The verifier is HQ's own runner or an in-process sub-agent whose
   run id HQ issued, compared with every lease, session and event actor on the
   task. An external MCP client can never be the verifier.
4. **A verdict is bound to an artifact.** It names a commit sha or a note content
   hash and is void if the artifact changes before it is applied.
5. **Trust is measured and earned per class.** Canaries, an audit sample and
   escapes produce an error rate. A tier is auto-completed only while its
   numbers hold, and falls back to the owner when they do not.
6. **Forced at the single write surface.** The rule lives in
   `hq_db::tasks` so MCP, REST and the supervisor all meet it.
7. **Every `complete` says how it was verified.** Downstream readers (skill
   outcomes, precedents, reports) weight a machine-and-peer completion above an
   owner override above an unverified legacy one.

## Workflow

```
in_progress ──release / update to ready_for_review──▶ ready_for_review
                         (the only meaning: "verification queued")
                                       │
                 verify-dispatch collects evidence, decides, applies:
                                       │
   pass ─────────▶ complete      (actor = verifier, level recorded, dependents mailed)
   fail ─────────▶ to_do         (findings become the checkpoint; reopen count + 1)
   inconclusive ─▶ blocked       (reason names the missing evidence; waiting_on the creator)
   3 fails, tier-2 veto window, canary alarm ─▶ blocked + ONE owner item with a one-line choice
   owner override at any time (web board or owner key) ─▶ complete, level "owner"
```

Rules that keep the states honest:

- An agent that asks for `complete` is not refused. The call is treated as
  `ready_for_review` and the reply says verification is queued and HQ applies
  `complete`. Existing agents and skills keep working.
- "Paused, waiting on the owner" is `blocked` with `waiting_on`. A task in
  `ready_for_review` whose criteria are visibly unmet, or whose sub-tasks are
  open, is bounced by the first check, not passed.
- A parent is verified last: it needs every sub-task complete plus its own
  criteria.
- A failed task goes to `to_do`, keeping its assignees, so `task_next` offers it
  with the findings in `resume`. It does not wait on any other change.

## Evidence

| Class | What it is | Who produces it |
|---|---|---|
| E1a | PR state: merged, head sha, files changed | HQ reads it with `gh` |
| E1b | Required check runs on the head sha concluded success | GitHub, read by HQ |
| E1c | A probe of a URL or health endpoint named in the contract | HQ |
| E1d | A contract command run in a sandbox at the artifact sha | HQ (later work stream) |
| E1e | A vault note exists with the recorded content hash | HQ |
| E2 | An independent reviewer's per-criterion verdict, each cited to observations | in-process sub-agent |
| E3 | The worker's summary | never sufficient |
| E4 | The owner | override or audit |

Notes on what counts:

- Agents commonly push and merge under one shared GitHub account, so "merged by
  someone else" is not independence. The independent part is the required check
  runs and branch protection. Where a repo has no required checks, E1b is
  unavailable and the result is capped (see tiers).
- HQ finds the artifact from links first, then from the lease: a lease records
  `branch`, so a pull request can be found by head branch when the agent never
  linked one. A task with no discoverable artifact cannot reach a machine level.
- E1d is deliberately later. CI is already an independent executor for code, so
  the first version does not need HQ to run tests itself.

## Risk tiers (policy, set by the owner once)

The tier is the highest of: explicit tag, paths touched, link kinds, and
configured globs. The default is tier 1 for a task with a code artifact and
tier 0 for a task that only touches the vault.

| Tier | Typical work | To auto-complete |
|---|---|---|
| 0 | Notes, research, planning, docs. Reversible and internal. | One independent E2 verdict, every criterion cited to an observation. Level `peer`. |
| 1 | Code with a pull request, repo config, UI. | E1a and E1b, plus one E2 diff review that includes the tamper scan. Level `machine_peer`. |
| 2 | Deploys, migrations and data changes, auth and security, payments, outbound messages, deletion. | E1, two E2 verdicts from different harness families, no tamper flags, and a veto window (default 24 hours) in which the owner may reverse it. |

If a tier needs E1 and none is available, the verdict is `inconclusive`, never a
quiet downgrade to peer-only. A config switch can allow peer-only completion for
code, and it is recorded as such.

## The verdict record

A new append-only table, `task_verdicts`:

| Column | Holds |
|---|---|
| `id`, `task_id`, `created_at` | identity |
| `artifact` | `pr:<owner>/<repo>#<n>@<sha>`, `note:<path>@<sha256>` or `none` |
| `contract_hash` | hash of the criteria and checks the verdict was made against |
| `tier`, `verdict` | 0 to 2; `pass`, `fail` or `inconclusive` |
| `level` | `machine_peer`, `peer`, `owner` or `none` |
| `verifier`, `independence` | HQ-issued identity; `different_session`, `different_harness` or `different_model` |
| `observations` | facts HQ recorded before any model ran: source, fact, time |
| `criteria` | per criterion: text, result, the observation ids it rests on |
| `applied` | 0 in shadow mode, 1 once it moved the task |
| `canary` | set when the task was a seeded test of the verifier |

Observations are written by HQ first and are immutable. The reviewer's answer is
a structured verdict that may only cite observation ids that exist. A `pass`
with a criterion that cites nothing is rejected as invalid. `task_get` shows the
latest verdict as `{verdict, level, tier, artifact, at}`.

## The verifier

A daemon task, `verify-dispatch`, built like
`crates/hq-cli/src/commands/start/daemon/tasks_periodic/stack_dispatch.rs`:

1. Pick `ready_for_review` tasks with no current verdict, oldest first, under a
   run budget (the existing `budgets:` config and `BUDGET_WINDOW` idea).
2. Discover the artifact and gather E1 observations. Deterministic checks first:
   criteria present, sub-tasks done, PR and checks, tamper scan of the diff.
3. If the tier needs E2, run a read-only sub-agent through `spawn_subagents` with
   an adversarial brief: find why this is not done, try to falsify each
   criterion, answer only in the verdict schema. Task text, summaries and
   artifact content are fenced as data, as in context packets. It has no task
   write tools.
4. Validate the verdict: observation ids exist, every criterion is mapped, the
   artifact hash is current, the verifier is independent of every actor on the
   task.
5. Apply through the single write surface with the verdict id, or record it
   unapplied in shadow mode.

The deterministic tamper scan flags: deleted or weakened tests, new `#[ignore]`,
`skip` or `xit`, edits to CI workflow files, relaxed lint or `allow(` settings,
and updated snapshots. A flag does not fail the task. It raises the tier and
obliges the reviewer to address it by name.

## Enforcement

`hq_db::tasks::apply_update` (the one path MCP, REST and the supervisor share)
gains one guarded edge: a transition to `complete` needs a verdict id from the
verifier or an owner marker, when `tasks.verification.mode` is `enforce`. Every
other transition stays as free as today; the doc's earlier "no transition table
on purpose" holds for everything else. `WriteCtx` carries the marker.

`notify_ready_for_review` (`crates/hq-tools/src/tasks/json.rs`, called from
`tools_task.rs`, `tools_lease.rs` and `hq-web/src/tasks_api.rs`) stops emitting
on every transition. It emits only the exceptions listed in the workflow, plus
the weekly audit digest.

Config (`tasks.verification`): `mode` is `off` (default, a fresh install is
unchanged), `shadow` or `enforce`; `enforce_tiers`, `max_attempts` (3),
`tier2_tags`, `tier2_paths`, `veto_hours` (24), `audit_rate`, `concurrency`,
`sweep_confirm_over` (25), `owner_queue_alert` (20), `owner_queue_age_days` (3),
`shadow_e2_sample` (50) and a reviewer cost ceiling. Reviewer (E2) runs stay off
until a ceiling is set; E1 verification needs none.

## Calibration: how the owner can trust it unread

1. **Shadow first.** In shadow mode every `ready_for_review` task gets a verdict
   that moves nothing. Compare it with what the owner and agents then did. This
   yields the first accuracy numbers and the real cost per verification before
   anything is enforced.
2. **Canaries.** HQ files synthetic tasks with a planted defect (a failing
   check, an unmet criterion, a fabricated citation) and matching good controls,
   at an unpredictable rate. The catch rate and false-reject rate are tracked
   per verifier configuration and tier.
3. **Audit sample.** The push is a floor: a random sample of auto-completed tasks
   (5%, at least 10 a week) arrives as one weekly digest, one tap each: agree or
   not. You can pull more at any time from the audit view, stratified toward the
   tiers and verifier configurations with the least data. Every disagreement
   reopens the task, raises the push rate for that tier and counts against the
   verifier.
4. **Escapes.** A completed task that is reopened, or that gets a follow-up
   tagged `regression` linked to it, counts as a late failure against both the
   worker and the verifier.
5. **Autonomy is earned.** Starting values, to be set from shadow data: a tier
   auto-completes only with at least 30 audited verdicts, at least 20 canaries
   with a catch rate of 90% or more, and an audit disagreement upper bound of
   10% or less. Below that the tier routes to the owner as exceptions and an
   alert says why. Until there is enough data the report says "insufficient
   data" and gives the count, not a number.

At 20 completions a day and a 5% sample that is one audit a day. A usable
estimate of a 5% error rate (about plus or minus 5 points) needs roughly 60
audited verdicts, so expect a few weeks.

## Threats and answers

| Threat | Answer |
|---|---|
| Worker edits tests or CI so checks pass | Tamper scan, tier bump, reviewer must address it by name |
| Worker weakens criteria after claiming | Criteria hashed at claim, add-only, change resets verification |
| Worker, creator and verifier are one orchestrator | Verifier must be independent of every actor on the task, by issued identity |
| Free-text actor name posing as independent | Names are not identity; only HQ-issued run and lease ids count |
| Verdict reused after the artifact moved | Verdict names sha or hash, re-checked at apply time |
| Reviewer approves everything | Canaries and audit measure it; autonomy withdrawn when they say so |
| Same-model blind spot | Independence level recorded; tier 2 needs different harness families; tier 1 needs a model-independent E1 |
| Injection through task text or artifact | Fenced as data; read-only reviewer; structured verdict only |
| A `pass` with nothing behind it | Every criterion must cite an existing observation, or the verdict is invalid |
| Task with no criteria | Tier 0 only, flagged `assessed`; code tasks are bounced to the creator |
| Verification cost runs away | Budget cap, E1 before any model, bounded attempts |
| Two verifiers racing | Verdicts are append-only; apply is one transaction on the task row |

## Verify and audit are different jobs

| | Verify | Audit |
|---|---|---|
| Who does it | The machine (the verifier) | You |
| What it acts on | Tasks waiting in `ready_for_review` | Tasks already verified and completed |
| What it produces | Verdicts, and with them completed, reopened or parked tasks | Agree or disagree on a verdict |
| What it is for | Clearing the queue | Measuring whether the verifier can be trusted |
| Cadence | Always running; you can also start it | 5% and at least 10 a week is only the floor you are pushed; pull as many as you like |

A human audit can never clear a backlog, it only samples one. So the button you
reach for on a Monday is Verify, and Audit is how you decide how far to trust
what Verify did.

An on-demand audit is stratified, not uniform: it favours tiers and verifier
configurations with the least audit data and the highest risk, so 25 audits on a
Monday teach the system more than 25 random ones. A second opinion on one task (a
fresh independent verifier, on request) counts as an audit datum when it
disagrees with the first verdict.

## Clearing a queue on demand: sweeps

A sweep is a persisted batch over a scope: everything waiting, one initiative,
tasks changed since a date, or a hand-picked set. It survives a restart and can
be paused, resumed or cancelled. The first three steps change nothing.

1. **Plan.** Deterministic collectors only: no model, no spend. Every task goes
   into a bucket. The plan shows counts, the estimated spend and time for the
   reviewer runs it would need, and how many tasks would wait on you.
2. **Run.** Verdicts are produced in priority order: tasks other tasks wait on,
   then tier 2 (its veto clock should start early), then oldest. Progress is live.
3. **Review the buckets, not the tasks.** One summary, one decision.
4. **Apply.** One confirmation applies the policy to the buckets. Above
   `sweep_confirm_over` tasks, apply always needs your tap, whatever the mode.
   Every change carries the sweep id.

Undo restores the previous status of every task the sweep moved that has not been
changed since, and reports the ones it left alone.

| Bucket | Meaning | What apply does |
|---|---|---|
| Passed | Evidence met the tier's bar | Complete, with the level recorded |
| Failed | A criterion is unmet, with the finding | Return to `to_do` with the findings as the checkpoint |
| No artifact | Nothing a verifier could inspect (the approved default) | Return to `to_do` with a note naming what to link; never completed |
| Undecidable | Evidence conflicts, checks still running, or the reviewer was unsure | Left in place and retried; shown to you only after the retries are used |
| Needs you | Tier 2 inside its veto window, three failures, or a canary alarm | Stays in the exceptions list |

An existing legacy queue is handled the same way, which is why the first run over
a large backlog is a plan you read once and an apply you confirm once, not a
review of each task. Unverifiable tasks are never auto-completed.

## Weekend bursts and the Monday briefing

The risk is not the size of one queue. It is that agents produce completions
while you are away and the check-in turns into a pile. Four rules prevent that.

**Verification never waits for you.** `verify-dispatch` runs continuously. A burst
is absorbed by a concurrency cap, the priority order above, and a budget that
degrades instead of stalling: when the ceiling is reached, deterministic (E1)
verification continues for free, and tasks that need a reviewer are marked
deferred with the reason, visible and one tap to raise the ceiling for the day.

**Evidence is captured when the work ends.** The cheapest moment to find a pull
request is the release, while the agent still has it in context. A release to
`ready_for_review` on a task with a code artifact HQ cannot find gets an
immediate reply naming what to link (the lease's branch is tried first). Modes
are `warn` then `enforce`, as with leases. Most of the unverifiable backlog is
created by skipping this step.

**Every waiting task names who acts next.** A task that is not done carries a
`waiting_for`, and only `owner` counts against you.

| waiting_for | Because | Handled by |
|---|---|---|
| `worker` | Failed, or no artifact | Returned to `to_do` with findings; the next claim sees them |
| `ci` | Required checks still running | Re-checked with backoff; never failed for being slow |
| `verifier` | Queued behind a burst | Priority order; capacity is visible |
| `budget` | Ceiling reached | Deterministic checks continue; one tap to raise it |
| `owner` | Three failures, tier 2 veto window, canary alarm, or a policy you must choose | The exceptions list, with a one-line choice |

**The briefing is exception-first.** "While you were away" is built on demand and,
if you set a time, on a schedule. It leads with what needs you, then what was
verified and completed (grouped by initiative with the evidence level), what
failed and was reworked without you, tier 2 completions inside their veto window
(with a veto button), and a suggested audit set. It never lists passes one by
one. If more than `owner_queue_alert` items wait on you, it stops listing and
offers a bulk policy instead ("40 of 47 have no artifact: apply the default
disposition?"), because a long flat list is the bottleneck this plan exists to
remove. Queue health sits beside it: backlog by `waiting_for`, oldest age,
24-hour throughput, the estimated time to clear, and spend against the ceiling.
One alert is raised when the owner-waiting set ages past `owner_queue_age_days` or
the estimated time to clear passes 24 hours.

## Surfaces: web and mobile

The Android app is the same web app in a WebView (`apps/android`), and its
`Alerts` service already turns unread approvals and `notify_owner` messages into
notifications that open `/notifications`. So this is one responsive, touch-first UI
in `apps/hq-web`, plus items on the existing notification path. There is no native
task screen to build.

| Surface | What it does |
|---|---|
| Verification bar on the tasks page | Counts by `waiting_for`, and **Verify now** with a scope picker (everything waiting, this initiative, since a date, selected) |
| Sweep sheet | Plan, estimate, run with live progress, the bucket summary, apply, undo, pause, cancel |
| Briefing card | At the top of the tasks page and on the home screen |
| Audit view | A stack of cards: title, criteria with each check's result, evidence links, verdict and level. Agree, Disagree (reopens with a note) or Skip. **Audit 10 more** whenever you want, with the running error estimate and its sample size |
| Task drawer, Verification section | Verdict history and evidence, **Second opinion**, **Complete anyway** (owner override), **Return to to_do** |
| Queue health | Backlog, ages, throughput, spend, and the deferral reasons |
| Notifications | One summary item for the briefing, and an `ActionNeeded` item only for a decision |

Touch rules: large targets, bottom sheets, a sticky action bar (the pattern
`BulkActionBar` already uses), gestures through `usePointerDrag` and its
long-press, and polling through the existing `usePolled` and `useRefreshOn`.

REST is owner-only: the web credential (the configured token or a paired device),
and never an MCP key of any scope, so no agent can start, apply or undo a sweep or
record an audit.

| Endpoint | Does |
|---|---|
| `POST /api/verification/sweeps` | Plan a sweep for a scope |
| `POST /api/verification/sweeps/{id}/run`, `/pause`, `/cancel` | Run and control |
| `POST /api/verification/sweeps/{id}/apply`, `/undo` | Apply and undo |
| `GET /api/verification/queue` | Queue health |
| `GET /api/verification/briefing?since=` | The briefing |
| `GET /api/verification/audits?n=` and `POST /api/verification/audits/{id}` | Pull a stratified audit set; record a decision |
| `POST /api/tasks/{id}/reverify` | Second opinion on one task |

Agents get one read-only addition: `task_get` shows the task's verification state
and `waiting_for`, so an agent can see why its task is waiting.

## Work streams

Each stream's acceptance is machine-checkable, in the style the plan asks of
tasks.

**WS0. Foundations from the task-system review.** Any lease-bearing call
refreshes the lease and an expired, unheld lease revives; `task_next` offers
orphaned in-progress work first; acceptance counts only when the completing actor
differs from the working actor (`crates/hq-db/src/skill_outcomes.rs`,
`task_signal`). Accept: tests for each; `fresh_agent_task_loop` still passes.

**WS1. Verdict store, config and the guarded edge.** Migration after 094,
`task_verdicts`, `TasksConfig.verification`, the check in `apply_update`, owner
marker on `WriteCtx`, `task_get` summary. Accept: with `enforce`, an agent
`task_update status=complete` from MCP, REST and a supervisor event never yields
`complete` without a verdict; the web board still lets the owner complete;
`off` changes nothing (existing tests unchanged); migration test with rollback.

**WS2. Evidence collectors.** Artifact discovery from links then lease branch;
PR facts and check runs through the `gh` runner in `hq-tools/src/github.rs`
(made reusable); note hash; URL probe; the tamper scan. Accept: fixture-based
tests per collector; a missing `gh` yields "unavailable", never a pass.

**WS3. Verifier runner.** `verify-dispatch` daemon task, the sub-agent reviewer,
verdict validation, independence check, budget. Accept: a verdict citing an
unknown observation is rejected; a worker cannot verify its own task; a changed
sha voids a verdict; shadow mode moves nothing.

**WS4. Workflow plumbing.** Agent `complete` becomes queued review with a clear
reply; release-time artifact check (lease branch first, modes `warn` and
`enforce`); rejection loop (`to_do`, findings as checkpoint, reopen count); parent
rule; a `waiting_for` on every waiting task; exception items replace the per-task
owner notification. Accept: the `fresh_agent_task_loop` acceptance extended with
a fail then pass cycle; a code task released with no findable artifact gets a
reply naming what to link; the owner receives no item for a clean pass.

**WS5. Calibration.** Canary generator, audit digest (push floor) and stratified
audit selection (pull), escapes, autonomy gate, `hq verify` report. Accept: a seeded bad verifier is demoted by the
canaries within the configured window; the report prints counts and "insufficient
data" correctly.

**WS6. Docs and agent guidance.** Replace the "HQ never completes a task" claims
listed under "A principle this plan reverses", including the chat driver prompt
and its test, with the verified-completion rule. Reword the `hq-tasks` skill and
the `task_create` description so tasks carry an `Acceptance` section (the parser
already exists in `crates/hq-memory/src/context_task.rs`). Accept: this finds
nothing outside this plan (it finds nine sites today):
`grep -rniE 'HQ never (marks|sets|completes)|never mark(s|ed)? (the task |their work |it )?(complete|done)|never `?complete`?[;:.,]|never completed' crates docs --include=*.rs --include=*.md`.

**WS7. Rollout.** Shadow on the live instance (E1 on everything, E2 on a bounded
sample to price the rest), a plan-only sweep over the existing backlog and one
apply, then `enforce` for tier 0, then tier 1 once calibration passes, then
tier 2. Accept: each step gated on the WS5 numbers, not on elapsed time.

**WS8. Sweeps and on-demand verification.** Persisted sweeps with plan, run,
pause, cancel, apply and undo; the bucket policy; the sweep id on every change;
priority order; the confirm threshold; the owner-only REST endpoints. Accept: a
plan changes no status and makes no model call; a sweep survives a daemon restart
mid-run; undo restores exactly the tasks still in the status the sweep set;
apply above the threshold is refused without confirmation; an MCP key of any
scope is refused on every sweep and audit endpoint.

**WS9. Briefing, queue health and bottlenecks.** The briefing builder, queue
health, `waiting_for`, deferral states, the bulk-policy offer past
`owner_queue_alert`, the aging alert, and the notification items. Accept: a seeded
weekend (200 completions in 48 hours) yields a briefing that lists no pass
individually and a queue health whose `waiting_for` counts sum to the backlog;
budget exhaustion keeps E1 verification running and marks the rest deferred with
the reason; one alert fires when the owner-waiting set ages past the threshold.

**WS10. Web and mobile surfaces.** Everything under "Surfaces". Accept: component
tests in the style of `BulkActionBar.test.ts` and `TaskActivity.test.tsx`; each
flow works at a 390 px width in a browser run; the web build passes
(`cd apps/hq-web && bun install && bun run build`); Verify now, Audit more and Undo
are each reachable in two taps from the tasks page.

Order: WS0 first, because it removes failure modes this plan would otherwise
inherit. Then WS1, WS2, WS3 in sequence; then WS4 and WS8, which share the verdict
and apply path; then WS5, WS6, WS9 and WS10 in parallel. WS10 can start against
stubbed endpoints once WS8 fixes the contract. WS7 is last, and ends by running
the gate over this plan's own tasks.

## Decisions (resolved 2026-10-22)

1. HQ completes verified tasks. The "never completes" claims change together.
2. Tier 2: two independent verifiers, E1, and a 24 hour veto window.
3. Audit: 5% and at least 10 a week pushed, and as many pulled on demand as the
   owner wants.
4. Tasks with no discoverable artifact return to `to_do` with a note, are never
   auto-completed, and are handled through a sweep.
5. Reviewer cost: no ceiling is set yet. E2 runs stay off until one is, and E1
   verification runs meanwhile. Shadow mode prices E2 on a bounded sample first.

## Unknowns to resolve first

- How many of the waiting tasks are code with a discoverable pull request, and
  how many are vault-only or have nothing. This sets how much of the queue can
  reach a machine level at all.
- Which repos have required check runs and branch protection (readable through
  the API). E1b is only as strong as that.
- Whether a worker's model is knowable. A lease records harness, host and
  branch but not model, so `different_model` independence needs the optional
  `model` field and the MCP `clientInfo` item in `TECHDEBT.md`.
