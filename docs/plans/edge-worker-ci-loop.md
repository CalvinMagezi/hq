# Edge worker: CI waits without a turn

The edge worker (`docs/plans/edge-coding-worker.md`) treats CI as its test runner. Until now it
waited for CI inside its own chat turn: the `edge-coding-worker` skill told it to read `run_list`,
look again a few minutes later, and loop until the run finished. That works, but the wait is the
expensive part of the job, and it is spent in the worst place.

## The problem

A CI run takes from two to twenty minutes. While the worker waits inside its turn:

- **The queue stops.** `edge-worker-dispatch` runs one task at a time and counts a pending reply
  as busy, so every other `edge-task` waits behind someone else's CI.
- **Each look costs a model call.** Every poll is another round trip carrying the whole
  conversation so far, which costs tokens and, on a phone, mobile data.
- **A long turn is fragile.** Android stops background work, and a restart fails the pending
  ask, so a task can be parked after its pull request was already open and CI was nearly done.

None of the waiting needs a model. Reading check runs on a commit is one GET, and HQ already has
the code for it (`collect_checks`, used by task verification).

## The idea

Borrowed from the Infinity runtime (Hydro project, Apache 2.0): between turns an agent is only its
conversation, and an agent waiting on a slow tool should cost nothing until the result arrives.
HQ already keeps the conversation (the ask's chat thread), so the change is to stop holding a turn
open while waiting:

1. After `github_write` pushes, the worker calls `edge_ci_wait`, which records the pushed commit
   and releases its lease. Then the worker ends its turn.
2. On each `edge-worker-dispatch` tick the daemon reads the checks on that commit, with no model
   involved.
3. When CI passes, the daemon hands the task over: `ready_for_review`, with a comment that
   links the pull request. No model call is needed.
4. When CI fails, the daemon opens a new turn **in the worker's own thread**, saying which round
   this is and which checks failed. The worker reads the log, fixes, pushes and waits again.

## Steps

| Step | What | Status |
|------|------|--------|
| 1 | `edge_ci_wait`, the `edge_ci_waits` table, and the dispatcher's resolver: hands over on green, resumes the thread on red, caps rounds and wait time; skill updated | done |
| 2 | The daemon fetches the failing job's log and passes a bounded excerpt, marked untrusted, into the resumed turn, saving the worker two or three tool calls per round | done |
| 3 | Resume from the `hq/` branch: `github_clone` takes `ref`, so a checkout Android cleared between rounds is fetched at the branch, not the default branch, which would drop the earlier rounds' edits from the next push | done |
| 4 | Keep the queue moving while CI runs: start a task on another repository while one waits, at most one per repository (two tasks on one repository share a checkout) | done |
| 5 | Wake on a webhook (`check_suite`) instead of polling, only if the measured cost of polling turns out to matter | only if measured |

## Step 1 in detail

### `edge_ci_wait`

Arguments: `lease` (the token `task_claim` returned), `repo`, `branch`, and an optional
`pull_request` URL. It refuses unless all of these hold:

- the caller is not on a restricted key (the handoff or tasks key, or a claude.ai connector
  grant, the same rule as `refuse_queue_write`);
- the lease is live, and its task carries `edge-task` and is in progress;
- the task was started by the dispatcher (an `edge-worker:<task id>:` ask exists), because that
  ask's thread is where a failed round resumes;
- `repo` is in `github_write.repos`, and `github_write` has pushed to `hq/<branch>` there.

The commit it waits on is the newest one in `github_write`'s own `pushed.json` record, which file
tools cannot touch, so the worker cannot point the wait at a commit it did not push. A pull request
URL that is not `https://github.com/<repo>/pull/<n>` is dropped, never echoed back.

A wait recorded for a task that already has one open replaces it (the worker pushed again before
CI finished). The round is the number of earlier waits for the task plus one.

### The resolver

On each tick, before anything else. A wait whose own task still has a reply running is skipped (that reply may still be pushing). Everything that needs no model is settled even while another task's reply runs; a failure resumes its worker only when no worker reply runs, so there is still at most one at a time.

| What the checks say | What happens |
|---|---|
| still running, or nothing reported yet | keep waiting |
| nothing reported after 20 minutes | hand over, saying no CI ran, so the reviewer knows nothing was tested |
| all passed | hand over: `ready_for_review`, a comment with the commit and the pull request |
| a check failed, round below 4 | resume the worker's thread with the round, the commit and the failing checks' names |
| a check failed on round 4 | park the task (blocked, `edge-task-parked`, inbox note) with the pull request and the failing checks |
| not finished after 6 hours, or could not be read for 6 hours | park the task with the reason |
| the task left `in_progress`, lost its `edge-task` tag, or was deleted | close the wait; someone else has it |

A resumed turn uses the ask key `edge-worker:<task id>:ci-<commit>`, so one failure is never
answered twice, and it is marked `untrusted` like every worker ask. The text HQ writes into it is
fixed, apart from the check names, which come from the repository's own CI definitions, are
capped in length and number, and are quoted.

### Log excerpts (step 2)

When a check fails, the resolver reads the failing GitHub Actions jobs' logs (at most two; other
CI apps have no log endpoint) and puts an excerpt of each into the resumed turn: colour codes and
timestamps dropped, the lines around the first recognisable failure (15 before, 45 after), at most
4,000 characters, fenced, with any fence inside the log broken so it cannot close HQ's. The
question says the excerpts are the repository's and are data, not instructions. A log that cannot
be read is left out, and the question tells the worker to fetch it.

### Resuming from the branch (step 3)

`github_clone` takes `ref`, an `hq/` branch. A checkout on disk is reused as before; a missing one
is fetched at the branch, and only while the branch's head is a commit `github_write` recorded
pushing there, so a branch someone else moved never becomes the base of the next push. The base
record then names the branch head, and `github_write` builds the next commit on it.

### The queue while CI runs (step 4)

A task waiting on CI is neither busy nor abandoned. The dispatcher may start another task, unless
it names a repository that has a task waiting on CI: two tasks on one repository would share the
checkout that still holds the waiting task's edits.

### What it costs on a phone

A waiting task costs two small GETs per tick (check runs and commit status), every two minutes,
and no model calls. Private repositories need a read token (`GITHUB_TOKEN` or `GH_TOKEN`), as
`github_read` already does.

## How we will know it works

- Model calls and tokens per task, before and after (the `task_outcomes` ledger).
- How long the queue sits blocked on CI (the gap between a push and the next task starting).
- The share of the worker's pull requests that are green when handed over, and the share parked.
