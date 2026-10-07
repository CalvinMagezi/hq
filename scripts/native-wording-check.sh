#!/usr/bin/env bash
# Keeps the word "herdr" from spreading while HQ moves to its built-in agent
# host. The word is true only where the herdr CLI adapter, provenance and history
# live; elsewhere HQ says "agent host".
#
#   native-wording-check.sh            compare against the baseline, fail on growth
#   native-wording-check.sh --update   rewrite the baseline from the working tree
#
# The baseline (scripts/native-wording-baseline.txt) lists "<count> <path>" for
# every tracked file that mentions the word. A file may mention it less, never
# more, and a file that is not in the baseline may not mention it at all.
# Provenance and history files are exempt (ALLOWED below).
set -euo pipefail
export LC_ALL=C

BASELINE="${NATIVE_WORDING_BASELINE:-scripts/native-wording-baseline.txt}"
ALLOWED='^(NOTICE|CHANGELOG\.md|TECHDEBT\.md|docs/provenance/|docs/HERDR_HARNESS\.md|docs/FLEET_HARNESS\.md|docs/HERMES_HARNESS\.md|docs/N8N_HARNESS\.md|scripts/native-wording-)'

counts() {
  git ls-files -z | xargs -0 grep -I -i -c 'herdr' 2>/dev/null \
    | awk -F: '$NF > 0 { n = $NF; sub(/:[0-9]+$/, ""); print n, $0 }' \
    | grep -Ev " ${ALLOWED#^}" \
    | grep -Ev "^[0-9]+ ${ALLOWED#^}" \
    | sort -k2
}

if [ "${1:-}" = "--update" ]; then
  counts > "$BASELINE"
  echo "native-wording-check: wrote $(wc -l < "$BASELINE" | tr -d ' ') files to $BASELINE"
  exit 0
fi

[ -s "$BASELINE" ] || { echo "native-wording-check: baseline $BASELINE missing (run with --update)" >&2; exit 2; }

fail=0
while read -r n path; do
  allowed=$(awk -v p="$path" '$2 == p { print $1 }' "$BASELINE")
  if [ -z "$allowed" ]; then
    echo "native-wording-check: $path mentions herdr ($n times) and is not in the baseline; say \"agent host\" instead" >&2
    fail=1
  elif [ "$n" -gt "$allowed" ]; then
    echo "native-wording-check: $path mentions herdr $n times, baseline allows $allowed" >&2
    fail=1
  fi
done < <(counts)
[ "$fail" = 0 ] && echo "native-wording-check: ok"
exit "$fail"
