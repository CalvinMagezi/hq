#!/usr/bin/env bash
# Verifies a release: manifest signature, artifact checksums and sizes.
#
#   verify.sh --repo OWNER/REPO --tag TAG [--pubkey FILE] [--dir DIR]
#   verify.sh --dir DIR [--pubkey FILE]        # already-downloaded release assets
#
# With --repo and --tag the assets are downloaded into --dir (a temp dir by
# default) over HTTPS. Exit status is 0 only if everything verifies.
set -euo pipefail

repo="" tag="" dir="" pubkey="release/minisign.pub"
while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || { echo "verify.sh: missing value for $1" >&2; exit 2; }
    case "$1" in
        --repo) repo=$2 ;;
        --tag) tag=$2 ;;
        --dir) dir=$2 ;;
        --pubkey) pubkey=$2 ;;
        *) echo "verify.sh: unknown argument $1" >&2; exit 2 ;;
    esac
    shift 2
done

fail() { echo "verify.sh: $*" >&2; exit 1; }
command -v minisign >/dev/null || fail "minisign not installed"
command -v jq >/dev/null || fail "jq not installed"
[ -f "$pubkey" ] || fail "public key not found: $pubkey"

sha256() {
    if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

name_ok() { [[ "$1" =~ ^[A-Za-z0-9][A-Za-z0-9._+-]*$ ]]; }
get() { curl --fail --silent --show-error --location --proto '=https' --output "$2" "${@:3}" "$1"; }

if [ -n "$repo" ] || [ -n "$tag" ]; then
    [ -n "$repo" ] && [ -n "$tag" ] || fail "--repo and --tag go together"
    [ -n "$dir" ] || dir=$(mktemp -d)
    mkdir -p "$dir"
    base="https://github.com/$repo/releases/download/$tag"
    get "$base/manifest.json" "$dir/manifest.json"
    get "$base/manifest.json.minisig" "$dir/manifest.json.minisig"
    # Nothing the manifest names is fetched until its signature verifies.
    minisign -V -p "$pubkey" -m "$dir/manifest.json" -x "$dir/manifest.json.minisig" -q ||
        fail "manifest signature does not verify"
    get "$base/SHA256SUMS" "$dir/SHA256SUMS"
    while IFS=$'\t' read -r name size; do
        name_ok "$name" || fail "unsafe artifact name '$name'"
        [[ "$size" =~ ^[0-9]+$ ]] || fail "bad size for $name"
        get "$base/$name" "$dir/$name" --max-filesize "$size"
    done < <(jq -r '.artifacts[] | [.name, .size] | @tsv' "$dir/manifest.json")
fi
[ -n "$dir" ] || fail "give --dir, or --repo and --tag"

minisign -V -p "$pubkey" -m "$dir/manifest.json" -x "$dir/manifest.json.minisig" -q ||
    fail "manifest signature does not verify"

count=$(jq '.artifacts | length' "$dir/manifest.json")
[ "$count" -ge 1 ] || fail "manifest lists no artifacts"
for i in $(seq 0 $((count - 1))); do
    name=$(jq -r ".artifacts[$i].name" "$dir/manifest.json")
    want_sha=$(jq -r ".artifacts[$i].sha256" "$dir/manifest.json")
    want_size=$(jq -r ".artifacts[$i].size" "$dir/manifest.json")
    name_ok "$name" || fail "unsafe artifact name '$name'"
    [ -f "$dir/$name" ] || fail "missing artifact $name"
    [ "$(sha256 "$dir/$name")" = "$want_sha" ] || fail "sha256 mismatch for $name"
    [ "$(wc -c < "$dir/$name" | tr -d ' ')" = "$want_size" ] || fail "size mismatch for $name"
    grep -qxF "$want_sha  $name" "$dir/SHA256SUMS" || fail "$name is not listed in SHA256SUMS"
done

echo "verify.sh: OK $(jq -r '.version + " (" + .channel + ", " + .git_sha[0:12] + ")"' "$dir/manifest.json")"
