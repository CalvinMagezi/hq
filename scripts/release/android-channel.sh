#!/usr/bin/env bash
# Writes the Android update manifest the in-app updater reads (signed afterwards with sign.sh):
#   android-channel.sh --apk FILE --version V --tag TAG --repo OWNER/REPO --out FILE --seq N
#                      [--rollout 0-100] [--urgent true|false] [--withdrawn CODE,CODE]
#                      [--notes LANG --note LINE [--note LINE [--note LINE]]]
# The APK is checked first (release key, app id), then described by what it says about itself, so the
# manifest cannot disagree with the file it names. Field shapes: docs/plans/ux-run/resolutions.md D-3.
# --seq must rise with every signed manifest (release, promote, withdraw use the signing time).
set -euo pipefail

MAX_NOTES=3
MAX_NOTE_CHARS=120

apk="" version="" tag="" repo="" out="" seq="" rollout=100 urgent=false withdrawn="" notes_lang="en"
notes=()
while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || { echo "android-channel.sh: missing value for $1" >&2; exit 2; }
    case "$1" in
        --apk) apk=$2 ;;
        --version) version=$2 ;;
        --tag) tag=$2 ;;
        --repo) repo=$2 ;;
        --out) out=$2 ;;
        --seq) seq=$2 ;;
        --rollout) rollout=$2 ;;
        --urgent) urgent=$2 ;;
        --withdrawn) withdrawn=$2 ;;
        --notes) notes_lang=$2 ;;
        --note) notes+=("$2") ;;
        *) echo "android-channel.sh: unknown argument $1" >&2; exit 2 ;;
    esac
    shift 2
done
[ -f "$apk" ] && [ -n "$version" ] && [ -n "$tag" ] && [ -n "$repo" ] && [ -n "$out" ] && [ -n "$seq" ] ||
    { echo "usage: android-channel.sh --apk FILE --version V --tag TAG --repo OWNER/REPO --out FILE --seq N [--rollout N] [--urgent B] [--withdrawn CODES] [--notes LANG --note LINE...]" >&2; exit 2; }
[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || { echo "android-channel.sh: bad repo" >&2; exit 2; }
[[ "$tag" =~ ^v[0-9A-Za-z._-]+$ ]] || { echo "android-channel.sh: bad tag" >&2; exit 2; }
[[ "$seq" =~ ^[0-9]+$ ]] || { echo "android-channel.sh: seq must be a whole number" >&2; exit 2; }
[[ "$rollout" =~ ^[0-9]+$ ]] && [ "$rollout" -le 100 ] || { echo "android-channel.sh: rollout is 0 to 100" >&2; exit 2; }
[[ "$urgent" =~ ^(true|false)$ ]] || { echo "android-channel.sh: urgent is true or false" >&2; exit 2; }
[[ -z "$withdrawn" || "$withdrawn" =~ ^[0-9]+(,[0-9]+)*$ ]] || { echo "android-channel.sh: withdrawn is versionCodes separated by commas" >&2; exit 2; }
[[ "$notes_lang" =~ ^[a-z]{2,3}(-[A-Za-z0-9]+)?$ ]] || { echo "android-channel.sh: bad notes language" >&2; exit 2; }
[ "${#notes[@]}" -le "$MAX_NOTES" ] || { echo "android-channel.sh: at most $MAX_NOTES notes" >&2; exit 2; }
for line in ${notes[@]+"${notes[@]}"}; do
    [ "${#line}" -le "$MAX_NOTE_CHARS" ] || { echo "android-channel.sh: a note is longer than $MAX_NOTE_CHARS characters" >&2; exit 2; }
done

here=$(cd "$(dirname "$0")" && pwd)
read -r code min < <(bash "$here/android-check.sh" "$apk")

if command -v sha256sum >/dev/null; then sha=$(sha256sum "$apk" | cut -d' ' -f1); else sha=$(shasum -a 256 "$apk" | cut -d' ' -f1); fi
size=$(wc -c < "$apk" | tr -d ' ')
base=${RELEASE_BASE_URL:-https://github.com}
issued=$(date -u +%Y-%m-%dT%H:%M:%SZ)
withdrawn_json=$(jq -cn --arg w "$withdrawn" '$w | if . == "" then [] else split(",") | map(tonumber) end')
notes_json=$(jq -cn --arg lang "$notes_lang" '{($lang): $ARGS.positional}' --args ${notes[@]+"${notes[@]}"})

jq -n --arg version "$version" --argjson code "$code" --arg url "$base/$repo/releases/download/$tag/agent-hq-android.apk" \
    --arg sha "$sha" --argjson min "$min" --argjson size "$size" --argjson notes "$notes_json" --argjson seq "$seq" \
    --arg issued "$issued" --argjson rollout "$rollout" --argjson urgent "$urgent" --argjson withdrawn "$withdrawn_json" \
    '{version:$version, versionCode:$code, url:$url, sha256:$sha, minSdk:$min, size:$size, notes:$notes,
      seq:$seq, issued_at:$issued, rollout:$rollout, urgent:$urgent, withdrawn:$withdrawn}' > "$out"
