#!/usr/bin/env bash
# setup-signing.sh — One-time setup: creates a stable local code-signing certificate.
#
# WHY: Ad-hoc signatures (codesign -s -) embed CDHash in the CSREQ. CDHash changes
# every build, so macOS TCC invalidates the FDA grant. A self-signed cert gives a
# stable CSREQ: "identifier + cert root hash". FDA is granted once and never breaks.
#
# Usage: ./scripts/setup-signing.sh
# After running: ./scripts/install-hq.sh (sudo only for the one-time --link)
# Then re-grant FDA once in System Settings — last time ever.

set -euo pipefail

CERT_NAME="HQ Local Code Signing"
KEYCHAIN_PATH="$HOME/.hq/signing.keychain"
KEYCHAIN_PASS="hq-$(hostname -s)-signing"
P12_PASS="hq-p12-import"

echo "=== HQ Code Signing Setup ==="

# Already configured with a VALID identity? (not just present but trusted)
if [[ -f "$KEYCHAIN_PATH" ]] && \
   security find-identity -p codesigning "$KEYCHAIN_PATH" 2>/dev/null | grep -q "$CERT_NAME"; then
    echo "Certificate already present."
    security find-identity -p codesigning "$KEYCHAIN_PATH" | grep "$CERT_NAME"
    echo ""
    echo "Run: ./scripts/install-hq.sh  (sudo only for the one-time: sudo ./scripts/install-hq.sh --link)"
    exit 0
fi

# Clean up any partial state
security delete-keychain "$KEYCHAIN_PATH" 2>/dev/null || true
rm -f "$KEYCHAIN_PATH" "${KEYCHAIN_PATH}-db" 2>/dev/null || true
rm -f /tmp/hq-sign-key.pem /tmp/hq-sign-cert.pem /tmp/hq-sign.p12 2>/dev/null || true

mkdir -p ~/.hq

echo "Generating self-signed code-signing certificate..."
openssl req -x509 -newkey rsa:2048 \
    -keyout /tmp/hq-sign-key.pem \
    -out /tmp/hq-sign-cert.pem \
    -days 9999 -nodes \
    -subj "/CN=$CERT_NAME/O=Agent HQ" \
    -addext "keyUsage=critical,digitalSignature" \
    -addext "extendedKeyUsage=critical,codeSigning" \
    -addext "basicConstraints=critical,CA:FALSE" 2>/dev/null

# macOS security requires legacy PBE ciphers (OpenSSL 3.x compat)
openssl pkcs12 -export \
    -out /tmp/hq-sign.p12 \
    -inkey /tmp/hq-sign-key.pem \
    -in /tmp/hq-sign-cert.pem \
    -passout "pass:$P12_PASS" \
    -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES -macalg SHA1 2>/dev/null

echo "Creating signing keychain..."
security create-keychain -p "$KEYCHAIN_PASS" "$KEYCHAIN_PATH"

echo "Importing certificate..."
security import /tmp/hq-sign.p12 \
    -k "$KEYCHAIN_PATH" \
    -P "$P12_PASS" \
    -T /usr/bin/codesign

# Allow codesign to use the private key without GUI prompts
security set-key-partition-list \
    -S "apple-tool:,apple:,codesign:" \
    -k "$KEYCHAIN_PASS" \
    "$KEYCHAIN_PATH" 2>/dev/null || true

# Add to keychain search list so codesign can find it
EXISTING=$(security list-keychains -d user | tr -d '"' | tr '\n' ' ' | xargs)
security list-keychains -d user -s "$KEYCHAIN_PATH" $EXISTING

# Cleanup temp files
rm -f /tmp/hq-sign-key.pem /tmp/hq-sign-cert.pem /tmp/hq-sign.p12

# Verify
CERT_HASH=$(security find-identity -p codesigning "$KEYCHAIN_PATH" 2>/dev/null | grep "$CERT_NAME" | awk '{print $2}')
if [[ -n "$CERT_HASH" ]]; then
    # Never lock by idle timeout so the daemon can sign during self-update
    security set-keychain-settings -t 0 "$KEYCHAIN_PATH" 2>/dev/null || true

    # Save stable CSREQ so grant-fda.sh can embed it in the PPPC profile
    CERT_HASH_LC=$(echo "$CERT_HASH" | tr '[:upper:]' '[:lower:]')
    CSREQ="identifier \"com.agent-hq.hq\" and certificate root = H\"${CERT_HASH_LC}\""
    echo "$CSREQ" > "$HOME/.hq/signing-csreq.txt"

    echo ""
    echo "Certificate ready: $CERT_HASH"
    echo ""
    echo "CSREQ (stable — FDA survives all future rebuilds):"
    echo "  $CSREQ"
    echo ""
    echo "Next steps:"
    echo "  1. ./scripts/install-hq.sh  (after a one-time: sudo ./scripts/install-hq.sh --link)"
    echo "  2. ./scripts/grant-fda.sh   ← one-time profile install, no more prompts ever"
else
    echo "Warning: cert not found after import. Check $KEYCHAIN_PATH"
    exit 1
fi
