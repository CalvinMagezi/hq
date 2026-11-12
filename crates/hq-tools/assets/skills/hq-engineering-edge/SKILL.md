---
name: hq-engineering-edge
description: How to make a code change when you have file and search tools but no shell, as the edge worker's coding sub-agent does. Write the test in the same change, read every caller by hand, never run commands, and treat a red CI log as the only diagnosis input. The no-shell variant of hq-engineering-core. Use when your tools have no bash, or when told to load it.
hints: [no shell, no bash, edge change, ci log, red ci, fix from ci]
provenance:
  mintedBy: hq
---

# Engineering without a shell

This is `hq-engineering-core` for a session that cannot run anything. Same goals, different means: CI runs the commands you cannot, and a person reads the result.

## Before the edit

- Read the files you will touch and one sibling test next to them. Copy its setup and naming.
- Find every caller of what you change by reading and searching files: the function name, the type, the config key, the docs that mention it. Open each hit. Do not assume a rename or a new argument reaches only one place.

## The edit

- Write the test in the same change as the code, at the seam a caller already uses. Expected values are literals or worked examples, never a recomputation of what the code does. A test that needs outside state (network, a real vault) is marked ignored with its reason.
- You cannot watch the test fail, so make it unable to pass by accident: it must assert something the old code gets wrong, and say in your handover which behavior it pins.
- Match the surrounding style by eye. A formatter and a linter run later in CI, so write the layout they would produce: sorted imports, no unused names, no leftover debug output.
- Touch only what the task needs. Do not edit `.github/`. An unrelated problem goes in your handover, not the diff.

## When CI is red

You are handed the first error lines of the failing log. They are your whole diagnosis, so:

- Fix the first error before any later one, since later ones often follow from it.
- When the tool says what to change (a formatter diff, a lint name), make exactly that change by hand.
- When the cause is not clear from the lines, read the named file and line, and the caller of what it names. Do not guess a second change to see if it helps.

## Handover

End with the files you changed and why, what the new test pins, and what you could not check because nothing ran here. Never write "should work".
