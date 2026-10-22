# Task verification

Agents finish tasks faster than one person can read what they did. Without this, finished work piles up in
`ready_for_review`, every item asks for your attention, and a task that is wrongly marked done starts
dependent work on something that is not.

Task verification takes you out of reading each result. When an agent hands work over, HQ checks it against
evidence the agent could not have written (a merged pull request with green required checks, a note's content
hash, a page that answers), completes what passes, sends back what does not with the reasons, and asks you only
about exceptions. It also measures how often it is wrong, so you can decide how far to trust it without reading
the results.

It is off by default. A fresh install behaves exactly as before: a task reaches `complete` when you or an agent
sets it. Design and reasoning: `docs/plans/task-verification.md`.

## How a task moves

```
in_progress --release / update--> ready_for_review     ("verification queued", nothing more)
                                         |
                    verify-dispatch looks at the evidence
                                         |
   pass ---------------> complete        (done by a verifier, never by the worker)
   fail / nothing found -> to_do         (findings become the checkpoint; the worker keeps it)
   can't decide yet ----> stays          (waiting_for says on whom: ci, verifier, budget)
   3 returns ------------> blocked       (one item for you, with the findings)
   tier 2 pass ----------> stays         (completes after a veto window unless you veto it)
```

- An agent that asks for `complete` is not refused. The call becomes `ready_for_review` and the reply says
  verification is queued, so existing agents and skills keep working.
- You can still complete any task yourself from the board. That is recorded as a verdict of level `owner`.
- A task that comes back arrives with `resume`: the findings, in a checkpoint, from `task_next`. After
  `max_attempts` returns (3) it goes to you once instead of bouncing again.
- `task_get` shows `waiting_for`: `worker`, `ci`, `verifier`, `budget` or `owner`. Only `owner` is waiting on you.
- Turning enforcement on does not bulk-process old work. Tasks that entered review before enforcement began are the
  legacy backlog: the live queue leaves them alone, they say so in `waiting_for`, and a sweep (below) handles them.

## Clearing a queue: sweeps

A sweep is one decision about a whole backlog instead of a review of each task. It works in phases the daemon
advances a few tasks a minute, so it survives a restart and no request waits on GitHub or a model.

1. **Plan.** HQ reads each task's evidence with no model and no spend, and sorts every task into a group with an
   estimate of the reviewer runs it would need. Nothing moves.
2. **Run.** You start it. Verdicts are recorded in priority order (tasks others wait on, then tier 2, then oldest),
   with the reviewer where one is needed and the budget allows. Nothing moves yet. Pause and cancel work between tasks.
3. **Read the groups, not the tasks.**

| Group | Meaning | What apply does |
|---|---|---|
| Passed | Evidence met the tier's bar | Completes it, with the evidence level recorded |
| Failed | A criterion is unmet | Back to `to_do` with the findings as the checkpoint |
| Nothing to check | No pull request, commit or note HQ can find | Back to `to_do` with what to link; not completed |
| Could not decide | Checks running, a reviewer unsure, budget spent | Left where it is and retried; shown after the retries |
| Need you | Tier 2 (opens its veto window), held passes, three failures | Stays in your list |

4. **Apply.** One confirmation applies the groups. Above `sweep_confirm_over` tasks (25) apply always needs your
   explicit confirmation, whatever the mode. Each task and the pull request or note behind its verdict is read again
   first: work that changed since is left alone and reported, not completed on an old verdict. Every change carries the
   sweep id (the actor is `verify:sweep-<id>`).

**Undo** puts back every task the sweep moved that has not changed since, and reports the ones it left alone. A
restored task is not verified automatically until it is submitted again, so the undo is not simply redone. It cannot
unsend the notices a completion already sent to the agents waiting on it.

A sweep can be applied only while `mode` is `enforce`; in `shadow` it can be planned and run to see the numbers.

## The briefing and queue health

`hq verify briefing` (and the card at the top of the tasks page and on the home screen) is exception first: what needs
you; then what HQ verified and completed, one line per initiative and never a pass at a time; what failed and was sent
back without you; tier 2 tasks inside their veto window; and the audit prompt. Past `owner_queue_alert` items waiting
on you it stops listing and offers one decision for the biggest group ("40 of 47 were submitted before verification was
enforced: plan a sweep"), because a long flat list is the bottleneck this exists to remove.

`hq verify queue` shows the backlog by who it waits on, the oldest item waiting on you, the last 24 hours, how long
the queue behind the verifier takes to clear at that rate, what is deferred and why, and reviewer spend against the
ceiling. When the budget runs out, machine checks keep running and the tasks that need a reviewer wait, with the
reason shown. A daemon task raises at most one alert a day when more than `owner_queue_alert` items wait on you, the
oldest has waited `owner_queue_age_days`, the queue would take over 24 hours to clear, or the budget is spent with
tasks waiting for it. Set `briefing_hour_utc` to also have the briefing posted once a day.

## What counts as evidence

HQ reads these itself, and a source it cannot read is recorded as unavailable and never as support.

| Evidence | What it shows |
|---|---|
| Pull request state and check runs, required checks from branch protection | The change merged, and the checks that guard it passed on that commit |
| Diff scan | A test skipped or deleted, a lint or CI step weakened, in the same change |
| Note content hash | The note a task says it wrote exists and has not changed since it was checked |
| URL probe | A page answers (https only, public addresses, no redirects) |
| Reviewer (a model, optional) | Judges each acceptance criterion against the recorded facts above, citing them by id |

The task's acceptance criteria come from an `Acceptance` section in its description. A task with none is judged
on its title alone and, for code, is sent back: put one checkable line per thing that must be true.

A reviewer cannot pass anything the facts do not show. A criterion that cites nothing, or a fact that was never
recorded, makes the verdict inconclusive. A verdict is dropped if the pull request gets a new commit, the note
is edited, or the task is submitted again while it is being checked.

HQ finds the artifact from the task's links (`pr`, `commit`, produced notes), then from the branch the worker's
lease recorded. At the moment of handover, a code task with nothing HQ can find gets a reply naming exactly what
to link (`release_check: warn`, the default) or, with `release_check: enforce`, is refused until it is linked.

## Risk tiers

The tier is the highest of: an explicit tag, the paths the change touches, and the kind of link.

| Tier | Typical work | To auto-complete |
|---|---|---|
| 0 | Notes, research, planning. Vault-only. | One independent review with every criterion cited. Level `peer`. |
| 1 | Code with a pull request. | Pull request and required-check evidence plus a review that includes the diff scan. Level `machine_peer`. |
| 2 | Tags in `tier2_tags` (security, payments, production, deploy, migration, outbound, deletion), paths in `tier2_paths`, or a change that weakens tests | Same, with reviewers from two different model families, and then a veto window |

Tier 2 is approved when it passes and completes `veto_hours` later (24 by default) unless you veto it. During the
window it is in your list as a single item; dependents are not told it is done until it completes. If the pull
request or note changed in the window, it is not completed on the old verdict: it is checked again.

Verification is not a guard for irreversible actions. A deploy, a payment or a deletion needs its approval before
it happens. This runs after the fact.

## Turning it on

Roll it out in steps and let the numbers, not the calendar, decide when to move to the next. `hq verify rollout`
(and the Turning it on section of the queue panel on the tasks page) says where you are, what the next step waits
for, and the one setting that takes it. It changes nothing itself.

1. **Set a reviewer budget first** (`reviewer_monthly_ceiling_usd`). Every tier needs a review to pass, so without
   one a shadow verdict can fail a task but never pass it, and the numbers below would say nothing about false
   passes. It also lets the canaries run: planted defects and clean controls put to the reviewer, which is how its
   judgement is tested.
2. **Shadow.** `tasks.verification.mode: shadow`. Every task in review gets a verdict that moves nothing;
   `shadow_e2_sample` bounds how many reviewer runs it spends to price the rest.
3. **Let shadow compare itself.** Shadow completes nothing, so there are no completed tasks to audit yet. What there
   is: your own decisions. For each verdict, `rollout` looks at the first thing that happened to that submission
   afterwards, if someone other than its worker (or HQ) did it. Accepted work the verifier would have passed is an
   agreement; work you sent back that it would have passed is a **false pass**, the dangerous kind; work you
   accepted that it would have held back is a false reject. A move with no recorded person says nothing and is not
   counted. Keep reviewing the way you do now and the numbers build up.
4. **Read the backlog once.** `hq verify sweep plan` reads everything waiting without a model and without moving
   anything. Run and apply it when the groups look right (see Clearing a queue).
5. **Enforce a tier when `hq verify rollout` says every condition is met.** For tier 0 that is: enough verdicts
   compared (`autonomy_min_audits`, 30 by default), their false passes rare enough
   (`autonomy_max_disagreement`, 10% as an upper bound at 95% confidence), the reviewer catching planted defects
   (`autonomy_min_catch_rate`), and the backlog read. Set `mode: enforce` with `enforce_tiers: [0]`. Tier 1 then
   waits on tier 0 having earned its autonomy from your audits (`hq verify pull 20`, `hq verify agree <id>` or
   `hq verify disagree <id> --note "..."`), and tier 2 on tier 1.
6. **Watch `hq verify status`.** It says, per tier, whether auto-completion is earned, has too little data
   (with the counts), or has been withdrawn and why. `rollout` repeats a withdrawn tier as a warning.
7. **Last, run the gate over its own plan.** `hq verify sweep plan --initiative <the verification initiative>`,
   run, apply, so the tasks that built this are completed the way every other task now is.

```yaml
tasks:
  verification:
    mode: shadow            # off | shadow | enforce
    enforce_tiers: [0, 1, 2]
    reviewer_monthly_ceiling_usd: 20   # unset: no model is ever called
```

### Settings

| Key | Default | Meaning |
|---|---|---|
| `mode` | `off` | `off` changes nothing, `shadow` records verdicts that move nothing, `enforce` gates completion |
| `enforce_tiers` | `[0, 1, 2]` | Tiers gated when enforcing; the others behave as before |
| `max_attempts` | `3` | Returns to the worker before the task goes to you |
| `release_check` | `warn` | At handover, a code task with nothing findable: `off`, `warn` or `enforce` |
| `tier2_tags`, `tier2_paths` | see above | What raises a task to tier 2 |
| `veto_hours` | `24` | Tier 2 veto window; `0` completes tier 2 at once |
| `concurrency` | `2` | Verifications at once |
| `sweep_confirm_over` | `25` | A sweep that moves more tasks than this needs your explicit confirmation to apply |
| `owner_queue_alert` | `20` | Items waiting on you past which the briefing offers one group decision, and an alert is raised |
| `owner_queue_age_days` | `3` | Days the oldest item waiting on you may age before an alert |
| `briefing_hour_utc` | unset | UTC hour to post the briefing once a day; unset builds it on demand only |
| `reviewer_monthly_ceiling_usd` | unset | Spend allowed on reviewer calls this month; unset means no model |
| `reviewer_models` | `premium` alias | Reviewers; tier 2 needs two different families |
| `reviewer_timeout_secs` | `120` | One reviewer call |
| `shadow_e2_sample` | `50` | Reviews run in shadow mode to price the rest |
| `peer_only_code` | `false` | Let code pass on a review alone when its pull request has no checks. Off: code needs machine evidence |
| `audit_rate`, `audit_min_per_week` | `0.05`, `10` | The weekly audit floor pushed to you |
| `canary_rate` | `0.1` | Share of reviewer runs followed by a canary |
| `autonomy_gate` | `demote` | `off`, `demote` or `require`, below |
| `autonomy_min_audits` | `30` | Audits a tier needs before its error rate means anything |
| `autonomy_min_canaries` | `20` | Planted defects a verifier needs before its catch rate means anything |
| `autonomy_max_disagreement` | `0.10` | Upper bound on audits you disagreed with |
| `autonomy_min_catch_rate` | `0.90` | Share of planted defects the reviewer must catch |

## Can you trust it

HQ measures itself three ways.

- **Canaries.** After reviewer runs, HQ asks the real reviewer to judge made-up work: a planted defect (a
  failing check, an unmet criterion, a worker's claim with no evidence, an instruction hidden in the task text) or a
  clean control. A reviewer that approves everything shows a low catch rate; one that rejects everything shows false
  rejects. They run at an unpredictable rate and respect the monthly ceiling.
- **Audits.** A random sample of what HQ completed, favouring the tiers and verifier configurations with the least
  data, arrives as a weekly digest (the floor), and you can pull more. A disagreement reopens the task and counts
  against the verifier configuration that made the call.
- **Escapes.** A completed task that is reopened, or that gets a follow-up tagged `regression` linked to it,
  counts against the tier.

From those, each tier is **earned** (enough audits and canaries, and they are good), **insufficient** (the report
gives the counts and no invented rate), or **withdrawn** (the numbers say it cannot be trusted). The gate decides
what that does:

- `demote` (default): a withdrawn tier stops completing on its own. Its verdicts are kept, the task waits for
  you, and one alert names the numbers that did it. A tier with too little data keeps going, because you
  enabled it.
- `require`: a tier completes on its own only once its calibration is earned.
- `off`: calibration is reported and holds nothing back.

Changing the reviewer models starts a new record, because trust earned by one reviewer says nothing about another.

## Commands

```bash
hq verify               # mode, who tasks are waiting on, calibration per tier
hq verify rollout       # where turning it on stands, what the next step waits for, the setting that takes it
hq verify audits        # completed tasks waiting for your agree or not
hq verify pull 20       # draw more audits now
hq verify agree <id>
hq verify disagree <id> --note "the test is empty"
hq verify holds         # tier 2 tasks inside their veto window
hq verify veto <task> --note "needs a backfill"
hq verify briefing 72   # what happened in the last 72 hours, exceptions first
hq verify queue         # who tasks wait on, how old, how fast it clears, spend
hq verify sweep plan [--initiative <id> | --since 2026-10-01]
hq verify sweep show <sweep-id>   # groups, estimate, progress
hq verify sweep run|pause|cancel <sweep-id>
hq verify sweep apply <sweep-id> [--confirm]
hq verify sweep undo <sweep-id>
```

The same things are on the tasks page, which is built for a phone: a verification bar (who tasks wait on, **Verify
now** with a scope of everything waiting, this initiative, since a date or the tasks you picked, **Audit**, and queue
health), the briefing card, a sweep sheet with live progress, a stack of audit cards (Agree, Disagree, Skip, **Audit 10
more**, and the running error estimate with its sample size), and a Verification section in each task with the verdict
history and evidence, **Second opinion**, **Complete anyway** and **Return to to_do**. A **second opinion** asks another
model (the first verdict's models are left out when others are configured) to judge the task again; one that disagrees
counts as an audit datum against the first verdict and puts the task in front of you. It never changes the task.

These read and write this machine's database. They are deliberately not agent tools: an agent must not be able to
agree with an audit of its own work. The daemon tasks are `verify-dispatch` (every minute, only when `mode` is not
`off`) and `verify-audit` (every six hours, queues the weekly floor).

## Limits

- It judges what it can read. A task whose result lives somewhere HQ cannot read is not passed; it goes back with
  what to link.
- A criterion no fact can show ("the page looks good") stays with a reviewer, who can only say pass, fail or
  unclear, and is checked by canaries rather than trusted.
- Sweeps and the other owner actions are for the owner only: the web credential, or the `hq` command on the machine.
  No agent key, of any scope, can start, apply or undo a sweep, answer an audit, veto a task or ask for a second opinion.
  The `hq verify` commands open the database directly, so a process that can run `hq` as you on your machine can use
  them, as it could open the database itself. Coding agents run in the host's sandbox, which keeps HQ's own folder out
  of their reach; an agent you start outside it is as trusted as you are.
- The tier and the acceptance criteria are read from the task when it is verified. A worker can edit its own task's
  tags and `Acceptance` section before handing it over, and nothing records what they were before. After the
  verdict that is covered: a task whose criteria were edited or whose tier rose since it was checked is not
  completed on the old verdict, whether it waits in a sweep or a veto window. Before it, the audits are what catch
  it, and the task's event log shows who changed what.
- Only the first three pull requests and five notes a task links are checked, and the rest are not looked at. Link
  what the task produced and nothing else.
- Several processes can share one database. The daemon writes the completion policy from its configuration on every
  verification pass, and another process (the MCP server, a chat session) does so only when it really read a
  configuration file, so one started under another user with no config cannot switch verification off. A policy that
  cannot be read at all is treated as enforcing.
- The daemon does the slow work, so a sweep moves only while the daemon runs. `hq verify sweep show` says where it is.
  The queue health panel says when tasks have waited for verification with nothing verified for a day, which usually
  means the daemon is not running, and when they wait only because no reviewer budget is set.
- Turning it on for a real instance is a decision to make in steps (see "Turning it on"), reading `hq verify status`
  between them. Nothing in HQ does that for you.
