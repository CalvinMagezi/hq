#!/usr/bin/env bash
# Installs pinned, checksum-verified minisign and jq into $1 (default $RUNNER_TEMP/tools)
# on a Linux x86_64 runner, and prints the directory so the caller can add it to PATH.
# Release tarballs are used instead of apt so the version cannot drift under a release job.
set -euo pipefail

MINISIGN_VERSION="0.12"
MINISIGN_SHA256="9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73"
JQ_VERSION="1.8.2"
JQ_SHA256="b1c22172dd303f3be49e935aa56aa48a8b7a46e0bc838b4997d3bb451495870f"

dest=${1:-${RUNNER_TEMP:-/tmp}/tools}
mkdir -p "$dest"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

fetch_verified() { # url sha256 file
    curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 --output "$3" "$1"
    echo "$2  $3" | sha256sum -c - > /dev/null || { echo "install-tools: checksum mismatch for $1" >&2; exit 1; }
}

fetch_verified "https://github.com/jedisct1/minisign/releases/download/$MINISIGN_VERSION/minisign-$MINISIGN_VERSION-linux.tar.gz" "$MINISIGN_SHA256" "$tmp/minisign.tgz"
tar -xzf "$tmp/minisign.tgz" -C "$tmp"
install -m 755 "$tmp/minisign-linux/x86_64/minisign" "$dest/minisign"

fetch_verified "https://github.com/jqlang/jq/releases/download/jq-$JQ_VERSION/jq-linux-amd64" "$JQ_SHA256" "$tmp/jq"
install -m 755 "$tmp/jq" "$dest/jq"

echo "$dest"
