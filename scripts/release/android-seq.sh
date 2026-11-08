#!/usr/bin/env bash
# The highest seq ever signed into the stable Android manifest, kept outside the release assets.
#
#   android-seq.sh floor  --repo OWNER/REPO            prints that seq (0 when none is recorded)
#   android-seq.sh check  --repo OWNER/REPO --seq N    exits 1 unless N >= the floor
#   android-seq.sh record --repo OWNER/REPO --seq N --sha COMMIT
#
# promote.yml and android-withdraw.yml re-sign the manifest they find on channel-stable. That file
# is a release asset, which any token with contents: write can replace with an older manifest that
# still carries a valid signature (one with fewer withdrawn versions, say). Re-signing it would give
# it a fresh seq, and phones would take it. So each run records the seq it published as a git tag,
# android-stable-seq-<seq>, and refuses to re-sign a manifest whose seq is below the highest such
# tag. The tags are the trust source: a tag ruleset (deploy/runner/RUNBOOK.md) blocks updating or
# deleting them for everyone, so the floor can only rise. A bogus higher tag stops these workflows
# (they fail closed) until a maintainer removes it; it cannot make them sign an older manifest.
set -euo pipefail
shopt -s inherit_errexit

cmd=${1:-}
shift || true
repo="" seq="" sha=""
while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || { echo "android-seq.sh: $1 needs a value" >&2; exit 2; }
    case "$1" in
        --repo) repo=$2 ;;
        --seq) seq=$2 ;;
        --sha) sha=$2 ;;
        *) echo "android-seq.sh: unknown argument $1" >&2; exit 2 ;;
    esac
    shift 2
done
[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || { echo "android-seq.sh: --repo OWNER/REPO is required" >&2; exit 2; }
PREFIX="android-stable-seq-"

floor() {
    local refs max=0 n
    # A failed lookup must stop the run, never read as "no floor".
    refs=$(gh api --paginate "repos/$repo/git/matching-refs/tags/$PREFIX" --jq '.[].ref') ||
        { echo "android-seq.sh: could not list the $PREFIX tags" >&2; return 1; }
    while IFS= read -r ref; do
        [ -n "$ref" ] || continue
        n=${ref#refs/tags/"$PREFIX"}
        # A malformed name is ignored rather than trusted; only whole numbers count.
        [[ "$n" =~ ^[0-9]{1,18}$ ]] || { echo "android-seq.sh: ignoring malformed tag $ref" >&2; continue; }
        n=$((10#$n))
        [ "$n" -gt "$max" ] && max=$n
    done <<< "$refs"
    echo "$max"
}

need_seq() { [[ "$seq" =~ ^[0-9]{1,18}$ ]] || { echo "android-seq.sh: --seq must be a whole number" >&2; exit 2; }; }

case "$cmd" in
    floor) floor ;;
    check)
        need_seq
        f=$(floor) || exit 1
        [ "$((10#$seq))" -ge "$f" ] || { echo "android-seq.sh: manifest seq $seq is older than $f, the newest this channel has published; refusing to sign it again" >&2; exit 1; }
        echo "seq $seq is not older than the published floor $f"
        ;;
    record)
        need_seq
        [[ "$sha" =~ ^[0-9a-f]{40}$ ]] || { echo "android-seq.sh: --sha must be a full commit" >&2; exit 2; }
        gh api "repos/$repo/git/refs" -f ref="refs/tags/$PREFIX$((10#$seq))" -f sha="$sha" --jq .ref
        ;;
    *) echo "usage: android-seq.sh floor|check|record --repo OWNER/REPO [--seq N] [--sha COMMIT]" >&2; exit 2 ;;
esac
