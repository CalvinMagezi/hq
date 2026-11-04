# Handover: CI waits on the phone, and durable waits (item 3)

For the agent that takes this over on a machine with the phone at hand. It covers what was built,
the phone tests nobody could run in a cloud container, and the next build (item 3). Read
`CLAUDE.md` first; its gates and rules apply to everything here.

## Where things stand

Three commits, all in one pull request from the `ccr-...` branch:

| Commit | What |
|---|---|
| `feat(edge-worker): wait on CI without holding a turn open` | `edge_ci_wait`, table `edge_ci_waits` (migration 109), the resolver inside `edge-worker-dispatch` |
| `feat(edge-worker): log excerpts, resume from the hq/ branch, queue moves while CI runs` | steps 2 to 4 of the plan |
| `feat(sessions): a timed-out wait names how the caller can stop waiting` | item 1: `instead` in a timed-out `harness_session_wait`, the `hq-fleet` skill (plugin 0.2.4) |

The design and its reasons are in `docs/plans/edge-worker-ci-loop.md`. The code:

- `crates/hq-tools/src/github/ci_wait.rs`: the tool, the resolver, the log excerpt; tests in
  `ci_wait_tests.rs`
- `crates/hq-tools/src/github/dispatch.rs`: the queue, `named_repos`, the `Ask` helper
- `crates/hq-db/src/edge_ci_waits.rs` and `crates/hq-db/sql/109_edge_ci_waits.sql`
- `crates/hq-tools/src/github/mod.rs` (`job_log_text`, `github_clone`'s `ref`),
  `tarball.rs` (`fetch_at`), `write.rs` (`pushed_to`, `newest_pushed`)
- `crates/hq-tools/assets/skills/edge-coding-worker/SKILL.md`: steps 5 to 7
- `crates/hq-tools/src/harness_session/control.rs` (`Waiter`, `not_settled`) and `tools.rs`
  (`waiter`)

What was verified in the cloud container: clippy on the workspace, and the test suites of
`hq-db`, `hq-tools`, `hq-mcp`, `hq-agent` and `hq-cli`, crate by crate. Two tests in
`crates/hq-tools/src/coding/edit.rs` failed there because the container ran as root (they make a
file read-only to force a write error). They have nothing to do with this work.

Not verified anywhere yet: a full `cargo test --workspace` in one run, and any of the behaviour
below on a real phone against real GitHub.

## 1. Before the phone

1. Pull `main` once the pull request is merged. Run `./scripts/install-hooks.sh`.
2. Run the gates as a normal user: `cargo clippy --workspace --all-targets -- -D warnings` and
   `cargo test --workspace`. Check the totals, not just the exit code. The two `coding::edit`
   tests should pass as a non-root user; if they fail, look into that separately.
3. Get the phone onto a build that has these commits: the signed APK from a release build of
   `main` (`docs/HQ_MOBILE.md`, "Installing the APK without the Play Store"; turn on "Follow every
   build of main" if the stable channel lags). Do not side-load a local build.

## 2. Set up the test

- **A scratch repository** you own, `<owner>/<repo>`, with no Actions secrets, and a CI workflow
  that is fast and deterministic. Give it a check the first attempt fails and the second passes,
  so the red path is certain. For example, CI runs
  `test -f DONE.md && grep -qx ok DONE.md || { echo "error: DONE.md must exist and hold the line ok"; exit 1; }`
  and the task asks for something unrelated, such as adding a `NOTES.md`. The first push fails
  with a readable error, and the fix is obvious from the log.
- **A second scratch repository** with the same CI, for the queue test.
- **On the phone**, in HQ's config: `github_write.repos` lists both repositories,
  `github_write.dispatch_tasks: true`, and `github_write.dispatch_model` names a backend you have.
  Under Settings, "Keys on this phone": `HQ_GITHUB_WRITE_TOKEN` (fine-grained, the two
  repositories only, Contents and Pull requests read and write, Actions read). Add a read token
  (`GITHUB_TOKEN`) if the repositories are private: the daemon needs it to read check runs and
  job logs.
- Confirm `github_clone` makes **tarball** checkouts on the phone (its reply says
  `"via": "tarball"`). `github_write` works only with those.

Where to look while it runs:

- the task's comments and status in the app;
- the worker's chat thread (the task links it, and so does Helpers on the phone);
- the database, read-only: `SELECT id, task_id, sha, round, state, detail, created_at FROM edge_ci_waits ORDER BY id;`
  and `SELECT external_id, status, error FROM hq_asks WHERE external_id LIKE 'edge-worker:%' ORDER BY created_at;`
- the owner's inbox, for parked tasks.

## 3. The phone tests

Record each result (pass, fail, what you saw) in a `## Results` section at the end of
`docs/plans/edge-worker-ci-loop.md`, with dates and no personal details.

| # | Do | Expect |
|---|---|---|
| A | Queue an `edge-task` on repository 1 | The worker claims it, pushes an `hq/` branch, calls `edge_ci_wait` and its turn ends. A task comment says "Waiting on CI for <sha> ... (round 1 of 4)". No more model calls while CI runs. |
| B | Let CI fail (the `DONE.md` check) | Within a tick or two (the dispatcher runs every 2 minutes) the same chat thread gets a message: round 1, the failing check's name, and a fenced log excerpt holding the `DONE.md` error. The worker fixes it, pushes to the same branch and hands CI over again (round 2). |
| C | Let CI pass | The task moves to `ready_for_review` with a comment naming the commit and the pull request, and the PR appears in the task's links. No model call is made for this step. |
| D | With task 1 waiting on CI, queue a task on repository 2, and another on repository 1 | The repository 2 task starts; the second repository 1 task waits until task 1's wait closes. Task 1 is never parked as dropped. |
| E | During B, after CI fails and before the worker comes back, clear the app's cache (Android settings, Storage, Clear cache) | The resumed worker calls `github_clone` with `ref: hq/<branch>`; the reply has `"ref"` and `"cached": false`. The next push keeps round 1's change: the PR diff holds both rounds. |
| F | While a task waits on CI, force-stop the app, then start it again | The wait survives (it is a database row). After the restart the next tick settles it as in B or C. If the worker's own reply was cut off mid-turn, the task is parked, which is the known behaviour. |
| G | Optional: a repository whose CI always fails | Four rounds, then the task is parked (blocked, tag `edge-task-parked`, inbox note naming the PR and the failing checks). Costs four rounds of tokens. |
| H | Optional: a repository with no workflows | After about 20 minutes the task is handed over with a comment saying nothing tested the change. |

Item 1, from Claude Code on a laptop with plugin 0.2.4 and a paired HQ:

| # | Do | Expect |
|---|---|---|
| I | Spawn a worker session, then call `harness_session_wait` with a short timeout while it works | `settled: false` with `instead.command` = `hq sessions watch --session <id> --until finished --until blocked --until exited`. Running that in the background and ending the turn brings Claude Code back when the worker finishes. |
| J | Same from HQ's web chat | `instead.tool` = `harness_session_watch`, `drive: false`. |
| K | A worker with no state rules (Hermes, say) | `state_readable: false` and the note about reading `harness_session_logs`. |

### What to measure

These numbers decide whether the CI loop worked and whether step 5 (webhooks) is worth building:

- model calls and tokens per task, from `hq usage` (the `task_outcomes` ledger). Compare against a
  task worked before these commits if one exists in the ledger, or against what the old skill
  would have spent (one call per poll);
- how long the queue sat blocked while CI ran (push time against the next task's start);
- the share of pull requests handed over green, and the share parked.

## 4. Risks to watch

- **Log excerpts are a heuristic.** The excerpt starts at the first line matching an error
  pattern (`ci_wait.rs`, `FAILURE`). On an unfamiliar toolchain it may pick the wrong spot; the
  worker can still fetch the full log. Only GitHub Actions jobs have logs; other CI apps give
  check names only.
- **Untrusted text in the worker's turn.** The check names and log excerpts are the repository's.
  They are capped, quoted or fenced, and labelled as data, and the ask is `untrusted`. Watch for a
  worker that treats them as instructions.
- **`github_clone` with `ref` was tested on the tarball path only.** The `gh` and `git` paths pass
  `--branch` and were not run.
- **A worker's reply cut off by a restart** still parks its task (the ask fails as "HQ restarted").
  That is the existing behaviour, not something these commits changed.

## 5. Item 3: durable waits

Start this after the phone tests, and only once A to C pass.

### The problem

When an agent in HQ has to wait for something it cannot see (another agent's turn, CI, a task
moving), it either holds its turn open or spends a model call on a timer. Only two waits end
without a model call today: `edge_ci_wait` and a web chat watching a session. Everywhere else:

- HQ sessions with no chat thread (relay chats, sub-agents) and harness sessions with no task
  have nothing that wakes them, so they loop on `harness_session_wait`. The new `instead` field
  can only tell them to call again.
- An observer gets one message per supervisor sweep, typed in only when the sweep finds it idle,
  and each message is its own model turn: five events cost five turns.
- The web driver keeps one wake reason per session (`registry::set_wake`), and a newer event
  overwrites an older one. A busy chat skips the wake and tries again later.
- Driver check-ins (every 30 minutes) and `watch_create` firings (every 5 minutes or more) are
  model turns on a timer, even when the thing they check can be read without a model.
- Nothing serialises input into one harness pane: a driver send, a delivered message and a REST
  send can arrive together, guarded only by an idle check.

### Why it matters

Model calls are the running cost of every agent HQ runs, and waiting is where most of them go
without doing any work. On a phone, where data is metered and Android stops background work, it
is the difference between a worker that can run for hours and one that cannot. One way to wait
also makes every new kind of agent cheaper to add.

### The shape (sketch; write the plan before building)

Generalise `edge_ci_waits` into one durable wait:

- **A wait row**: who waits (a web chat thread, an HQ chat or relay chat, a harness session's
  task thread), what it waits for (a session event, a task status, CI on a commit, a time), what
  to do when it happens (resume that thread with fixed text, or settle without a model as
  `edge_ci_wait` does on green), and limits (an expiry and a round cap).
- **One resolver** in the daemon that checks every condition HQ can read without a model, the
  way `ci_wait::resolve` does, with the supervisor's host events as the fast path.
- **Delivery that batches and queues**: events for one thread queue in order and are delivered
  together as one turn when the thread is free. This replaces the latest-wins `pm_wake` and the
  one-message-per-sweep delivery.
- **One tool**, roughly `wait_for(condition)`: the caller ends its turn and comes back when the
  condition holds. `harness_session_wait`'s `instead` then points every caller at it, including
  the two that have no route today.

Rules that bind the design:

- identity comes only from the runtime's `_hq_*` arguments, never from the model (`CLAUDE.md`,
  "Identity is never model-writable");
- chat relays carry chat only: a wait must never post a notification to Telegram or Discord;
  notices go to the owner's inbox (`value_items`);
- text from another agent or a repository is data, presented as such, and the relay depth cap
  (`MAX_RELAY_DEPTH` in `crates/hq-tools/src/a2a.rs`) still applies to agents waking each other;
- one turn per thread at a time.

### Where to read first

Verify each of these before relying on it; they came from a read-only survey, not a review.

- observers and delivery: `report_to_observers` in `crates/hq-tools/src/a2a.rs`, and
  `claim_next_message` in `crates/hq-db/src/tasks/messages.rs`;
- the supervisor sweep (every 60 seconds) and its event fast path:
  `crates/hq-cli/src/commands/start/daemon/tasks_periodic/session_supervisor.rs` and
  `session_events.rs`;
- the web driver: `crates/hq-web/src/session_driver.rs` (60-second pass, 48 drive turns a day)
  and `set_wake` in `crates/hq-db/src/harness_sessions_registry.rs`;
- timed turns: `crates/hq-tools/src/background_turns.rs` and `crates/hq-relay/src/watch_scheduler.rs`;
- how a busy chat is handled today: `crates/hq-relay/src/relay_common.rs` (the steer inbox) and
  `crates/hq-web/src/subagent_followup.rs`;
- the pattern to generalise: `crates/hq-tools/src/github/ci_wait.rs`.

### First steps

1. Write `docs/plans/durable-waits.md`: the problem, who waits on what today (with file
   references), the wait row, the resolver, delivery, migration, and how it will be measured.
   Push back in the plan on anything that does not pay for itself.
2. Build the smallest useful slice first: a session event resuming an HQ chat with no thread,
   and a harness session with no task. These are the two callers with no route today. Point
   `instead` at the new tool for both.
3. Then batched, ordered delivery to observers and the web driver.
4. Measure the model calls per waiting minute before and after each step, with the same numbers
   as section 3.

## Working rules

Branch and pull request for every change; the gates in `CLAUDE.md` before opening one. No real
hostnames, tokens, chat ids or names in code, docs, tests or fixtures. Servers and the phone
update from signed releases only.
