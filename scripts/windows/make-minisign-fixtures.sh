#!/usr/bin/env bash
# Regenerates scripts/windows/fixtures with throwaway minisign keys (no password). The secret keys
# are deleted; only public keys and signed samples are kept. Needs minisign
# (scripts/release/install-tools.sh installs a pinned one).
set -euo pipefail
cd "$(dirname "$0")/fixtures"
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT
for k in test other; do minisign -G -W -f -p "$k.pub" -s "$tmp/$k.key" > /dev/null; done
zip=hq-lite-0.0.0-abc1234-windows-x86_64.zip
printf '%s  %s\n' "$(printf 'sample zip' | sha256sum | cut -d' ' -f1)" "$zip" > "$zip.sha256"
minisign -S -s "$tmp/test.key" -m "$zip.sha256" -x "$zip.sha256.minisig" -t "hq lite test fixture" > /dev/null
# Longer than two BLAKE2b blocks, so the multi-block path is covered.
head -c 1000 /dev/zero | tr '\0' 'x' > blob.bin
minisign -S -s "$tmp/test.key" -m blob.bin -x blob.bin.minisig -t "hq lite test fixture" > /dev/null
echo "fixtures written"
