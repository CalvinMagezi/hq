#!/usr/bin/env bash
# The CLAUDE.md check passes on the repository and fails on each thing it guards.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 -I scripts/claude-md-check.py >/dev/null

python3 -I - <<'PY'
import importlib.util, sys
spec = importlib.util.spec_from_file_location("c", "scripts/claude-md-check.py")
c = importlib.util.module_from_spec(spec); spec.loader.exec_module(c)
ok = lambda f: True
assert c.problems("x", 100, {"a/CLAUDE.md"}, {"a/CLAUDE.md"}, ok) == []
assert any("over the" in p for p in c.problems("x", c.BUDGET + 1, {"a"}, {"a"}, ok))
assert any("does not exist" in p for p in c.problems("x", 1, {"a"}, {"a"}, lambda f: False))
assert any("does not list" in p for p in c.problems("x", 1, {"a"}, {"a", "b/CLAUDE.md"}, ok))
assert any("no \"Where things live\"" in p for p in c.problems("x", 1, set(), set(), ok))
PY
echo "claude-md-check-test: all passed"
