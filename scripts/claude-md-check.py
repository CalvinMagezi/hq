#!/usr/bin/env python3
"""Keep the root CLAUDE.md small and its index honest.

The root file is loaded into every session, so it has a size budget. Area notes live in a
CLAUDE.md beside their code. This check fails when:

- the root CLAUDE.md is over the budget,
- the "Where things live" table names a file that does not exist,
- a nested CLAUDE.md exists that the table does not list.

Run from the repository root: python3 -I scripts/claude-md-check.py
"""
import re
import subprocess
import sys
from pathlib import Path

BUDGET = 20 * 1024
ROOT = Path(__file__).resolve().parent.parent


def nested_files() -> set[str]:
    out = subprocess.run(
        ["git", "ls-files", "*CLAUDE.md"], cwd=ROOT, capture_output=True, text=True, check=True
    ).stdout.split()
    return {p for p in out if p != "CLAUDE.md"}


def indexed(root_text: str) -> set[str]:
    m = re.search(r"^## Where things live\n(.*?)(?=^## )", root_text, re.S | re.M)
    if not m:
        return set()
    return set(re.findall(r"^\| `([^`]+CLAUDE\.md)` \|", m.group(1), re.M))


def problems(root_text: str, size: int, listed: set[str], on_disk: set[str], exists) -> list[str]:
    found = []
    if size > BUDGET:
        found.append(f"CLAUDE.md is {size} bytes, over the {BUDGET} byte budget: move area notes into a nested CLAUDE.md")
    if not listed:
        found.append('CLAUDE.md has no "Where things live" table')
    for f in sorted(listed):
        if not exists(f):
            found.append(f"the index lists {f}, which does not exist")
    for f in sorted(on_disk - listed):
        found.append(f"{f} exists but the index in CLAUDE.md does not list it")
    return found


def main() -> int:
    text = (ROOT / "CLAUDE.md").read_text()
    found = problems(
        text, len(text.encode()), indexed(text), nested_files(), lambda f: (ROOT / f).is_file()
    )
    for f in found:
        print(f"claude-md-check: {f}", file=sys.stderr)
    if not found:
        print(f"claude-md-check: ok ({len(text.encode())} bytes, {len(indexed(text))} nested files)")
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main())
