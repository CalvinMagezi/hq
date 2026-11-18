#!/usr/bin/env python3
"""Fold changelog.d/<slug>.<kind>.md fragments into CHANGELOG.md's [Unreleased] section.

Usage: python3 -I scripts/changelog_compile.py [--root DIR] [--check] [--keep] [--dry-run]
       (scripts/changelog-compile.sh is the entry point people run)

  --check    validate fragment names and bodies, write nothing (exit 1 on a problem)
  --keep     fold the fragments but leave the files in place
  --dry-run  print what would be folded, write nothing
  --root     repository root (default: the folder above this script)

Each fragment becomes one bullet at the top of the first `### <Kind>` section of `## [Unreleased]`
(the section is created, in the order Added, Changed, Fixed, Removed, Security, Deprecated, when the
file has none). Fragments are applied in file name order, so the output does not depend on the order
the filesystem lists them. Standard library only.
"""
import os
import re
import sys

KINDS = ["added", "changed", "fixed", "removed", "security", "deprecated"]
NAME = re.compile(r"^([a-z0-9]+(?:-[a-z0-9]+)*)\.(" + "|".join(KINDS) + r")\.md$")


def fragments(root):
    """(file name, kind, body) for every fragment, plus the problems found."""
    folder = os.path.join(root, "changelog.d")
    found, problems = [], []
    if not os.path.isdir(folder):
        return found, problems
    for name in sorted(os.listdir(folder)):
        if name == "README.md" or name.startswith("."):
            continue
        path = os.path.join(folder, name)
        if not os.path.isfile(path):
            continue
        m = NAME.match(name)
        if not m:
            problems.append(f"{name}: not named <slug>.<kind>.md (kinds: {', '.join(KINDS)})")
            continue
        body = open(path, encoding="utf-8").read().strip("\n")
        if not body.strip():
            problems.append(f"{name}: empty")
        elif body.lstrip().startswith(("- ", "* ", "#")):
            problems.append(f"{name}: write the entry text only, without a leading '- ' or heading")
        else:
            found.append((name, m.group(2), body.rstrip()))
    return found, problems


def bullet(body):
    lines = body.split("\n")
    out = ["- " + lines[0]]
    out += [("  " + l) if l.strip() else "" for l in lines[1:]]
    return out


def fold(text, items):
    lines = text.split("\n")
    try:
        start = next(i for i, l in enumerate(lines) if l.strip().startswith("## [Unreleased]"))
    except StopIteration:
        raise SystemExit("CHANGELOG.md has no '## [Unreleased]' heading")
    end = next((i for i in range(start + 1, len(lines)) if lines[i].startswith("## [")), len(lines))
    for kind in KINDS:
        mine = [b for _, k, b in items if k == kind]
        if not mine:
            continue
        heading = "### " + kind.capitalize()
        block = []
        for b in mine:
            block += bullet(b)
        at = next((i for i in range(start + 1, end) if lines[i].strip() == heading), None)
        if at is not None:
            # new entries go first, after the heading's blank line
            ins = at + 1
            if ins < end and not lines[ins].strip():
                ins += 1
            lines[ins:ins] = block
            end += len(block)
        else:
            ins = start + 1
            if ins < len(lines) and not lines[ins].strip():
                ins += 1
            new = [heading, ""] + block + [""]
            lines[ins:ins] = new
            end += len(new)
    return "\n".join(lines)


def main(argv):
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    keep = dry = check = False
    it = iter(argv)
    for a in it:
        if a == "--root":
            root = next(it)
        elif a == "--check":
            check = True
        elif a == "--keep":
            keep = True
        elif a == "--dry-run":
            dry = True
        else:
            raise SystemExit(__doc__)
    items, problems = fragments(root)
    for p in problems:
        print("changelog.d/" + p, file=sys.stderr)
    if problems:
        return 1
    if check:
        print(f"changelog.d: {len(items)} fragment(s) ok")
        return 0
    if not items:
        print("changelog.d: nothing to fold")
        return 0
    path = os.path.join(root, "CHANGELOG.md")
    text = open(path, encoding="utf-8").read()
    new = fold(text, items)
    for name, kind, _ in items:
        print(f"folded {name} into [Unreleased] / {kind.capitalize()}")
    if dry:
        return 0
    open(path, "w", encoding="utf-8").write(new)
    if not keep:
        for name, _, _ in items:
            os.remove(os.path.join(root, "changelog.d", name))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
