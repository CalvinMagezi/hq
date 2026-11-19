#!/usr/bin/env bash
# One-time ceremony: makes the ECDSA P-256 key behind the release's second signature, the one a Mac
# with no minisign and no Homebrew can verify with the system openssl.
#   scripts/release/fallback-key.sh <private key file, outside the repository>
# It writes the private key (mode 600) to that path, writes the public key to
# release/fallback-signing.pub, and prints the base64 line that goes into install.sh. It never
# touches GitHub: you store the secret yourself, then delete the local private key.
set -euo pipefail
out=${1:?usage: fallback-key.sh <private key file, outside the repository>}
case "$(cd "$(dirname "$out")" && pwd -P)/" in
    "$(git rev-parse --show-toplevel)"/*) echo "put the private key outside the repository" >&2; exit 1 ;;
esac
[ ! -e "$out" ] || { echo "$out exists; refusing to overwrite a key" >&2; exit 1; }

umask 077
openssl ecparam -name prime256v1 -genkey -noout -out "$out"
openssl ec -in "$out" -pubout -out release/fallback-signing.pub 2>/dev/null
b64=$(openssl ec -in "$out" -pubout -outform DER 2>/dev/null | base64 | tr -d '\n')

cat <<MSG
Wrote $out (private) and release/fallback-signing.pub (public).

1. Store the private key as the repository secret FALLBACK_SIGNING_KEY:
     gh secret set FALLBACK_SIGNING_KEY < $out
2. Delete the local private key once the secret is set (keep a backup in your password manager):
     shred -u $out    # or: rm -P $out on macOS
3. Put this line in apps/site/public/install.sh (replacing FALLBACK_PUBKEY=""):
     FALLBACK_PUBKEY="$b64"
4. Commit release/fallback-signing.pub and install.sh together, then cut a release. A release made
   before the secret exists has no .ecdsa files, and install.sh on a Mac without minisign says so.
MSG
