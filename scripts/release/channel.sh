#!/usr/bin/env bash
# Writes a channel pointer:
#   channel.sh --channel NAME --repo OWNER/REPO --manifest FILE --seq N --out FILE
# seq must increase strictly within a channel (workflows use run_number*1000+attempt);
# issued_at is the signing time. Both let an updater refuse a replayed older pointer.
# The pointer names the release by version and pins the exact manifest bytes by
# sha256 (format: docs/UPDATE_SYSTEM.md, "Channel pointers").
set -euo pipefail

channel="" repo="" manifest="" out="" seq=""
while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || { echo "channel.sh: missing value for $1" >&2; exit 2; }
    case "$1" in
        --channel) channel=$2 ;;
        --repo) repo=$2 ;;
        --manifest) manifest=$2 ;;
        --out) out=$2 ;;
        --seq) seq=$2 ;;
        *) echo "channel.sh: unknown argument $1" >&2; exit 2 ;;
    esac
    shift 2
done
[ -n "$channel" ] && [ -n "$repo" ] && [ -n "$manifest" ] && [ -n "$out" ] && [ -n "$seq" ] ||
    { echo "usage: channel.sh --channel NAME --repo OWNER/REPO --manifest FILE --seq N --out FILE" >&2; exit 2; }
[[ "$seq" =~ ^[0-9]+$ ]] || { echo "channel.sh: seq must be an integer" >&2; exit 2; }
[[ "$channel" =~ ^[a-z][a-z0-9-]*$ ]] || { echo "channel.sh: bad channel name" >&2; exit 2; }
[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || { echo "channel.sh: bad repo" >&2; exit 2; }

if command -v sha256sum >/dev/null; then msha=$(sha256sum "$manifest" | cut -d' ' -f1); else msha=$(shasum -a 256 "$manifest" | cut -d' ' -f1); fi

base=${RELEASE_BASE_URL:-https://github.com}

jq -n --arg base "$base" --arg channel "$channel" --arg repo "$repo" --arg msha "$msha" --argjson seq "$seq" --arg issued "$(date -u +%Y-%m-%dT%H:%M:%SZ)" --slurpfile m "$manifest" \
    '{schema:1, channel:$channel, version:$m[0].version,
      manifest_url:($base + "/" + $repo + "/releases/download/v" + $m[0].version + "/manifest.json"),
      manifest_sha256:$msha, issued_at:$issued, seq:$seq}' > "$out"
