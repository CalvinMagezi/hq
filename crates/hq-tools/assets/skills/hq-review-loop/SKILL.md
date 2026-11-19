---
name: hq-review-loop
description: Run a bounded plan-implement-review-fix loop on a coding change, with a security checklist in every review round. Review the diff fresh (not from the author's summary), fix only confirmed findings, review again. Cap at 3 rounds. Use when asked for a review loop, to review my changes, for a security review before a PR, or to plan then implement.
hints: [review loop, review my changes, security review before pr, plan then implement, bounded review, fix then re-review]
provenance:
  mintedBy: hq
---

# Bounded review loop with a security pass

A coding change that stops after one self-check misses what a second read finds. Run this loop. Do not skip a step. Do not invent a fourth review round.

## The loop

1. **Restate acceptance.** One sentence: what must be true when this is done. Take it from the task, ticket or ask. If none is clear, write the sentence you will hold yourself to and say that you chose it.
2. **Smallest plan.** A short list of the fewest edits that meet that sentence. No drive-by cleanup.
3. **Implement.** Only what the plan names. Stay inside the files you own.
4. **Independent review.** Read the diff as a stranger. Do not trust your own summary of what you changed. Open the files and lines. Run the security checklist below on this round.
5. **Fix confirmed findings only.** A finding is confirmed when you can point to a file and line, and you can reproduce or show the fault. Anything else is dropped (see Verify).
6. **Review again.** Same independent read and the same security checklist on the new diff. Then either stop (no confirmed finding) or fix and review once more.

**Cap.** At most 3 review rounds (the first review after implement, then up to two more after fixes). When the third round still has confirmed findings, stop looping. Report the remainder in the final report and do not start a fourth round.

## Security checklist (every review round)

For each item answer **yes**, **no**, or **not applicable**, and name the file and line that decided it. "Not applicable" still needs a one-line why (for example "no new network path").

1. **Access material in the tree.** Private values that grant access appear in code, logs or fixtures? (yes / no / n/a, path:line)
2. **Authorization gaps.** A new or changed read or write path is missing a check of who may do it (look past the path the ticket names)? (yes / no / n/a, path:line)
3. **Injection.** SQL, shell, path traversal, or prompt injection from untrusted text is possible on a changed path? (yes / no / n/a, path:line)
4. **Trust boundaries.** Untrusted input (web, mail, documents, other agents, user text) is accepted without hostile treatment where it enters? (yes / no / n/a, path:line)
5. **Dependencies.** A new dependency was added that is unnecessary, or its tree was not reviewed? (yes / no / n/a, path:line or "none added")
6. **Error paths.** A failure leaks internals to callers, or fails open (allows when the check errors)? (yes / no / n/a, path:line)

A **yes** on any item is a finding. Carry it into Verify.

## Verify before you act

Before you change code for a finding:

- Point to the file and line.
- Reproduce it, or show why that line is wrong without a run (for example a missing auth call on a write path).

If you cannot do both, **drop** the finding with one line: what it claimed, and why it is dropped (cannot reproduce, no line, already fixed, out of scope).

## Final report

When the loop ends (clean review, or cap hit), write:

1. **Findings fixed.** Each one: what it was, file:line, what you changed.
2. **Findings dropped.** Each one: claim, one-line reason.
3. **Not verified.** Gates, platforms or paths you did not check, and why.
4. If the cap was hit: **Remainder.** Confirmed findings still open after round 3. Do not keep looping.
