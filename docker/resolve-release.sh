#!/usr/bin/env bash
# Picks the release to publish as an image, verifies it, and decides its tags.
#
#   resolve-release.sh --repo OWNER/REPO --dir DIR --source dispatch --tag TAG
#   resolve-release.sh --repo OWNER/REPO --dir DIR --source release --run-number N
#   resolve-release.sh --repo OWNER/REPO --dir DIR --source promote
#   [--pubkey FILE]   default release/minisign.pub
#
# source=release maps a Release workflow run to its tag (v<base>-main.<run number>);
# source=promote reads the signed channel-stable pointer. Release assets land in DIR
# only after verify.sh accepted the manifest signature and every checksum. Prints
# key=value lines for $GITHUB_OUTPUT: tag, version, revision, created, extra_tags.
#
# Tags: <version> always; `main` when the version is the signed channel-main pointer;
# `latest` and `stable` when it is the signed channel-stable pointer. Older releases
# therefore never move a moving tag backwards.
# Needs: gh (GH_TOKEN), minisign, jq, and scripts/release/verify.sh.
set -euo pipefail

repo="" dir="" source="" tag="" run_number="" pubkey="release/minisign.pub"
while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || { echo "resolve-release.sh: missing value for $1" >&2; exit 2; }
    case "$1" in
        --repo) repo=$2 ;;
        --dir) dir=$2 ;;
        --source) source=$2 ;;
        --tag) tag=$2 ;;
        --run-number) run_number=$2 ;;
        --pubkey) pubkey=$2 ;;
        *) echo "resolve-release.sh: unknown argument $1" >&2; exit 2 ;;
    esac
    shift 2
done

fail() { echo "resolve-release.sh: $*" >&2; exit 1; }
[ -n "$repo" ] && [ -n "$dir" ] && [ -n "$source" ] || fail "usage: see the header of this script"
[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || fail "bad repo '$repo'"
TAG_RE='^v[0-9]+\.[0-9]+\.[0-9]+(-main\.[0-9]+)?$'
VERSION_RE='^[0-9]+\.[0-9]+\.[0-9]+(-main\.[0-9]+)?$'
here=$(cd "$(dirname "$0")" && pwd)
verify="$here/../scripts/release/verify.sh"
[ -f "$verify" ] || fail "missing $verify"

# Version named by a channel pointer, after checking its signature. Empty when the
# channel has never been published, which only means there is nothing to move.
pointer_version() {
    local channel=$1 work
    work=$(mktemp -d)
    if ! gh release download "channel-$channel" --repo "$repo" --dir "$work" \
            --pattern "channel-$channel.json" --pattern "channel-$channel.json.minisig" 2> /dev/null; then
        rm -rf "$work"
        return 0
    fi
    minisign -V -p "$pubkey" -m "$work/channel-$channel.json" -x "$work/channel-$channel.json.minisig" -q ||
        fail "channel-$channel pointer signature does not verify"
    [ "$(jq -r .channel "$work/channel-$channel.json")" = "$channel" ] || fail "channel-$channel pointer names another channel"
    jq -r .version "$work/channel-$channel.json"
    rm -rf "$work"
}

main_version=$(pointer_version main)
stable_version=$(pointer_version stable)
for v in "$main_version" "$stable_version"; do
    [ -z "$v" ] || [[ "$v" =~ $VERSION_RE ]] || fail "pointer carries a bad version '$v'"
done

case "$source" in
    dispatch) [ -n "$tag" ] || fail "--tag is required for source=dispatch" ;;
    release)
        [[ "$run_number" =~ ^[0-9]+$ ]] || fail "--run-number is required for source=release"
        tag=$(gh release list --repo "$repo" --limit 100 --json tagName \
            --jq "[.[].tagName | select(endswith(\"-main.$run_number\"))][0] // empty")
        [ -n "$tag" ] || fail "no release tag ends with -main.$run_number" ;;
    promote)
        [ -n "$stable_version" ] || fail "no channel-stable pointer to publish"
        tag="v$stable_version" ;;
    *) fail "unknown source '$source'" ;;
esac
[[ "$tag" =~ $TAG_RE ]] || fail "bad tag '$tag'"
version=${tag#v}

rm -rf "$dir"
mkdir -p "$dir"
gh release download "$tag" --repo "$repo" --dir "$dir" --pattern '*'
bash "$verify" --dir "$dir" --pubkey "$pubkey" >&2

manifest="$dir/manifest.json"
[ "$(jq -r .version "$manifest")" = "$version" ] || fail "manifest version does not match $tag"
[ "$(jq -r .channel "$manifest")" = "main" ] || fail "$tag was not built for the main channel"
revision=$(jq -r .git_sha "$manifest")
created=$(jq -r .built_at "$manifest")
[[ "$revision" =~ ^[0-9a-f]{40}$ ]] || fail "bad git_sha in the manifest"
[[ "$created" =~ ^[0-9T:Z-]+$ ]] || fail "bad built_at in the manifest"

extra=()
[ "$version" != "$main_version" ] || extra+=(main)
[ "$version" != "$stable_version" ] || extra+=(latest stable)

echo "tag=$tag"
echo "version=$version"
echo "revision=$revision"
echo "created=$created"
echo "extra_tags=${extra[*]:-}"
