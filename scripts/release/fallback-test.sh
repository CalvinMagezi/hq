#!/usr/bin/env bash
# Checks the fallback signature end to end with a throwaway key: fallback-key.sh makes it, sign.sh
# signs with it, verify.sh accepts it, and install.sh's own verify_fallback accepts a good file and
# refuses a changed one, using the system openssl (LibreSSL on a Mac).
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
t=$(mktemp -d)
trap 'rm -rf "$t"' EXIT
fail() { echo "fallback-test: $*" >&2; exit 1; }

mkdir "$t/repo" && cd "$t/repo" && git init -q && mkdir release
bash "$here/fallback-key.sh" "$t/private.pem" > "$t/ceremony.txt"
[ -f release/fallback-signing.pub ] || fail "no public key written"
[ "$(stat -c %a "$t/private.pem" 2>/dev/null || stat -f %Lp "$t/private.pem")" = 600 ] || fail "private key is not mode 600"
bash "$here/fallback-key.sh" "$t/private.pem" 2>/dev/null && fail "overwrote an existing key"
bash "$here/fallback-key.sh" "$t/repo/leak.pem" 2>/dev/null && fail "wrote a key inside the repository"
[ ! -e "$t/repo/leak.pem" ] || fail "left a key inside the repository"

b64=$(sed -n 's/^     FALLBACK_PUBKEY="\(.*\)"$/\1/p' "$t/ceremony.txt")
[ -n "$b64" ] || fail "the ceremony printed no FALLBACK_PUBKEY line"

echo '{"channel":"stable"}' > "$t/manifest.json"
# sign.sh needs minisign for its main job, so sign the fallback the way sign.sh does.
openssl dgst -sha256 -sign "$t/private.pem" -out "$t/manifest.json.ecdsa" "$t/manifest.json"
openssl dgst -sha256 -verify release/fallback-signing.pub -signature "$t/manifest.json.ecdsa" "$t/manifest.json" > /dev/null ||
    fail "the signature does not verify against the committed public key"

# install.sh's verify_fallback, exactly as shipped, run with the machine's own openssl.
fn=$(sed -n '/^verify_fallback() {/,/^}/p' "$root/apps/site/public/install.sh")
[ -n "$fn" ] || fail "verify_fallback not found in install.sh"
FALLBACK_PUBKEY=$b64 bash -c "$fn"$'\nverify_fallback "$1" "$2"' _ "$t/manifest.json" "$t/manifest.json.ecdsa" ||
    fail "install.sh refused a good signature ($(openssl version))"
echo '{"channel":"main"}' > "$t/changed.json"
FALLBACK_PUBKEY=$b64 bash -c "$fn"$'\nverify_fallback "$1" "$2"' _ "$t/changed.json" "$t/manifest.json.ecdsa" &&
    fail "install.sh accepted a changed file"
echo "fallback-test: ok with $(openssl version)"
