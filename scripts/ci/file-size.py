#!/usr/bin/env python3
"""Production-line ratchet for Rust files: no new file grows past 1,500 lines, and the few that
already are can only shrink.

Usage: python3 -I scripts/ci/file-size.py [--root DIR] [--top N]

A "production line" is a line of a .rs file under crates/*/src that is not inside a
`#[cfg(test)]` item (an inline `mod tests { ... }`, a test-only function and so on). Blank lines
and comments count: the number is the file as a person scrolls through it, minus the tests. Left
out entirely: `tests.rs`, `*_tests.rs`, anything under a `tests/`, `benches/` or `examples/`
folder, `build.rs`, and generated files (`.gen.` in the name, or a first lines marker saying so).

  warn   more than 1,000 production lines: printed, the run still passes
  fail   more than 1,500 production lines, unless the file is in scripts/ci/file-size-allow.toml
         at or under the cap written there

The allowlist can only shrink, like the layering allowlist: a cap above the file's real size fails
(lower it to the number printed), an entry for a file that is no longer over 1,500 fails (delete
it), and an entry for a file that does not exist fails. Raising a cap or adding a file is a
deliberate edit that review sees.

--top N prints the N largest files (default 0). Standard library only (tomllib, Python 3.11+).
"""
import os
import sys
import tomllib

WARN_AT = 1000
FAIL_AT = 1500
SKIP_DIRS = {"tests", "benches", "examples", "target", "node_modules"}
GENERATED_MARKERS = ("@generated", "this file is generated", "do not edit", "auto-generated")


def is_ident(c):
    return c.isalnum() or c == "_"


def scan(text):
    """Yields (line number from 0, in_test_item) for every line of `text`.

    Tracks comments, string and char literals so a brace in one is not counted. When it sees an
    attribute `#[cfg(test)]` it marks everything up to the end of the item that follows (to the
    matching `}` of its first block, or to its `;` if it has no block) as test code.
    """
    n = len(text)
    line_of = [0] * (n + 1)
    ln = 0
    for i, ch in enumerate(text):
        line_of[i] = ln
        if ch == "\n":
            ln += 1
    line_of[n] = ln
    total_lines = ln + (0 if text.endswith("\n") or not text else 1)
    test_lines = set()

    i = 0
    depth = 0
    pending = None          # (depth at the attribute, start line) once `#[cfg(test)]` is seen
    in_item = None          # while inside a test item: the depth to return to
    start_line = 0

    def skip_string(i):
        # i is at the opening quote of a normal string
        i += 1
        while i < n:
            if text[i] == "\\":
                i += 2
                continue
            if text[i] == '"':
                return i + 1
            i += 1
        return n

    def skip_raw(i):
        # i is at 'r'; returns the index after the raw string, or None if it is not one
        j = i + 1
        hashes = 0
        while j < n and text[j] == "#":
            hashes += 1
            j += 1
        if j >= n or text[j] != '"':
            return None
        end = '"' + "#" * hashes
        k = text.find(end, j + 1)
        return n if k < 0 else k + len(end)

    while i < n:
        c = text[i]
        two = text[i:i + 2]
        if two == "//":
            j = text.find("\n", i)
            i = n if j < 0 else j
            continue
        if two == "/*":
            level, i = 1, i + 2
            while i < n and level:
                if text[i:i + 2] == "/*":
                    level, i = level + 1, i + 2
                elif text[i:i + 2] == "*/":
                    level, i = level - 1, i + 2
                else:
                    i += 1
            continue
        if c == '"':
            i = skip_string(i)
            continue
        if c == "r" and (i == 0 or not is_ident(text[i - 1])):
            j = skip_raw(i)
            if j is not None:
                i = j
                continue
        if c == "b" and (i == 0 or not is_ident(text[i - 1])) and text[i + 1:i + 2] in ('"', "'"):
            i += 1
            continue
        if c == "'":
            # a char literal ('a', '\n', '\'' ) or a lifetime ('a); only the first is skipped
            if text[i + 1:i + 2] == "\\":
                j = text.find("'", i + 3)
                i = n if j < 0 else j + 1
                continue
            if text[i + 2:i + 3] == "'":
                i += 3
                continue
            i += 1
            continue
        if c == "#" and text.startswith("#[cfg(test)]", i) and in_item is None and pending is None:
            pending = (depth, line_of[i])
            i += len("#[cfg(test)]")
            continue
        if pending is not None and in_item is None:
            if c == ";":
                for k in range(pending[1], line_of[i] + 1):
                    test_lines.add(k)
                pending = None
            elif c == "{":
                in_item = pending[0]
                start_line = pending[1]
                pending = None
                depth += 1
                i += 1
                continue
        if c == "{":
            depth += 1
        elif c == "}":
            depth -= 1
            if in_item is not None and depth == in_item:
                for k in range(start_line, line_of[i] + 1):
                    test_lines.add(k)
                in_item = None
        i += 1
    return total_lines, test_lines


def production_lines(text):
    total, test = scan(text)
    return total - len(test)


def is_generated(path, text):
    name = os.path.basename(path)
    if ".gen." in name or name.endswith(".generated.rs"):
        return True
    head = "\n".join(text.split("\n", 6)[:6]).lower()
    return any(m in head for m in GENERATED_MARKERS)


def rust_files(root):
    base = os.path.join(root, "crates")
    for dp, dirs, files in os.walk(base):
        dirs[:] = sorted(d for d in dirs if d not in SKIP_DIRS)
        rel = os.path.relpath(dp, root).replace(os.sep, "/")
        if "/src" not in "/" + rel + "/" and not rel.endswith("/src"):
            continue
        for f in sorted(files):
            if not f.endswith(".rs") or f in ("tests.rs", "build.rs") or f.endswith("_tests.rs"):
                continue
            yield rel + "/" + f


def measure(root):
    sizes = {}
    for rel in rust_files(root):
        text = open(os.path.join(root, rel), encoding="utf-8", errors="replace").read()
        if is_generated(rel, text):
            continue
        sizes[rel] = production_lines(text)
    return sizes


def load_allow(root):
    path = os.path.join(root, "scripts", "ci", "file-size-allow.toml")
    if not os.path.exists(path):
        return {}
    data = tomllib.load(open(path, "rb"))
    return {k: int(v) for k, v in data.get("allow", {}).items()}


def check(sizes, allow):
    """(problems, warnings) for measured sizes and the allowlist."""
    problems, warnings = [], []
    for path, n in sorted(sizes.items()):
        cap = allow.get(path)
        if n > FAIL_AT:
            if cap is None:
                problems.append(
                    f"{path}: {n} production lines, over {FAIL_AT}. Split it (one concern per file), "
                    f"or, if it truly cannot be split yet, add it to scripts/ci/file-size-allow.toml "
                    f"at {n} and say why in the pull request"
                )
            elif n > cap:
                problems.append(f"{path}: grew to {n} production lines, past its cap of {cap}. Caps only go down")
            elif n < cap:
                problems.append(f"{path}: is {n} production lines now, below its cap of {cap}: lower the cap to {n}")
        elif cap is not None:
            problems.append(
                f"{path}: is {n} production lines, no longer over {FAIL_AT}: delete it from scripts/ci/file-size-allow.toml"
            )
        if WARN_AT < n <= FAIL_AT:
            warnings.append(f"{path}: {n} production lines (warn above {WARN_AT})")
    for path in sorted(allow):
        if path not in sizes:
            problems.append(f"{path}: is in scripts/ci/file-size-allow.toml but is not a measured file: delete the entry")
    return problems, warnings


def main(argv):
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    top = 0
    it = iter(argv)
    for a in it:
        if a == "--root":
            root = next(it)
        elif a == "--top":
            top = int(next(it))
        else:
            raise SystemExit(__doc__)
    sizes = measure(root)
    problems, warnings = check(sizes, load_allow(root))
    for w in warnings:
        print("warn:", w)
    for n, path in sorted(((n, p) for p, n in sizes.items()), reverse=True)[:top]:
        print(f"{n:6d}  {path}")
    for p in problems:
        print("error:", p, file=sys.stderr)
    over = sum(1 for n in sizes.values() if n > FAIL_AT)
    print(
        f"file-size: {len(sizes)} files, {sum(1 for n in sizes.values() if n > WARN_AT)} over {WARN_AT}, "
        f"{over} over {FAIL_AT}, {len(warnings)} warning(s), {len(problems)} problem(s)"
    )
    return 1 if problems else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
