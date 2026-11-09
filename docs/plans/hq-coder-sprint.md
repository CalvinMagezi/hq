# HQ Coder: the full cycle from chat

Status: in progress. Builds on `docs/plans/mobile-helpers.md` and `docs/plans/edge-coding-worker.md`.

## The goal

From the main chat, a person can plan against their private repositories, scaffold, hand a job to
the coder with its task list, notes, budget and model, watch it, steer it, and take the finished
pull request. A phone being killed mid-job costs a retry, not a parked task. The Helpers screens
show this work as HQ's own.

## What is wrong today

Found by reading the code. Each is fixed below.

| Problem | Where | Slice |
|---|---|---|
| A kill during a turn fails the ask, holds the queue for the 900 s lease, then parks the task until the owner unparks it | `ws/ask.rs:263`, `dispatch.rs:120` | 2 |
| One provider error, 429 or network blip ends the run; with one backend nothing retries | `session/loop.rs:293`, `dispatch.rs:141` | 2 |
| CI is waited on inside a turn, the likeliest place to be killed; `watch_create` cannot work there | `background_turns.rs:49` | 2 |
| Child spend is not added to the task's budget, and each new ask gets a fresh budget | `service.rs:685`, `ask.rs:142` | 1 |
| The fix limit is only a sentence in the skill | skill step 6 | 2 |
| A retry that refreshes can push a commit that drops earlier edits | `write.rs:388`, `mod.rs:629` | 2 |
| Adding a link to a queued task changes `updated_at`, so the same task can run again without limit | `dispatch.rs:249`, `links.rs:17` | 1 |
| Up to two minutes before a job starts; the worker's prompt carries the owner's whole system prompt | `daemon/mod.rs:448`, `builder/prompt.rs:94` | 3 |
| Chat cannot start, stop or read a run as one thing; budget and model are global | `dispatch.rs:151` | 1, 4 |
| On a phone, `github_read` refuses pull requests, issues, diffs, releases and search | `github/mod.rs:336` | 5 |
| A run's card is one line that opens the chat: no steps, cost, PR, CI or controls | `HelperWords.kt:384`, `helpers.tsx:368` | 6 |

## The one new thing: a job

A job is a task plus one row in a new `coder_jobs` table: repository, budget, credit cap, model,
phase, attempt counts, pull request, head commit, next check time, spend so far, stop reason, and
the chat thread that asked. Tasks stay the source of truth for the work; the row holds only what
the worker must not lose. Everything else reads it.

Phases: `queued`, `working`, `waiting_ci`, `waiting_owner`, `done`, `stopped`.

## Slices

One branch, one focused commit per step, one pull request at the end.

### 1. Job and money

- `coder_jobs` table; dispatch keys attempts on the job, not `updated_at`.
- Budget belongs to the job, across attempts and children: each ask gets what remains, and child
  cost rolls up into the parent's total.
- Per-job budget and model. The model is checked against enabled backends. A new owner-only
  `github_write.task_budget_max_usd` (default $5) is the ceiling, and replaces
  `session_cap_usd` for worker runs only.
- Running out parks at once with the reason "budget", closing the `TECHDEBT.md` row.
- Copilot: dollars already apply as an estimate. Add an optional credit cap that compares the
  account-wide counter with its value when the job started. Other Copilot use counts against it,
  so it can stop early but cannot overspend because of other use.

### 2. Surviving a kill (taken from `pi/packages/durable`)

- **A kill is a counted attempt.** On restart, end the dead lease and requeue, up to three tries.
  A new checkpoint resets the count. No wait for the lease, no owner needed.
- **Transient errors** back off and retry in the loop, then requeue with the same limit.
- **Checkpoints.** The skill records one after each push and fix; a new attempt reads it first.
- **Refresh only when nothing was pushed.** Otherwise keep the checkout, since `github_write`
  publishes the whole checkout against its base.
- **CI waits without a model.** `github_read` action `wait_ci` polls inside the tool, so the wait
  costs no model calls. A kill during it is one more counted attempt, and the checkpoint resumes at
  CI. A separate stored wait in the daemon was the first idea; this does the same job with far less.

Not taken from `pi`, because they solve problems one task on a phone does not have: group commit,
per-tool durable tasks, the code-mode journal, the task graph, partial-change encoding and
persistent background sub-agents.

### 3. Faster and cheaper

- Dispatch runs at once on Start and when an ask settles; the tick stays as the floor.
- A worker prompt of its own: no soul, memory or skill catalog, and stable so it caches.
- Drop skill steps the tools already do: `task_get` after claim, a comment beside release, the
  immediate `run_list`.
- Baseline to beat: 24 model calls and $0.0157 for a one-file job.

### 4. One chat tool: `hq_coder`

Actions: `start`, `status`, `list`, `steer`, `stop`, `retry`, `feedback`, `budget`.

- `start` takes a repository, a job or existing task ids, vault notes, budget, model, and an
  optional branch to continue. It links the notes and starts now. A task list runs in order, one at
  a time, using the existing `blocked_by`.
- `steer` is a comment returned by the next `task_heartbeat`, so it lands at the worker's next
  milestone. Mid-call interruption is not worth the machinery.
- `stop` cancels the running turn. How is to be confirmed in the slice; no ask cancel exists yet.
- When a job finishes or stops, the chat that started it gets a message.
- The worker gains `context_packet`, limited to notes linked to its task.

### 5. GitHub from chat

- `github_read` answers every action over HTTPS with App tokens, so private repositories work
  without `gh`.
- Chat keeps `github_clone` and `github_write` behind owner approval, as today.
- Scaffolding: GitHub needs the repository to exist with one commit, and an App cannot be assumed
  to create one under a personal account. So HQ gives the person the new-repository link and the
  App's repository picker, then scaffolds on an `hq/` branch, then `hq_coder start` continues that
  branch.
- The `.github/` block stays. HQ writes the workflow file and hands it to the person to add.

### 6. Helpers

- `GET /api/coder/jobs` and a job detail route, used by phone and web. Reuses the existing poll.
- A card that reads as HQ's own, listed first: repository, phase in plain words, spend against
  budget, backend (Copilot or OpenRouter), attempt when above one.
- A job page: a timeline from checkpoints and comments, the pull request and CI, cost, and Send a
  note, Stop, Retry, Send back, Raise budget, Open the chat.
- Parked jobs say why in words and offer Retry.

### 7. Fewer permission requests

- Answered requests leave the chat, on the phone and the web, replaced by one quiet line.
- A fetch request offers Allow this site. Three approved fetches of one site in a chat allow that
  site for the rest of the chat, and clear its waiting requests.
- A site allowed in three different chats stops asking in new chats, and is listed in Settings
  where the owner can remove it. Only the owner's approvals count; the model cannot add one.
- Trust is per exact host, so one person's page on a shared host does not cover the others. A very
  long address still asks: that check is what stops notes leaving inside a URL.

## Security

- `hq_coder` is for the owner's chat only: never MCP, connector or a scoped key. `start`, `retry`,
  `feedback` and raising a budget need the owner's approval.
- The first-task yes stays the owner's: an Allow button in the Inbox, which chat cannot press.
- A budget set from chat cannot pass the owner-only ceiling.
- Task comments are untrusted input to the worker, as task text already is.
- **Vault into a repository is a new leak path.** The worker reads untrusted repository text and
  can publish. So it reads only notes the owner linked to that task, and linked notes on a public
  repository need explicit approval.
- Chat cannot merge. No one but the person merges.

## Done means

- A killed phone resumes the job with no owner action (real `pkill` on the A32 at each phase).
- A job started from chat, on Copilot and on OpenRouter, reaches a merged-ready pull request.
- Budget and credit caps stop a job and say so; child spend counts.
- Every `github_read` action works on a private repository on a phone.
- Phone and web show the job, its controls work, and a browser check covers the web.
- Gates: fmt, clippy, workspace tests with totals, web typecheck, test and build, Android tests and
  lint.

## Decisions

- CI for a scaffolded repository: the `.github/` block stays. HQ writes the workflow file and hands
  it to the person to add.
- HQ's own card is marked with a thin left rule in `BrandIndigo`.
