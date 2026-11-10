#!/usr/bin/env bash
# Lists the main-channel releases that may be promoted to stable, newest first:
#   pick-promotable.sh --releases FILE --stable-version V --now EPOCH --soak-hours N
# FILE is `gh release list --json tagName,isPrerelease,publishedAt`. A release qualifies when its
# tag is vX.Y.Z-main.N, it is newer than the stable version (sort -V) and it has been out for at
# least the soak time. Whether its checks passed is the caller's question, not this script's.
set -euo pipefail

releases="" stable="" now="" soak=""
while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || { echo "pick-promotable.sh: missing value for $1" >&2; exit 2; }
    case "$1" in
        --releases) releases=$2 ;;
        --stable-version) stable=$2 ;;
        --now) now=$2 ;;
        --soak-hours) soak=$2 ;;
        *) echo "pick-promotable.sh: unknown argument $1" >&2; exit 2 ;;
    esac
    shift 2
done
[ -n "$releases" ] && [ -n "$stable" ] && [ -n "$now" ] && [ -n "$soak" ] ||
    { echo "usage: pick-promotable.sh --releases FILE --stable-version V --now EPOCH --soak-hours N" >&2; exit 2; }
[[ "$now" =~ ^[0-9]+$ && "$soak" =~ ^[0-9]+$ ]] || { echo "pick-promotable.sh: --now and --soak-hours are whole numbers" >&2; exit 2; }
[[ "$stable" =~ ^[0-9A-Za-z._+-]+$ ]] || { echo "pick-promotable.sh: bad stable version" >&2; exit 2; }

cutoff=$((now - soak * 3600))
jq -r --argjson cutoff "$cutoff" '
    [ .[] | select(.isPrerelease and (.tagName | test("^v[0-9]+\\.[0-9]+\\.[0-9]+-main\\.[0-9]+$")))
          | select((.publishedAt | fromdateiso8601) <= $cutoff) | .tagName ] | .[]' "$releases" |
    while IFS= read -r tag; do
        [ "$(printf '%s\n%s\n' "${tag#v}" "$stable" | sort -V | tail -n1)" = "${tag#v}" ] &&
            [ "${tag#v}" != "$stable" ] && echo "$tag"
    done | sort -V -r
