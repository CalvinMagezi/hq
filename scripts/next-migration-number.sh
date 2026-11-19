#!/bin/sh
# Prints the next free SQL migration number: the highest NNN_ prefix in the
# migrations folder plus one, zero-padded to three digits.
# Usage: scripts/next-migration-number.sh [dir]   (default crates/hq-db/sql)
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
dir=${1:-"$root/crates/hq-db/sql"}
[ -d "$dir" ] || { echo "next-migration-number: no such directory: $dir" >&2; exit 1; }
max=0
for f in "$dir"/[0-9][0-9][0-9]_*.sql; do
    [ -e "$f" ] || continue
    n=$(basename "$f" | cut -c1-3 | sed 's/^0*//')
    n=${n:-0}
    if [ "$n" -gt "$max" ]; then max=$n; fi
done
printf '%03d\n' $((max + 1))
