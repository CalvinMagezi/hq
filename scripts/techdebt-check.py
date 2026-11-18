#!/usr/bin/env python3
"""Checks docs/techdebt/<slug>.md items: a slug name, a '# ' title, and the required labels.

Usage: python3 -I scripts/techdebt-check.py [--root DIR]

Required in every item: a first line starting with '# ', and lines starting with the bold labels
'**Reason skipped:**', '**Date:**' (YYYY-MM-DD) and '**Where:**'. A file named facade-<what>.md also
needs '**Remove when:**', because a facade is temporary by definition. README.md is skipped.
Standard library only.
"""
import os
import re
import sys

SLUG = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*\.md$")
DATE = re.compile(r"^\*\*Date:\*\*\s*\d{4}-\d{2}-\d{2}\s*$", re.M)


def label(text, name):
    m = re.search(r"^\*\*" + re.escape(name) + r":\*\*[ \t]*(\S.*)$", text, re.M)
    return bool(m)


def check(root):
    folder = os.path.join(root, "docs", "techdebt")
    problems, count = [], 0
    if not os.path.isdir(folder):
        return problems, count
    for name in sorted(os.listdir(folder)):
        if name == "README.md" or name.startswith("."):
            continue
        path = os.path.join(folder, name)
        if not os.path.isfile(path):
            continue
        count += 1
        if not SLUG.match(name):
            problems.append(f"{name}: name must be <slug>.md (lowercase letters, digits, hyphens)")
            continue
        text = open(path, encoding="utf-8").read()
        first = text.lstrip("\n").split("\n", 1)[0]
        if not first.startswith("# ") or not first[2:].strip():
            problems.append(f"{name}: the first line must be a '# ' title")
        for need in ("Reason skipped", "Where"):
            if not label(text, need):
                problems.append(f"{name}: missing '**{need}:**' with text after it")
        if not DATE.search(text):
            problems.append(f"{name}: missing '**Date:** YYYY-MM-DD'")
        if name.startswith("facade-") and not label(text, "Remove when"):
            problems.append(f"{name}: a facade item needs '**Remove when:**'")
    return problems, count


def main(argv):
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    if argv[:1] == ["--root"]:
        root = argv[1]
    elif argv:
        raise SystemExit(__doc__)
    problems, count = check(root)
    for p in problems:
        print("docs/techdebt/" + p, file=sys.stderr)
    if problems:
        return 1
    print(f"docs/techdebt: {count} item(s) ok")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
