# Tech debt, one file per item

`TECHDEBT.md` is a 378 row table that nearly every pull request wanted to add a row to, so two
pull requests collided on it almost every time. It is now frozen: its rows stay where they are.
**A new item is a new file here**, `docs/techdebt/<slug>.md`.

## The file

```markdown
# One line saying what the debt is

**Reason skipped:** why it was not fixed in the change that found it.
**Date:** 2026-11-18
**Where:** `crates/hq-tools/src/lib.rs`, `crates/hq-tools/src/a2a.rs`
**Remove when:** the condition that makes it safe to delete (required for a `facade-` item).

Anything longer: what it costs, what the fix looks like, links.
```

- `<slug>`: lowercase letters, digits and hyphens. A temporary re-export left in place by a split
  or an extraction is named `facade-<what>`; it must say when it can go.
- The first line is a `# ` title. The three bold labels above are required; `Remove when` is
  required for `facade-` items and welcome on any other.
- `python3 -I scripts/techdebt-check.py` checks every file here; CI runs it on every pull request.

Code comments that say "tracked in TECHDEBT.md" still point at the old table, which is still
correct for those rows. New comments should name the file in this folder.
