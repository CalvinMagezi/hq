#!/usr/bin/env bash
# Local check of package.sh, sign.sh, channel.sh and verify.sh with a throwaway key and fake inputs.
# With HQ_UPDATE_BIN=/path/to/hq (a build that has `hq update`) it also serves the fake
# release from a local http server and requires the real updater to accept it (dry run).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d)
server_pid=""
trap '[ -z "$server_pid" ] || kill "$server_pid" 2> /dev/null || true; rm -rf "$t"' EXIT

repo=acme/hq
# Newer than the 0.9.0 updater, with a base version above the floor: the floor, not the
# base, decides who may install it.
version=1.2.3-main.7
sha=$(printf 'a%.0s' $(seq 40))

mkdir "$t/web"
echo '<html></html>' > "$t/web/index.html"
echo '//sw' > "$t/web/sw.js"
echo '{}' > "$t/web/manifest.json"
printf '#!/bin/sh\necho "hq %s (%s 0)"\n' "$version" "${sha:0:7}" > "$t/hq"
minisign -G -W -f -p "$t/pub" -s "$t/sec" > /dev/null
echo 0.9.0 > "$t/floor"
export MIN_UPDATER_FILE="$t/floor"
export MINISIGN_KEY MINISIGN_PASSWORD=""
MINISIGN_KEY=$(cat "$t/sec")

# Layout of a GitHub release download tree, so a local server can stand in for github.com.
root="$t/site/$repo/releases/download"
rel="$root/v$version"
bash "$here/package.sh" --version "$version" --git-sha "$sha" --channel main \
    --bin "$t/hq" --web-dist "$t/web" --out "$rel" > /dev/null
bash "$here/sign.sh" "$rel/manifest.json"
bash "$here/verify.sh" --dir "$rel" --pubkey "$t/pub"

[ "$(tar -tzf "$rel/hq-$version-linux-x86_64.tar.gz" | sed 's#^\./##' | grep -v '/$')" = "hq" ] || {
    echo "selftest: binary tarball must hold exactly hq" >&2
    exit 1
}
[ "$(jq -r .min_updater_version "$rel/manifest.json")" = "0.9.0" ] || { echo "selftest: min_updater_version must come from the floor file" >&2; exit 1; }
echo 0.9.5 > "$t/floor"
bash "$here/package.sh" --version "$version" --git-sha "$sha" --channel main \
    --bin "$t/hq" --web-dist "$t/web" --out "$t/raised" > /dev/null
[ "$(jq -r .min_updater_version "$t/raised/manifest.json")" = "0.9.5" ] || { echo "selftest: floor change ignored" >&2; exit 1; }

port=$((20000 + RANDOM % 20000))
base="http://127.0.0.1:$port"
for ch in main stable; do
    mkdir -p "$root/channel-$ch"
    RELEASE_BASE_URL="$base" bash "$here/channel.sh" --channel "$ch" --repo "$repo" \
        --manifest "$rel/manifest.json" --seq 7001 --out "$root/channel-$ch/channel-$ch.json"
    bash "$here/sign.sh" "$root/channel-$ch/channel-$ch.json"
done

jq -e '.seq == 7001 and (.issued_at | test("^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:]{8}Z$"))' \
    "$root/channel-main/channel-main.json" > /dev/null || { echo "selftest: pointer lacks seq or issued_at" >&2; exit 1; }

if [ -n "${HQ_UPDATE_BIN:-}" ] && ! "$HQ_UPDATE_BIN" update --help > /dev/null 2>&1; then
    echo "selftest: HQ_UPDATE_BIN has no 'hq update', skipping the updater interop" >&2
    HQ_UPDATE_BIN=""
fi
if [ -n "${HQ_UPDATE_BIN:-}" ]; then
    (cd "$t/site" && exec python3 -m http.server "$port" --bind 127.0.0.1 > /dev/null 2>&1) &
    server_pid=$!
    sleep 1
    cat > "$t/update.conf" <<CONF
repo = "$repo"
channel = "stable"
base_url = "$base"
pubkey_path = "$t/pub"
state_dir = "$t/state"
bin_path = "$t/installed-hq"
web_dist = "$t/installed-web"
CONF
    printf '#!/bin/sh\necho "hq 1.2.2 (0000000 0)"\n' > "$t/installed-hq"
    chmod 755 "$t/installed-hq"
    for ch in stable main; do
        rc=0
        out=$("$HQ_UPDATE_BIN" update --conf "$t/update.conf" --channel "$ch" --apply --dry-run 2>&1) || rc=$?
        echo "$out"
        [ "$rc" -eq 0 ] || { echo "selftest: real updater refused the $ch release (exit $rc)" >&2; exit 1; }
        case "$out" in *"$version"*) ;; *) echo "selftest: updater did not report $version" >&2; exit 1 ;; esac
    done
fi

echo x >> "$rel/hq-web-$version.tar.gz"
if bash "$here/verify.sh" --dir "$rel" --pubkey "$t/pub" 2> /dev/null; then
    echo "selftest: tampered artifact was accepted" >&2
    exit 1
fi
echo "selftest: ok"
