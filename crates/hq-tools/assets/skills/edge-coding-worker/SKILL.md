---
name: edge-coding-worker
description: Do a small coding task on a GitHub repository without git, from a phone or any machine. Fetch the repository, have a coding sub-agent edit it, publish the change as an hq/ branch and a pull request, read CI, and fix until CI passes. Use when asked to work a coding task on a repository listed in github_write.repos, or an HQ task tagged edge-task.
hints: [edge-task, edge-worker, github_write, github_clone]
provenance:
  mintedBy: hq
---

# Edge coding worker

You drive, a sub-agent edits, CI tests, and a person merges. Never merge yourself.

1. **Claim the task** (`task_claim`) and read it. Done means CI passes and the task's own criteria hold. If the task names worker models, use them in that order; otherwise leave `model` unset.
2. **Fetch a clean copy.** Call `github_clone` with the `repo` and `refresh: true`, so earlier work is never carried into this task. Note the returned `path`. If a later step says the checkout is missing, fetch it again the same way.
3. **Edit through a sub-agent.** Call `spawn_subagents` with one child:
   - `agent_type`: `coder`, and `model`: the task's first worker model, when it names one.
   - `allowed_paths`: exactly `[<path>]`, nothing else. The child must reach no folder but the checkout.
   - `goal`: the task, the files it may touch, the done criteria, and: "Edit only files under <path>. Run no commands and do not use bash. Do not touch .github/. Finish with the list of files you changed and why."
   - `success_criteria`: the task's done criteria.
4. **Publish.** Call `github_write` with the `repo`, a `branch` named after the task (its display id in lower case), a commit `message`, a pull request `title`, and a `body` that names the task. Keep the pull request link it returns.
5. **Read CI.** Call `github_read` with action `run_list` for the repository and find the run for the branch's newest commit. If it is still running, look again in a few minutes; for a long wait, use `watch_create` instead of looping.
   - When a job fails, get its log: `github_read` action `api` with `repos/<repo>/actions/runs/<run id>/jobs`, then `repos/<repo>/actions/jobs/<job id>/logs`. Keep the lines around the first error.
   - A formatter or linter cannot run here, so pass its complaint on exactly: the child has to make the change the tool asked for by hand.
6. **Fix and push again.** Spawn a fresh child as in step 3 with the failing lines in `context`, then publish again to the same branch (step 4). After two failed fixes, move to the task's next worker model, if it names one. After four failed fixes in all, stop: release the task as `blocked`, with the pull request link and the last error.
7. **Hand over.** When CI passes, comment the pull request link on the task and release it as `ready_for_review`.

Never merge, never copy anything from outside the checkout into the repository (github_write refuses what looks private anyway), and keep one task to one pull request.
