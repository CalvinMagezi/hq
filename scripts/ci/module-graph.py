#!/usr/bin/env python3
"""Module dependency report for one crate's top-level modules.

Usage: python3 -I scripts/ci/module-graph.py [crate_src_dir] [--json]
Default crate_src_dir: crates/hq-tools/src

Reads every `crate::<module>` reference between the crate's top-level modules (a module is
`foo.rs`, `foo/`, or both) and prints:

  1. the largest strongly connected component (the tangle: modules that depend on each other),
  2. modules that are FREE (no path into the tangle) and DOWNSTREAM (depend on it),
  3. a layer order for the tangle that minimises upward references, and the BACK EDGES that
     remain (a lower layer reaching up). Each back edge is what must be inverted or moved.

It is a measurement tool, not a gate. The gate is crates/hq-tools/tests/layering.rs, which reads
crates/hq-tools/layers.toml. No third-party packages; stdlib only. Deterministic output.
"""
import collections
import json
import os
import re
import sys


def load(src):
    mods = collections.defaultdict(list)
    for e in sorted(os.listdir(src)):
        if e == "lib.rs":
            continue
        p = os.path.join(src, e)
        if e.endswith(".rs"):
            mods[e[:-3]].append(p)
        elif os.path.isdir(p):
            for dp, _, fs in os.walk(p):
                mods[e].extend(os.path.join(dp, f) for f in sorted(fs) if f.endswith(".rs"))
    return mods


def is_test_file(path):
    base = os.path.basename(path)
    return base.startswith("tests") or base.endswith("_tests.rs") or "/tests/" in path


def refs(text, names):
    """Yield (module, symbol) for each crate::module::symbol reference."""
    for m, s in re.findall(r"crate::([a-z_0-9]+)::([A-Za-z_0-9]+)", text):
        if m in names:
            yield m, s
    for m, grp in re.findall(r"crate::([a-z_0-9]+)::\{([^}]*)\}", text):
        if m in names:
            for s in re.findall(r"[A-Za-z_0-9]+", grp):
                yield m, s
    for m in re.findall(r"crate::([a-z_0-9]+)\b(?!::)", text): # `use crate::foo;`
        if m in names:
            yield m, "(module)"


def build(mods):
    names = set(mods)
    loc = {}
    w = collections.defaultdict(collections.Counter)            # (a,b) -> symbol counts, all files
    wnt = collections.defaultdict(collections.Counter)          # same, non-test files only
    for m, files in mods.items():
        loc[m] = 0
        for f in files:
            t = open(f, errors="ignore").read()
            loc[m] += t.count("\n")
            for b, s in refs(t, names):
                if b == m:
                    continue
                w[(m, b)][s] += 1
                if not is_test_file(f):
                    wnt[(m, b)][s] += 1
    return loc, w, wnt


def sccs(names, edges):
    sys.setrecursionlimit(10000)
    idx, low, st, on, out, c = {}, {}, [], set(), [], [0]

    def go(v):
        idx[v] = low[v] = c[0]
        c[0] += 1
        st.append(v)
        on.add(v)
        for x in sorted(edges.get(v, ())):
            if x not in idx:
                go(x)
                low[v] = min(low[v], low[x])
            elif x in on:
                low[v] = min(low[v], idx[x])
        if low[v] == idx[v]:
            comp = []
            while True:
                x = st.pop()
                on.discard(x)
                comp.append(x)
                if x == v:
                    break
            out.append(comp)

    for n in sorted(names):
        if n not in idx:
            go(n)
    return out


def layer_order(core, w):
    """Greedy + local search for the order with the fewest upward references."""
    cs = set(core)
    wt = {k: sum(v.values()) for k, v in w.items() if k[0] in cs and k[1] in cs}

    def cost(o):
        pos = {m: i for i, m in enumerate(o)}
        return sum(x for (a, b), x in wt.items() if pos[a] < pos[b])

    # Deterministic start: most depended-on first.
    indeg = collections.Counter()
    for (a, b), x in wt.items():
        indeg[b] += x
    o = sorted(core, key=lambda m: (-indeg[m], m))
    improved = True
    while improved:
        improved = False
        for i in range(len(o)):
            for j in range(len(o)):
                if i == j:
                    continue
                p = o[:]
                p.insert(j, p.pop(i))
                if cost(p) < cost(o):
                    o, improved = p, True
    return o, cost(o)


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    src = args[0] if args else "crates/hq-tools/src"
    mods = load(src)
    names = set(mods)
    loc, w, wnt = build(mods)
    edges = collections.defaultdict(set)
    for (a, b) in w:
        edges[a].add(b)
    comps = sccs(names, edges)
    core = max(comps, key=len)
    cs = set(core)

    def closure(m):
        seen, todo = set(), [m]
        while todo:
            x = todo.pop()
            for y in edges.get(x, ()):
                if y not in seen:
                    seen.add(y)
                    todo.append(y)
        return seen

    rest = names - cs
    downstream = sorted(m for m in rest if closure(m) & cs)
    free = sorted(m for m in rest if m not in downstream)
    order, cost = layer_order(sorted(core), w)
    pos = {m: i for i, m in enumerate(order)}
    back = []
    for (a, b), syms in sorted(w.items(), key=lambda kv: -sum(kv[1].values())):
        if a in cs and b in cs and pos[a] < pos[b]:
            back.append({
                "from": a, "to": b,
                "refs": sum(syms.values()),
                "refs_non_test": sum(wnt[(a, b)].values()),
                "symbols_non_test": dict(wnt[(a, b)].most_common(8)),
            })
    total = sum(loc.values())
    report = {
        "total_loc": total,
        "tangle": {"modules": order, "loc": sum(loc[m] for m in core)},
        "free": {"modules": free, "loc": sum(loc[m] for m in free)},
        "downstream": {"modules": downstream, "loc": sum(loc[m] for m in downstream)},
        "layer_order_low_to_high": order,
        "back_edges": back,
        "back_edge_refs": cost,
    }
    if "--json" in sys.argv:
        print(json.dumps(report, indent=2))
        return
    print(f"{src}: {total} lines in {len(names)} top-level modules")
    print(f"tangle: {len(core)} modules, {report['tangle']['loc']} lines ({100*report['tangle']['loc']//total}%)")
    print(f"free of the tangle: {len(free)} modules, {report['free']['loc']} lines")
    print(f"downstream of the tangle: {len(downstream)} modules, {report['downstream']['loc']} lines")
    print("\nlayer order, foundation first:\n " + " < ".join(order))
    print(f"\nback edges ({len(back)}, {cost} references): a lower layer reaching up")
    for b in back:
        print(f" {b['from']} -> {b['to']} refs={b['refs']} non-test={b['refs_non_test']} {b['symbols_non_test']}")
    print("\nfree modules by size:")
    for m in sorted(free, key=lambda k: -loc[k]):
        print(f" {m:24}{loc[m]:6} uses {sorted(edges.get(m, ()))}")


if __name__ == "__main__":
    main()
