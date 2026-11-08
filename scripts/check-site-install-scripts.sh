#!/usr/bin/env bash
# Checks a built site directory before it goes to Pages: install.sh and install.ps1 must be byte for
# byte the reviewed copies in apps/site/public, and each must embed exactly the key in
# release/minisign.pub and no other key-shaped string. Plain bash, so the Pages job that runs it
# runs no package code. Usage: scripts/check-site-install-scripts.sh <built site dir>
# Run from the repository root.
set -euo pipefail

site=${1:?usage: check-site-install-scripts.sh <built site dir>}
[ -d "$site" ] || { echo "$site is not a directory" >&2; exit 1; }

key=$(sed -n 2p release/minisign.pub | tr -d '\r')
[[ "$key" =~ ^RW[A-Za-z0-9+/]{54}$ ]] || { echo "release/minisign.pub has no key line" >&2; exit 1; }

for f in install.sh install.ps1; do
  { [ -f "$site/$f" ] && [ ! -L "$site/$f" ]; } || { echo "$f is missing from the built site" >&2; exit 1; }
  cmp -s "apps/site/public/$f" "$site/$f" || { echo "$f in the built site differs from apps/site/public/$f" >&2; exit 1; }
  found=$(grep -oE 'RW[QRST][A-Za-z0-9+/]{53}' "$site/$f" | sort -u || true)
  [ "$found" = "$key" ] || { echo "$f does not carry exactly the release key (found: ${found:-none})" >&2; exit 1; }
done
grep -qxF "PUBKEY_LINE=\"$key\"" "$site/install.sh" || { echo "install.sh does not set PUBKEY_LINE to the release key" >&2; exit 1; }
grep -qF "\$script:HqSigningKey = '$key'" "$site/install.ps1" || { echo "install.ps1 does not set HqSigningKey to the release key" >&2; exit 1; }
echo "install.sh and install.ps1 match apps/site/public and embed the release key."
