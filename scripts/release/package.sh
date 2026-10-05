#!/usr/bin/env bash
# Builds the release artifacts from a compiled hq binary and a web build.
#
#   package.sh --version V --git-sha SHA --channel C --bin PLATFORM=PATH [--bin ...] \
#              --web-dist DIR --out DIR \
#              [--min-updater-version X] [--requires-db-snapshot true|false]
#
# PLATFORM is <os>-<arch> with os in linux|darwin and arch in x86_64|aarch64. --bin is
# repeatable, and linux-x86_64 is mandatory: updaters released before multi-platform
# artifacts only look for that name.
# Writes to --out: hq-<V>-<platform>.tar.gz per --bin, hq-web-<V>.tar.gz, SHA256SUMS,
# manifest.json. Signing is a separate step (sign.sh). Format: docs/UPDATE_SYSTEM.md.
# --min-updater-version defaults to the floor in release/min-updater-version (override the
# path with MIN_UPDATER_FILE). Never derive it from --version: raising it strands older updaters.
set -euo pipefail

SCHEMA=1
version="" git_sha="" channel="" web_dist="" out=""
platforms=() bin_paths=()
min_updater=""
requires_snapshot="false"

die() { echo "package.sh: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || die "missing value for $1"
    case "$1" in
        --version) version=$2 ;;
        --git-sha) git_sha=$2 ;;
        --channel) channel=$2 ;;
        --bin)
            [[ "$2" =~ ^(linux|darwin)-(x86_64|aarch64)=.+$ ]] || die "--bin wants PLATFORM=PATH (linux|darwin)-(x86_64|aarch64), got '$2'"
            case " ${platforms[*]:-} " in *" ${2%%=*} "*) die "platform ${2%%=*} given twice" ;; esac
            platforms+=("${2%%=*}")
            bin_paths+=("${2#*=}") ;;
        --web-dist) web_dist=$2 ;;
        --out) out=$2 ;;
        --min-updater-version) min_updater=$2 ;;
        --requires-db-snapshot) requires_snapshot=$2 ;;
        *) die "unknown argument $1" ;;
    esac
    shift 2
done

[ -n "$version" ] && [ -n "$git_sha" ] && [ -n "$channel" ] && [ "${#platforms[@]}" -gt 0 ] && [ -n "$web_dist" ] && [ -n "$out" ] ||
    die "usage: package.sh --version V --git-sha SHA --channel C --bin PLATFORM=PATH --web-dist DIR --out DIR"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || die "version '$version' is not semver"
[[ "$git_sha" =~ ^[0-9a-f]{40}$ ]] || die "git sha must be 40 lowercase hex characters"
[[ "$channel" =~ ^[a-z][a-z0-9-]*$ ]] || die "channel '$channel' must be lowercase [a-z0-9-]"
[[ "$requires_snapshot" =~ ^(true|false)$ ]] || die "--requires-db-snapshot must be true or false"
case " ${platforms[*]} " in *" linux-x86_64 "*) ;; *) die "--bin linux-x86_64=PATH is required" ;; esac
for p in "${bin_paths[@]}"; do [ -f "$p" ] || die "binary not found: $p"; done

# A label that disagrees with the file would ship, say, an arm64 build as x86_64, and the
# updater would install it on hosts that cannot run it. Check the executable header instead.
hex_at() { od -An -tx1 -j "$2" -N "$3" "$1" | tr -d ' \n'; }
check_binary() {
    local platform=$1 path=$2 magic want_machine=""
    magic=$(hex_at "$path" 0 4)
    case "$platform" in
        linux-*)
            [ "$magic" = "7f454c46" ] || die "$path is not an ELF file but is labelled $platform"
            # ELF64 little-endian, then e_machine at offset 18 (0x3e x86-64, 0xb7 aarch64).
            [ "$(hex_at "$path" 4 1)" = "02" ] && [ "$(hex_at "$path" 5 1)" = "01" ] ||
                die "$path is not a 64-bit little-endian ELF file"
            case "$platform" in
                linux-x86_64) want_machine="3e00" ;;
                linux-aarch64) want_machine="b700" ;;
            esac
            [ "$(hex_at "$path" 18 2)" = "$want_machine" ] || die "$path has the wrong CPU for $platform (ELF e_machine $(hex_at "$path" 18 2))"
            ;;
        darwin-*)
            [ "$magic" = "cffaedfe" ] || die "$path is not a 64-bit Mach-O file but is labelled $platform"
            # cputype at offset 4: 0x0100000c arm64, 0x01000007 x86_64 (little-endian bytes).
            case "$platform" in
                darwin-aarch64) want_machine="0c000001" ;;
                darwin-x86_64) want_machine="07000001" ;;
            esac
            [ "$(hex_at "$path" 4 4)" = "$want_machine" ] || die "$path has the wrong CPU for $platform (Mach-O cputype $(hex_at "$path" 4 4))"
            ;;
    esac
}
for i in "${!platforms[@]}"; do check_binary "${platforms[$i]}" "${bin_paths[$i]}"; done
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
web_tar="hq-web-${version}.tar.gz"

stage=$(mktemp -d)
trap 'rm -rf "$stage"' EXIT

# Fixed owner, sorted names and a zeroed mtime keep the tarball reproducible for a given input.
tar_flags=(--sort=name --owner=0 --group=0 --numeric-owner --mtime=@0)
if ! tar --version 2>/dev/null | grep -q GNU; then tar_flags=(--uid 0 --gid 0); fi
hq_tars=()
for i in "${!platforms[@]}"; do
    name="hq-${version}-${platforms[$i]}.tar.gz"
    rm -f "$stage/hq"
    install -m 755 "${bin_paths[$i]}" "$stage/hq"
    (cd "$stage" && tar "${tar_flags[@]}" -czf "$out/$name" hq)
    hq_tars+=("$name")
done
(cd "$web_dist" && tar "${tar_flags[@]}" -czf "$out/$web_tar" .)

(cd "$out" && for f in "${hq_tars[@]}" "$web_tar"; do printf '%s  %s\n' "$(sha256 "$f")" "$f"; done > SHA256SUMS)

built_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
# One {name, sha256, size} object per file, built as JSON lines so the artifact list can be any length.
artifacts=$(for f in "${hq_tars[@]}" "$web_tar"; do
    jq -n --arg n "$f" --arg h "$(sha256 "$out/$f")" --argjson s "$(size "$out/$f")" '{name:$n,sha256:$h,size:$s}'
done | jq -s .)

jq -n \
    --argjson schema "$SCHEMA" --arg version "$version" --arg git_sha "$git_sha" \
    --arg channel "$channel" --arg built_at "$built_at" --arg min "$min_updater" \
    --argjson snap "$requires_snapshot" \
    --argjson artifacts "$artifacts" \
    '{schema:$schema, version:$version, git_sha:$git_sha, channel:$channel, built_at:$built_at,
      min_updater_version:$min, requires_db_snapshot:$snap,
      artifacts:$artifacts}' > "$out/manifest.json"

echo "package.sh: wrote ${hq_tars[*]} $web_tar SHA256SUMS manifest.json to $out"
