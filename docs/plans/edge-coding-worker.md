# Edge coding worker

HQ's own agent loop doing small coding tasks on a phone, with no laptop and no third-party coding
agent. The phone fetches a repository, a sub-agent edits it with HQ's file tools, and the result
goes back to GitHub as a branch and a pull request. CI on that pull request is the test runner:
a phone has no toolchains, and the worker never needs one.

## Why not a coding agent under PRoot

Running Claude Code or similar on Android means an Ubuntu userspace under PRoot. That is about
700 MB of bundles, a ptrace slowdown on every system call, an app pinned to `targetSdk 28`, and
redistribution questions for the agent binary. HQ already ships a Rust agent loop with file,
search and edit tools in `libhq.so`, so the worker costs no new binary. The model runs remotely
(an Anthropic API backend in `backends:`), so the phone's CPU barely matters.

## Steps

| Step | What | Where |
|------|------|-------|
| S0 | A sub-agent on the phone runs on an API backend declared in `backends:` | config only |
| S1 | `github_clone` without git or gh: the default branch's tarball, unpacked as untrusted input, with a base record beside the checkout | `crates/hq-tools/src/github/tarball.rs` |
| S6 | `grep` without ripgrep | `crates/hq-agent/src/coding/search_fallback.rs` |
| S2 | `github_write`: the checkout's changes as an `hq/` branch and a pull request | `crates/hq-tools/src/github/write.rs`, `changes.rs` |
| S3 | A worker preset (file and search tools, `github_clone`, `github_write`, no shell) and a first live run against a small repository with fast CI | to come |

## `github_write`: what it may do

- **Off by default.** It refuses every call until the owner lists repositories in
  `github_write.repos`. `config_manage` refuses that key. Agents cannot write the config file
  either, because the governance layer denies the active config path to every file tool.
- **Its own token.** It reads only `HQ_GITHUB_WRITE_TOKEN`, never the read-only `GH_TOKEN` or
  `GITHUB_TOKEN`, and sends it only to `api.github.com`. Use a fine-grained token limited to the
  listed repositories: Contents and Pull requests read and write, Actions read, no Workflows. On
  the phone, store it under Settings, Keys on this phone.
- **Branches under `hq/` and pull requests only.** No other branch, no force, no merge, no
  deleted ref. A branch that already exists must sit at the commit this tool last pushed there, a
  SHA recorded beside the checkout, so a person's later push to that branch is never rewritten.
  Branch protection on the default branch is a cheap server-side backstop.
- **Only the checkout is read.** The tool walks the `github_clone` checkout. It refuses links,
  special files and `.git` entries, so a link to a secret outside the checkout cannot be
  published. Caps: 100 changed files and 5 MiB per file.

## Why taint does not gate it

A worker reads repository text, issues and CI logs, all untrusted, so its session is always
tainted (`governance/taint.rs` marks `github_clone` and `github_read`). A taint gate would make
the tool useless. The risk taint guards against here is exfiltration: an injected instruction
that gets a secret published. That is handled at the boundary instead:

- every changed file and the branch name, commit message, pull request title and body are
  scanned with `hq_core::redact::looks_like_credential`, and a match refuses the whole push (the
  refusal names the file, never the value);
- a tainted session cannot read secret files in the first place (`governance/secrets.rs`);
- everything lands in a pull request that a person reviews before it reaches the default branch.

What the scan cannot catch is private but non-secret text, such as a vault note copied into the
checkout. The worker preset (S3) must therefore limit the worker's `allowed_paths` to its
checkout and scratch folder. S2's safety argument depends on that, and S3 has to deliver it.

## Phone notes

- `TMPDIR` is the app's cache folder (`HubService`), so checkouts live under
  `cacheDir/hq-github-clones`. Android may clear the cache under storage pressure, so the worker
  must tolerate a vanished checkout by fetching again.
- With a `backends:` chain configured, provider keys saved in Settings reach a model only through
  a backend entry that declares them.
