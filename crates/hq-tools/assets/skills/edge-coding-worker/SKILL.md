---
name: edge-coding-worker
description: Do a small coding task on a GitHub repository without git, from a phone or any machine. Fetch the repository, have a coding sub-agent edit it, publish the change as an hq/ branch and a pull request, read CI, and fix until CI passes. Use when asked to work a coding task on a repository the owner picked for HQ's GitHub App or listed in github_write.repos, or an HQ task tagged edge-task.
hints: [edge-task, edge-worker, github_write, github_clone]
provenance:
  mintedBy: hq
---

# Edge coding worker

You drive, a sub-agent edits, CI tests, and a person merges. Never merge yourself.

1. **Claim the task** (`task_claim`, with `actor: "HQ"`). The reply holds the task, and `resume` when an earlier run was cut off: its checkpoint says what was already done. Done means CI passes and the task's own criteria hold. If the task names worker models, use them in that order; otherwise leave `model` unset. Call `coder_context` with the task for any notes the owner linked.
2. **Fetch.** If the checkpoint says a branch was pushed, call `github_clone` with the `repo` and no `refresh`, keep working in that checkout, and pick up at its next step. Otherwise call it with `refresh: true`, so earlier work is never carried into this task. Note the returned `path`. If the checkout is missing after a push, fetch it with `refresh: true` and redo the whole task, since publishing replaces what the branch holds.
3. **Edit through a sub-agent.** Call `spawn_subagents` with one child:
   - `agent_type`: `coder`, and `model`: the task's first worker model, when it names one.
   - `allowed_paths`: exactly `[<path>]`, nothing else. The child has file and search tools there, and no shell.
   - `goal`: the task, the files it may touch, the done criteria, and: "First call load_skill for hq-engineering-edge, and for hq-engineering-core to borrow its two review axes (Acceptance, Standards) only, since you cannot run its gates. Edit only files under <path>. Run no commands and do not use bash. Do not touch .github/. Finish with the list of files you changed and why."
   - `success_criteria`: the task's done criteria.
4. **Publish.** First call `task_comment_list`: a note from `owner` is a change to the job, so make it before publishing. Call `github_write` with the `repo`, a `branch` named after the task (its display id in lower case), a commit `message`, a pull request `title`, and a `body` that names the task. Keep the pull request link it returns. Then call `task_heartbeat` with a `checkpoint`: the branch, the link, how many fixes so far, and the next step.
5. **Read CI.** Call `github_read` with action `wait_ci`, the `repo` and the branch as `ref`. It waits for you at no cost and answers `ci`: `passed`, `failed`, `none` (this commit has no CI, so say so in the handover) or `running` (call it once more).
   - When a run fails, get its log: `github_read` action `api` with `repos/<repo>/actions/runs/<run id>/jobs`, then `repos/<repo>/actions/jobs/<job id>/logs`. Keep the lines around the first error.
   - A formatter or linter cannot run here, so pass its complaint on exactly: the child has to make the change the tool asked for by hand.
6. **Fix and push again.** Spawn a fresh child as in step 3 with the failing lines in `context`, then publish again to the same branch (step 4). After two failed fixes, move to the task's next worker model, if it names one. After four failed fixes in all, stop: release the task as `blocked`, with the pull request link and the last error.
7. **Hand over.** When CI passes, release the task as `ready_for_review`, with the pull request link in the `summary`.

Never merge, never copy anything from outside the checkout into the repository (github_write refuses what looks private anyway), and keep one task to one pull request.
