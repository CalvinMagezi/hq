#!/bin/sh
# Tests next-migration-number.sh against throwaway folders. No network.
# Run: sh scripts/next-migration-number-test.sh
# An optional argument is passed through as the folder to check against a
# real migrations directory: it only has to print three digits.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fails=0

expect() { # expect <want> <dir>
    got=$(sh "$here/next-migration-number.sh" "$2" 2>&1 || true)
    if [ "$got" != "$1" ]; then
        echo "FAIL: wanted '$1', got '$got' ($2)" >&2
        fails=$((fails + 1))
    fi
}

mkdir "$t/empty"
expect 001 "$t/empty"

mkdir "$t/a"
: > "$t/a/001_x.sql"
: > "$t/a/009_y.sql"
expect 010 "$t/a"

mkdir "$t/b"
: > "$t/b/008_x.sql"
: > "$t/b/099_y.sql"
: > "$t/b/README.md"
expect 100 "$t/b"

mkdir "$t/c"
: > "$t/c/139_x.sql"
: > "$t/c/012_y.sql"
: > "$t/c/12_ignored.sql"
expect 140 "$t/c"

if [ $# -gt 0 ]; then
    got=$(sh "$here/next-migration-number.sh" "$1")
    case $got in
        [0-9][0-9][0-9]) ;;
        *) echo "FAIL: '$got' is not three digits ($1)" >&2; fails=$((fails + 1)) ;;
    esac
fi

if [ "$fails" -ne 0 ]; then
    echo "$fails failure(s)" >&2
    exit 1
fi
echo "ok"
