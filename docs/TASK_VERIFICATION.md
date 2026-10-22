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

Roll it out in steps and let the numbers, not the calendar, decide when to move to the next.

1. **Shadow.** `tasks.verification.mode: shadow`. Every task in review gets a verdict that moves nothing. Compare
   with what you and the agents then did. Reviewer (model) runs stay off until you set a ceiling, so shadow costs
   nothing but machine reads; `shadow_e2_sample` bounds how many reviews run to price the rest.
2. **Set a reviewer budget** if you want a model to judge criteria: `reviewer_monthly_ceiling_usd`. Without one,
   only deterministic checks run, which already settle a failing check, a missing artifact and an unread source.
3. **Audit.** `hq verify pull 20` draws completed work for you to check. `hq verify agree <id>` or
   `hq verify disagree <id> --note "..."`.
4. **Enforce a tier.** `mode: enforce` with `enforce_tiers: [0]`, then `[0, 1]`, then `[0, 1, 2]`.
5. **Watch `hq verify status`.** It says, per tier, whether auto-completion is earned, has too little data
   (with the counts), or has been withdrawn and why.

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
hq verify audits        # completed tasks waiting for your agree or not
hq verify pull 20       # draw more audits now
hq verify agree <id>
hq verify disagree <id> --note "the test is empty"
hq verify holds         # tier 2 tasks inside their veto window
hq verify veto <task> --note "needs a backfill"
```

These read and write this machine's database. They are deliberately not agent tools: an agent must not be able to
agree with an audit of its own work. The daemon tasks are `verify-dispatch` (every minute, only when `mode` is not
`off`) and `verify-audit` (every six hours, queues the weekly floor).

## Limits

- It judges what it can read. A task whose result lives somewhere HQ cannot read is not passed; it goes back with
  what to link.
- A criterion no fact can show ("the page looks good") stays with a reviewer, who can only say pass, fail or
  unclear, and is checked by canaries rather than trusted.
- Not built yet: on-demand sweeps over an existing backlog with a plan and undo, the Monday briefing and queue
  health view, and the web and phone screens for audits, holds and the exceptions list. Today those are the
  `hq verify` commands and the web inbox items.
