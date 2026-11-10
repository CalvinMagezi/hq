#!/usr/bin/env bash
# Tests pick-promotable.sh against a fixed release list. No network.
# Run: bash scripts/release/pick-promotable-test.sh
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT

cat > "$t/releases.json" <<'JSON'
[
 {"tagName":"v0.9.1-main.419","isPrerelease":true,"publishedAt":"2026-11-10T09:33:23Z"},
 {"tagName":"v0.9.1-main.418","isPrerelease":true,"publishedAt":"2026-11-09T17:24:38Z"},
 {"tagName":"v0.9.1-main.300","isPrerelease":true,"publishedAt":"2026-11-01T05:00:00Z"},
 {"tagName":"v0.9.1-main.99","isPrerelease":true,"publishedAt":"2026-10-01T05:00:00Z"},
 {"tagName":"channel-stable","isPrerelease":false,"publishedAt":"2026-11-01T05:46:00Z"},
 {"tagName":"v0.9.1-main.420","isPrerelease":false,"publishedAt":"2026-11-09T05:46:00Z"}
]
JSON
now=$(date -u -d 2026-11-10T14:00:00Z +%s 2>/dev/null || date -u -j -f %Y-%m-%dT%H:%M:%SZ 2026-11-10T14:00:00Z +%s)

pick() { bash "$here/pick-promotable.sh" --releases "$t/releases.json" --stable-version "$1" --now "$now" --soak-hours "$2"; }
check() { [ "$2" = "$3" ] || { echo "FAIL $1: got '$2', want '$3'" >&2; exit 1; }; }

check "a short soak lets the newest through, newest first" "$(pick 0.9.1-main.300 4 | tr '\n' ' ')" "v0.9.1-main.419 v0.9.1-main.418 "
check "soak holds back a fresh build" "$(pick 0.9.1-main.300 20 | tr '\n' ' ')" "v0.9.1-main.418 "
check "nothing newer than stable" "$(pick 0.9.1-main.419 4)" ""
check "sorted as versions, not text" "$(pick 0.9.1-main.99 4 | head -n1)" "v0.9.1-main.419"
check "channel tags and non-prereleases never qualify" "$(pick 0.9.1-main.1 1 | grep -c -E 'channel|main.420' || true)" "0"
echo "pick-promotable: ok"
