---
name: hq-diagnose
description: How to find the cause of something broken, failing, flaky or slow in HQ. Build a fast, deterministic red feedback loop before theorising (a failing test at an existing seam, a script, a replay of real data, tool_usage failures, hq doctor, daemon logs, the vault), shrink it, test one hypothesis at a time, fix at the shared seam, lock it with a regression test, remove the debug code. Redact sensitive values from anything you quote. Use when asked to diagnose or debug, or when something throws, fails or regresses.
hints: [diagnose, diagnosing, debug this, feedback loop, minimal repro, root cause, flaky, bisect, regression test]
provenance:
  mintedBy: hq
---

# Diagnosing a bug in HQ

The loop is the skill. With a tight pass/fail signal that goes red on this bug, finding the cause is mechanical. Without one, reading code will not save you. Do not theorise before step 1 is done.

## Redact first

Everything you show or save (commands, logs, request dumps, notes) is quoted for a reader. Replace every sensitive value (anything that grants access, cookies, auth headers, private addresses) with `<REDACTED>`. Keep such values in environment variables so they never appear in what you run or paste. Quote only the lines that carry the signal. If redacted output is not enough to diagnose, say so and ask.

## 1. Build a fast red loop

Pick the first of these that reaches the bug:

1. A failing test at a seam that already exists (the public function, tool or route a caller uses). Copy a sibling test's setup; database tests use the real migrated schema.
2. A script or CLI call with a fixed input, diffed against a known good output.
3. A replay of real data, saved to disk and run through the code path alone: a failed tool call from `tool_usage`, a chat turn from `chat_messages`, a note or task from the vault, a captured request.
4. What HQ already knows: `hq doctor`, `hq mcp doctor`, `hq skills validate`, and the daemon logs around the time it failed.
5. A minimal throwaway harness with one function call and fake dependencies.
6. For "sometimes wrong": loop the trigger a hundred times, or pin the clock, seed and filesystem, until the rate is high enough to debug.
7. For "it used to work": automate good or bad at one commit and run `git bisect run`.

Then tighten it: faster (narrow the test, skip unrelated setup), sharper (assert the user's exact symptom, not "did not crash"), more deterministic.

Done when you can name one command you have already run, that is red for this symptom, repeatable, takes seconds and needs no human. If you cannot build one, stop and say what you tried, and ask for access, a redacted artifact or permission to add temporary instrumentation. Do not guess.

## 2. Reproduce and minimise

Run it and confirm it fails the way the user described, not a nearby way. Then cut inputs, config, data and steps one at a time, re-running after each cut, until every remaining piece is load-bearing. That is your regression test.

## 3. Hypothesise

Write three to five ranked guesses before testing any. Each states a prediction: "if X is the cause, changing Y makes it go away." No prediction, no hypothesis. Show the list to the person if they are there; do not wait on them.

## 4. Instrument

One probe per prediction, one variable at a time. Prefer a debugger or a single targeted log at a boundary over logging everything. Tag every debug line with a unique prefix such as `[DEBUG-a4f2]` so cleanup is one grep. For slowness, measure a baseline first, then bisect; logs mislead.

## 5. Fix at the shared seam, then lock it

- Grep every caller and sibling path of what is broken (other relays, the web and CLI twins, other tools using the same helper). Fix where they share the code, not at the one call site you noticed.
- Turn the minimal repro into a test at a real seam, watch it fail, fix, watch it pass, then rerun the original loop. If only a too-shallow seam exists, say so in the PR: that is a finding.

## 6. Clean up

- The original loop no longer reproduces.
- The regression test passes, or the missing seam is written down.
- `grep` for your debug prefix finds nothing; throwaway scripts are deleted.
- The commit or PR says which hypothesis was right, so the next person learns it. Then run the gates from hq-engineering-core.

Adapted from the diagnosing-bugs skill in mattpocock/skills (MIT, Matt Pocock).
