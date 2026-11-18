#!/usr/bin/env python3
"""How much of the workspace must rebuild when a typical commit lands?

Usage:   python3 -I scripts/ci/affected-report.py [--commits N] [--only PATH_PREFIX]   (N defaults to 400)

For each crate: its own lines of Rust, and the lines in it plus everything that depends on it
(what a clean-cache CI build has to recompile when that crate changes). Then walks the last N
commits on the current branch, maps changed files to crates, and reports the share of the
workspace's lines that had to rebuild per commit.

A commit that changes Cargo.lock, the root Cargo.toml or .cargo/ counts as 100%.
Commits that touch no crate are reported separately and left out of the percentages.

--only PATH_PREFIX keeps only the commits that changed a file under that prefix (for example
`--only crates/suite/` for the suite family), so the median answers "what does a change in this
area rebuild?". The dependency graph is always the one in the working tree.

Stdlib only. Run it before and after a restructuring; the median is the number to move.
"""
import collections
import glob
import os
import re
import subprocess
import sys


def main():
    n_commits = 400
    if "--commits" in sys.argv:
        n_commits = int(sys.argv[sys.argv.index("--commits") + 1])
    only = None
    if "--only" in sys.argv:
        only = sys.argv[sys.argv.index("--only") + 1]

    crates = {}
    for p in glob.glob("crates/*/Cargo.toml") + glob.glob("crates/suite/*/Cargo.toml"):
        text = open(p).read()
        m = re.search(r'^name\s*=\s*"([^"]+)"', text, re.M)
        if m:
            crates[m.group(1)] = (os.path.dirname(p), text)
    names = set(crates)

    deps = {n: set() for n in names}
    for n, (_, text) in crates.items():
        for m in names:
            if m != n and re.search(r"^\s*" + re.escape(m) + r"\s*=", text, re.M):
                deps[n].add(m)

    loc = {}
    for n, (d, _) in crates.items():
        total = 0
        for dp, _, fs in os.walk(os.path.join(d, "src")):
            for f in fs:
                if f.endswith(".rs"):
                    total += open(os.path.join(dp, f), errors="ignore").read().count("\n")
        loc[n] = total
    total = sum(loc.values())

    rev = collections.defaultdict(set)
    for a, ds in deps.items():
        for b in ds:
            rev[b].add(a)

    def closure(n):
        seen, todo = {n}, [n]
        while todo:
            x = todo.pop()
            for y in rev[x]:
                if y not in seen:
                    seen.add(y)
                    todo.append(y)
        return seen

    affected = {n: sum(loc[x] for x in closure(n)) for n in names}
    print(f"workspace: {len(names)} crates, {total} lines of Rust (src/ only)\n")
    print(f"{'crate':22}{'lines':>8}{'rebuilds':>10}{'share':>7}")
    for n in sorted(names, key=lambda k: -affected[k])[:14]:
        print(f"{n:22}{loc[n]:8}{affected[n]:10}{100*affected[n]//total:6}%")

    out = subprocess.run(
        ["git", "log", f"-{n_commits}", "--name-only", "--format=@@%h"],
        capture_output=True, text=True, check=True,
    ).stdout.split("@@")[1:]
    dirs = {d: n for n, (d, _) in crates.items()}
    shares, no_crate = [], 0
    for blk in out:
        files = [l for l in blk.splitlines()[1:] if l]
        if only and not any(f.startswith(only) for f in files):
            continue
        touched, whole = set(), False
        for f in files:
            if f in ("Cargo.lock", "Cargo.toml") or f.startswith(".cargo/"):
                whole = True
            for d, n in dirs.items():
                if f.startswith(d + "/"):
                    touched.add(n)
        if whole:
            shares.append(1.0)
        elif touched:
            u = set()
            for n in touched:
                u |= closure(n)
            shares.append(sum(loc[x] for x in u) / total)
        else:
            no_crate += 1
    shares.sort()
    k = len(shares)
    if not k:
        print("\nno commits touched a crate")
        return
    print(f"\nlast {n_commits} commits: {k} touched a crate, {no_crate} touched none")
    print("share of the workspace that rebuilds per commit:")
    print(f" median {100*shares[k//2]:.0f}%    p75 {100*shares[int(k*.75)]:.0f}%   mean {100*sum(shares)/k:.0f}%")
    print(f" commits rebuilding >= 90%: {sum(1 for s in shares if s >= .9)} ({100*sum(1 for s in shares if s >= .9)//k}%)")
    print(f" commits rebuilding <= 40%: {sum(1 for s in shares if s <= .4)} ({100*sum(1 for s in shares if s <= .4)//k}%)")


if __name__ == "__main__":
    main()
