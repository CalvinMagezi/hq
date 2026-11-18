# Changelog fragments

A change a user or operator would notice adds **one file here** instead of editing `CHANGELOG.md`.
That keeps two pull requests from colliding on the same lines of a 1,100 line file.

## Add one

Name it `<slug>.<kind>.md`:

- `<slug>`: lowercase letters, digits and hyphens, for example `chat-keeps-your-place`.
- `<kind>`: `added`, `changed`, `fixed`, `removed`, `security` or `deprecated`.

The body is the entry as it should read in the changelog, without the leading `- `. Start with a bold
title, then say what changed and why it matters to the person using HQ:

```markdown
**A chat keeps your place.** A finished reply no longer collapses the chat for a moment, and a late
scroll report no longer stops it following the newest message.
```

More than one paragraph is fine; the compile step indents the continuation lines.

## What happens to it

`scripts/changelog-compile.sh` folds every fragment into the top of the matching `###` section of
`## [Unreleased]` in `CHANGELOG.md` (creating the section if there is none) and deletes the fragment
files. It runs when a release is cut, not in a pull request. `scripts/changelog-compile.sh --check`
validates names and bodies without writing anything, and CI runs it on every pull request.

Existing entries stay where they are in `CHANGELOG.md`; only new entries use this folder.
