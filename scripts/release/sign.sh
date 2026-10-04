#!/usr/bin/env bash
# Signs each file argument with minisign, writing <file>.minisig.
# The secret key comes from $MINISIGN_KEY (the key file contents), the optional
# password from $MINISIGN_PASSWORD. The key lives only in a tmpfs file with
# mode 600 and is shredded on exit.
set -euo pipefail

[ $# -ge 1 ] || { echo "usage: sign.sh FILE..." >&2; exit 2; }
[ -n "${MINISIGN_KEY:-}" ] || { echo "sign.sh: MINISIGN_KEY is not set" >&2; exit 1; }
command -v minisign >/dev/null || { echo "sign.sh: minisign not installed" >&2; exit 1; }

umask 077
base=${SIGN_TMPFS:-/dev/shm}
[ -d "$base" ] && [ -w "$base" ] || base=${TMPDIR:-/tmp}
dir=$(mktemp -d "$base/minisign.XXXXXX")
key="$dir/key"
cleanup() {
    if command -v shred >/dev/null; then shred -u "$key" 2>/dev/null || true; fi
    rm -rf "$dir"
}
trap cleanup EXIT

printf '%s\n' "$MINISIGN_KEY" > "$key"

for f in "$@"; do
    [ -f "$f" ] || { echo "sign.sh: no such file $f" >&2; exit 1; }
    if [ -n "${MINISIGN_PASSWORD:-}" ]; then
        printf '%s\n' "$MINISIGN_PASSWORD" | minisign -S -s "$key" -m "$f" -x "$f.minisig" -t "hq release $(basename "$f")"
    else
        minisign -S -s "$key" -m "$f" -x "$f.minisig" -t "hq release $(basename "$f")"
    fi
done
