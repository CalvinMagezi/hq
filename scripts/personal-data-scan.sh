#!/usr/bin/env bash
# Fails when a tracked file (or, with --history, any line ever added) matches a
# personal-data pattern.
#
#   personal-data-scan.sh [--history] [EXTRA_DENYLIST_FILE]
#
# Patterns: the in-repo baseline (scripts/personal-data-baseline.txt), plus an
# optional extra denylist (extended regexes, one per line) from the file
# argument or $AGENT_HQ_DENYLIST, plus the PERSONAL_DENYLIST environment
# variable. The extra lists live outside the repo so they never ship. Paths in
# .personal-data-allow (one per line) are skipped in the working-tree scan.
# Output names files and line numbers only, never the matched text. Any tool
# error (grep status 2 or above, sed or git failure) fails the scan.
set -euo pipefail
export LC_ALL=C

BASELINE="${PERSONAL_BASELINE:-scripts/personal-data-baseline.txt}"
ALLOW=".personal-data-allow"
history=0
if [ "${1:-}" = "--history" ]; then history=1; shift; fi
EXTRA_FILE="${1:-${AGENT_HQ_DENYLIST:-}}"

die() { echo "personal-data-scan: $*" >&2; exit 2; }
[ -s "$BASELINE" ] || die "baseline $BASELINE missing"

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
deny="$work/deny"

# Patterns only: skip comments, blanks and the "!" benign lines.
patterns() { grep -vE '^(#|!|[[:space:]]*$)' "$1" || [ $? -eq 1 ]; }
{
    patterns "$BASELINE"
    if [ -n "$EXTRA_FILE" ] && [ -s "$EXTRA_FILE" ]; then patterns "$EXTRA_FILE"; fi
    if [ -n "${PERSONAL_DENYLIST:-}" ]; then printf '%s\n' "$PERSONAL_DENYLIST" | patterns /dev/stdin; fi
} > "$deny"
[ -s "$deny" ] || die "no patterns loaded"

benign_re=$({ grep -E '^!' "$BASELINE" || [ $? -eq 1 ]; } | sed 's/^!//' | paste -sd'|' -)
[ -n "$benign_re" ] || benign_re='^$'
# A control character as the sed delimiter, so patterns may contain any printable character.
d=$'\001'
strip_benign() { sed -E "s${d}(${benign_re})${d}${d}g"; }

if [ "$history" -eq 1 ]; then
    # Added lines only, attributed to the commit and file they were added in.
    hits=$(git log --all --no-merges -p --no-color --format='@@commit %h' -U0 | strip_benign |
        awk -v denyfile="$deny" '
            BEGIN { while ((getline l < denyfile) > 0) pats[++n] = l }
            /^@@commit / { commit = $2; next }
            /^\+\+\+ b\// { file = substr($0, 7); next }
            /^\+[^+]/ || /^\+$/ {
                for (i = 1; i <= n; i++) if ($0 ~ pats[i]) { seen[file " (commit " commit ")"] = 1; break }
            }
            END { for (k in seen) print k }' | sort)
    if [ -n "$hits" ]; then
        printf 'personal data added in history:\n%s\n' "$hits"
        echo "personal-data-scan: history matches" >&2
        exit 1
    fi
    echo "personal-data-scan: history clean"
    exit 0
fi

hits=0
while IFS= read -r file; do
    if [ -f "$ALLOW" ] && grep -qxF -- "$file" "$ALLOW"; then continue; fi
    [ -f "$file" ] || continue
    cleaned=$(strip_benign < "$file")
    rc=0
    lines=$(printf '%s\n' "$cleaned" | grep -nIEf "$deny" | cut -d: -f1 | paste -sd, -) || rc=$?
    # With pipefail the pipeline status is grep's: 0 = matches, 1 = none, 2+ = error.
    case "$rc" in
        0) echo "personal data in $file (lines $lines)"; hits=$((hits + 1)) ;;
        1) ;;
        *) die "grep failed on $file (status $rc)" ;;
    esac
done < <(git ls-files -- . ':!Cargo.lock' ':!*.lock' ':!*.png' ':!*.jpg' ':!*.ico')

if [ "$hits" -gt 0 ]; then
    echo "personal-data-scan: $hits file(s) match" >&2
    exit 1
fi
echo "personal-data-scan: clean"
