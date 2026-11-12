---
name: hq-engineering-core
description: How to make a change in the HQ repo or finish an HQ task. Write the failing test first at an existing seam, run single-crate tests while working and the full gates once at the end, review your own diff on two axes (task Acceptance, repo standards) before the PR, report what was not verified, file unrelated findings as tasks, never merge. Use when implementing, fixing, or preparing a pull request.
hints: [red green, failing test first, test first, self review, before opening a pr, pull request gates, clippy gate, definition of done]
---

# Engineering core for HQ work

How to do a change in this repo, or an HQ task, so a reviewer finds nothing to send back. Keep to this order.

## 1. Test first, at an existing seam

- Write the failing test before the code, and watch it fail for the right reason. Then write only enough code to pass it. One test, one change, repeat; do not write all the tests up front.
- Put the test at a seam that already exists: the public function, tool or route a caller uses. Find a sibling test in the same crate and copy its setup. Do not mock internals or assert on private state.
- Database tests use the real migrated schema (`crate::migrations::run`), never a hand-built table.
- A test that needs outside state (a real vault, Ollama, an authenticated harness binary, the network) goes behind `#[ignore = "reason"]` with the opt-in command in a comment. It is a diagnostic, not a gate.
- Expected values come from a literal or a worked example, never from recomputing the answer the way the code does.

## 2. While working: small and fast

Run `cargo test -p <crate>` and `cargo check` as you go. Do not run the whole workspace after every edit.

## 3. Gates, once, at the end

1. `cargo fmt --all`
2. `rtk proxy cargo clippy --workspace --all-targets -- -D warnings` (the `rtk` filters hide errors, so go through `rtk proxy`)
3. `cargo test --workspace`. It stops at the first failing test binary, so read the totals across every suite, not the exit of the first one. A partial run is not green.
4. If you touched the web app: `cd apps/hq-web && bun install && bun run build`

## 4. Review your own diff before the PR

Read `git diff origin/main...HEAD` as a stranger, on two separate axes. Do not let one excuse the other.

- **Acceptance.** Go through the task's Acceptance lines one by one. For each, say what you ran or saw that proves it. Flag anything the diff does that the task did not ask for.
- **Standards.** Does it follow the repo CLAUDE.md and the sibling code? Grep every caller and every sibling path of what you changed (other relays, the web and CLI twins, docs that name it) and confirm each is consistent. Nothing machine-specific, no real hostnames or names in code, docs or fixtures. No em-dashes in prose.

For a bigger change, hand the diff to a fresh subagent with only the two axes and the task text, not your conclusions.

## 5. Report plainly

In the PR body and the task, list what you ran and what passed, then what you did not verify (a gate you skipped, a platform you could not test, an `#[ignore]` test you did not run) and why. Never write "should work".

## Boundaries

- A problem you find that is not this task becomes a new task, not part of the diff.
- Work on a branch, open a pull request, and stop. Never merge your own PR.

Adapted from the tdd, implement and code-review skills in mattpocock/skills (MIT, Matt Pocock).
