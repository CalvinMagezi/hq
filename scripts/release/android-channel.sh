#!/usr/bin/env bash
# Writes the Android update manifest the in-app updater reads (signed afterwards with sign.sh):
#   android-channel.sh --apk FILE --version V --tag TAG --repo OWNER/REPO --out FILE
# The APK is checked first (release key, app id), then described by what it says about itself, so the
# manifest cannot disagree with the file it names.
set -euo pipefail

apk="" version="" tag="" repo="" out=""
while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || { echo "android-channel.sh: missing value for $1" >&2; exit 2; }
    case "$1" in
        --apk) apk=$2 ;;
        --version) version=$2 ;;
        --tag) tag=$2 ;;
        --repo) repo=$2 ;;
        --out) out=$2 ;;
        *) echo "android-channel.sh: unknown argument $1" >&2; exit 2 ;;
    esac
    shift 2
done
[ -f "$apk" ] && [ -n "$version" ] && [ -n "$tag" ] && [ -n "$repo" ] && [ -n "$out" ] ||
    { echo "usage: android-channel.sh --apk FILE --version V --tag TAG --repo OWNER/REPO --out FILE" >&2; exit 2; }
[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || { echo "android-channel.sh: bad repo" >&2; exit 2; }
[[ "$tag" =~ ^v[0-9A-Za-z._-]+$ ]] || { echo "android-channel.sh: bad tag" >&2; exit 2; }

here=$(cd "$(dirname "$0")" && pwd)
read -r code min < <(bash "$here/android-check.sh" "$apk")

if command -v sha256sum >/dev/null; then sha=$(sha256sum "$apk" | cut -d' ' -f1); else sha=$(shasum -a 256 "$apk" | cut -d' ' -f1); fi
base=${RELEASE_BASE_URL:-https://github.com}

jq -n --arg version "$version" --argjson code "$code" --arg url "$base/$repo/releases/download/$tag/agent-hq-android.apk" \
    --arg sha "$sha" --argjson min "$min" \
    '{version:$version, versionCode:$code, url:$url, sha256:$sha, minSdk:$min}' > "$out"
