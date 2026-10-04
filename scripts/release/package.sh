#!/usr/bin/env bash
# Builds the release artifacts from a compiled hq binary and a web build.
#
#   package.sh --version V --git-sha SHA --channel C --bin PATH --web-dist DIR --out DIR
#              [--min-updater-version X] [--requires-db-snapshot true|false]
#
# Writes to --out: hq-<V>-linux-x86_64.tar.gz, hq-web-<V>.tar.gz, SHA256SUMS,
# manifest.json. Signing is a separate step (sign.sh). Format: docs/UPDATE_SYSTEM.md.
# --min-updater-version defaults to the floor in release/min-updater-version (override the
# path with MIN_UPDATER_FILE). Never derive it from --version: raising it strands older updaters.
set -euo pipefail

SCHEMA=1
version="" git_sha="" channel="" bin="" web_dist="" out=""
min_updater=""
requires_snapshot="false"

die() { echo "package.sh: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || die "missing value for $1"
    case "$1" in
        --version) version=$2 ;;
        --git-sha) git_sha=$2 ;;
        --channel) channel=$2 ;;
        --bin) bin=$2 ;;
        --web-dist) web_dist=$2 ;;
        --out) out=$2 ;;
        --min-updater-version) min_updater=$2 ;;
        --requires-db-snapshot) requires_snapshot=$2 ;;
        *) die "unknown argument $1" ;;
    esac
    shift 2
done

[ -n "$version" ] && [ -n "$git_sha" ] && [ -n "$channel" ] && [ -n "$bin" ] && [ -n "$web_dist" ] && [ -n "$out" ] ||
    die "usage: package.sh --version V --git-sha SHA --channel C --bin PATH --web-dist DIR --out DIR"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || die "version '$version' is not semver"
[[ "$git_sha" =~ ^[0-9a-f]{40}$ ]] || die "git sha must be 40 lowercase hex characters"
[[ "$channel" =~ ^[a-z][a-z0-9-]*$ ]] || die "channel '$channel' must be lowercase [a-z0-9-]"
[[ "$requires_snapshot" =~ ^(true|false)$ ]] || die "--requires-db-snapshot must be true or false"
[ -f "$bin" ] || die "binary not found: $bin"
[ -f "$web_dist/index.html" ] || die "web dist has no index.html: $web_dist"
command -v jq >/dev/null || die "jq is required"
if [ -z "$min_updater" ]; then
    floor_file=${MIN_UPDATER_FILE:-$(cd "$(dirname "$0")/../.." && pwd)/release/min-updater-version}
    [ -f "$floor_file" ] || die "no --min-updater-version and no floor file $floor_file"
    min_updater=$(tr -d '[:space:]' < "$floor_file")
fi
[[ "$min_updater" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "min updater version '$min_updater' is not X.Y.Z"

sha256() {
    if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi
}
size() { wc -c < "$1" | tr -d ' '; }

mkdir -p "$out"
out=$(cd "$out" && pwd)
hq_tar="hq-${version}-linux-x86_64.tar.gz"
web_tar="hq-web-${version}.tar.gz"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT
install -m 755 "$bin" "$stage/hq"

# Fixed owner, sorted names and a zeroed mtime keep the tarball reproducible for a given input.
tar_flags=(--sort=name --owner=0 --group=0 --numeric-owner --mtime=@0)
if ! tar --version 2>/dev/null | grep -q GNU; then tar_flags=(--uid 0 --gid 0); fi
(cd "$stage" && tar "${tar_flags[@]}" -czf "$out/$hq_tar" hq)
(cd "$web_dist" && tar "${tar_flags[@]}" -czf "$out/$web_tar" .)

(cd "$out" && for f in "$hq_tar" "$web_tar"; do printf '%s  %s\n' "$(sha256 "$f")" "$f"; done > SHA256SUMS)

built_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
jq -n \
    --argjson schema "$SCHEMA" --arg version "$version" --arg git_sha "$git_sha" \
    --arg channel "$channel" --arg built_at "$built_at" --arg min "$min_updater" \
    --argjson snap "$requires_snapshot" \
    --arg n1 "$hq_tar" --arg h1 "$(sha256 "$out/$hq_tar")" --argjson s1 "$(size "$out/$hq_tar")" \
    --arg n2 "$web_tar" --arg h2 "$(sha256 "$out/$web_tar")" --argjson s2 "$(size "$out/$web_tar")" \
    '{schema:$schema, version:$version, git_sha:$git_sha, channel:$channel, built_at:$built_at,
      min_updater_version:$min, requires_db_snapshot:$snap,
      artifacts:[{name:$n1,sha256:$h1,size:$s1},{name:$n2,sha256:$h2,size:$s2}]}' > "$out/manifest.json"

echo "package.sh: wrote $hq_tar $web_tar SHA256SUMS manifest.json to $out"
