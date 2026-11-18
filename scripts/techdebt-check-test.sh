#!/usr/bin/env bash
# Tests techdebt-check.py against throwaway folders. No network.
# Run: bash scripts/techdebt-check-test.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fails=0

new_root() { root=$(mktemp -d -p "$t"); mkdir -p "$root/docs/techdebt"; }
check() { python3 -I "$here/techdebt-check.py" --root "$root"; }
ok() { if "$@" > /dev/null 2>&1; then :; else echo "FAIL (should pass): $*" >&2; fails=$((fails + 1)); fi; }
no() { if "$@" > /dev/null 2>&1; then echo "FAIL (should refuse): $*" >&2; fails=$((fails + 1)); fi; }
good() {
    printf '# A thing is wrong\n\n**Reason skipped:** later.\n**Date:** 2026-11-18\n**Where:** `crates/x/src/lib.rs`\n'
}

new_root                                   # no folder content: fine
ok check
new_root; rmdir "$root/docs/techdebt"      # no folder at all: fine
ok check
new_root; good > "$root/docs/techdebt/a-thing.md"; ok check
new_root; printf '# readme\n' > "$root/docs/techdebt/README.md"; ok check

# a facade needs a "Remove when"
new_root; good > "$root/docs/techdebt/facade-x.md"; no check
new_root; { good; printf '**Remove when:** the callers move.\n'; } > "$root/docs/techdebt/facade-x.md"; ok check

# each required part, missing in turn
new_root; good | sed '/Reason skipped/d' > "$root/docs/techdebt/a.md"; no check
new_root; good | sed '/\*\*Where/d' > "$root/docs/techdebt/a.md"; no check
new_root; good | sed '/\*\*Date/d' > "$root/docs/techdebt/a.md"; no check
new_root; good | sed 's/2026-11-18/soon/' > "$root/docs/techdebt/a.md"; no check
new_root; good | sed 's/^# .*/no title here/' > "$root/docs/techdebt/a.md"; no check
new_root; good | sed 's/^\*\*Where:\*\*.*/**Where:**/' > "$root/docs/techdebt/a.md"; no check   # label with no text

# bad names
for bad in 'Bad Name.md' 'UPPER.md' 'thing.txt' 'under_score.md'; do
    new_root; good > "$root/docs/techdebt/$bad"; no check
done

# one bad file among good ones fails the run
new_root; good > "$root/docs/techdebt/ok.md"; printf 'nothing\n' > "$root/docs/techdebt/bad.md"; no check

# the real folder passes
ok python3 -I "$here/techdebt-check.py"

if [ "$fails" -gt 0 ]; then echo "$fails failed" >&2; exit 1; fi
echo "all passed"
