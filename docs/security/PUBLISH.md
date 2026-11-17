# Publishing for a sandboxed helper

`harness_session_publish` lets a coding agent that cannot push ask HQ to push its branch and
open a pull request. This note says what HQ checks, what it never does and where the trust
sits. Usage and config: `docs/AGENT_SESSIONS.md`, "Publishing a helper's work".

## Who is asking

The caller is the session the gateway attested with the call's own session token
(`_hq_caller_session`). The tool reads no session, path or branch from the arguments; its
schema is a title and a body. The worktree and branch are the ones HQ recorded when it made the
worktree, and the host checks the branch is what the worktree has checked out. A call that did
not come from a launched session is refused.

## What the host checks

Everything below runs on the host, outside any sandbox, with the pinned git configuration of
`hq-host::host_git` (no hooks, no fsmonitor, no filter drivers, no `ext::` transport).

- The path is a worktree the host made (`worktrees/<repository>/<session>`).
- The branch name passes the same check as `github_write`, is the checked-out branch, and is
  not the default branch, `main`, `master`, `develop` or `release/*`.
- The tree is clean and the branch is ahead of `origin/<default>`.
- The diff from the merge base lists no file under `.github/`, `.circleci`, `.buildkite`,
  `.gitlab`, `.husky`, `.githooks`, no CI runner file (`Jenkinsfile`, `.gitlab-ci.yml`, and
  similar), no `.git*` file, no submodule pointer; it stays under the file count and per-file
  size caps; and no added blob, title or body is shaped like a credential or equals a secret in
  the host's environment. Blobs are read with `git cat-file`, so no filter or text conversion
  runs.
- A push needs a transport, so only `protocol.allow` is relaxed. The repository is refused if
  its own configuration sets a key that makes a push run a program (`host_git::PUSH_PROGRAM_KEYS`)
  or sends it elsewhere (`remote.<name>.pushurl`, `url.<base>.insteadOf`). The host's login is
  never offered to an address the agent chose.
- `origin` must be the repository the owner allowed. The tool learns it from a check-only call
  first, then publishes with that name and the host checks it again.

## What HQ never does

- Merge, approve, or enable auto-merge on its own say. A merge happens only through
  `harness_session_request_merge` and the owner's yes for one commit (below).
- Force-push, delete a ref, or push a tag. The refspec is `HEAD:refs/heads/<branch>` and a
  diverged remote branch is a failed push, not an overwrite.
- Touch the default or a protected branch, or publish workflow, hook or git control files.
- Fall back to another credential or API when `gh` is missing or logged out.

`gh pr create` runs from the HQ folder, not from the agent's repository, with `--repo`,
`--head` and the body on stdin, so repository configuration the agent wrote is not read by `gh`.

## The owner's say

A repository not listed in `agent_host.publish.repos` needs the owner. The request is an inbox
item with the same signed-approval path as a new project folder (`harness_session/folder_request.rs`);
nothing a helper or an MCP caller can reach signs an approval. An approval lasts 30 days.

## Limits of v1

There is no model review of the diff: the checks are mechanical, so a harmful change that is
not a credential and not a protected path is reviewed by the people reviewing the pull request.
Logged in `TECHDEBT.md`.

## Merging a green pull request (`harness_session_request_merge`)

Off by default. `agent_host.publish.merge.enabled: true` turns it on; `merge.repos` lists the
repositories (empty means `publish.repos`) and `never_merge` lists repositories HQ refuses with
the reason "open pull requests and never merge" (a client repository whose rule that is). The
tool takes one argument, `pr` (an address or `<owner>/<repo>#<n>`). It never merges by itself.

1. HQ refuses a repository that is off the list or on `never_merge`, with no host call.
2. The host reads the pull request with its own `gh` (`pr.merge` without an approved commit) and
   refuses unless it is open, not a draft, from a branch of the repository itself, targeting the
   default branch, not from a protected branch, mergeable, free of merge commits, and every check
   reported passed (none pending, none failed; no checks at all is a refusal).
3. HQ files one inbox item naming the repository, title, number, head commit and the passing
   checks. The approval is signed, spent by exactly one merge, and bound by `args_digest` over
   `{repo, pr, head_sha}`, so a new commit is a different request and asks again. It lasts 24
   hours; a dismissal is a no for that commit only.
4. The helper gets `waiting_for_owner` and calls again after the owner answers. Only then does
   the host re-read the pull request, require the head to equal the approved commit and the
   checks to still be green, and run `gh pr merge --squash --delete-branch --match-head-commit
   <sha>`, once. GitHub itself refuses the merge if a commit landed in between.
5. HQ comments on the session's task and records a `merge` event.

Never: force, `--admin`, auto-merge, a repository outside the list, a fork, the default branch
as head. The helper's arguments carry no approval, commit or session; identity comes from the
gateway's attested session, and nothing a helper or an MCP caller can reach signs an approval.
