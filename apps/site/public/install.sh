#!/usr/bin/env bash
# Installs the hq CLI from a signed release of github.com/CalvinMagezi/hq.
#
#   curl -fsSL https://agent-hq.online/install.sh | bash
#   curl -fsSL https://agent-hq.online/install.sh | bash -s -- --channel main --prefix ~/bin
#
# It downloads the channel pointer and manifest, verifies their minisign signatures
# against the key below, checks the archive SHA-256, and only then installs the binary.
# Everything runs inside main(), so a truncated download executes nothing.
set -euo pipefail

REPO="CalvinMagezi/hq"
BASE_URL="https://github.com"
# Must match release/minisign.pub in the repository.
PUBKEY_LINE="RWTSi05PPMb9UVVnGilhLWT7h/mjQ1VjfAEXszxJB/Er8UEsCXFc3o1/"

CHANNEL="stable"
PREFIX="${HQ_PREFIX:-$HOME/.local/bin}"
WORK=""

die() { echo "install: $*" >&2; exit 1; }
say() { echo "==> $*"; }
cleanup() { [ -z "$WORK" ] || rm -rf "$WORK"; }

usage() {
    cat <<'USAGE'
usage: install.sh [--channel stable|main] [--prefix DIR]

  --channel  release channel (default: stable)
  --prefix   directory for the hq binary (default: ~/.local/bin, or $HQ_PREFIX)

The first download is verified with the minisign tool when it is installed, otherwise with
OpenSSL 1.1.1 or newer. On macOS the system OpenSSL cannot do this, so install minisign
(brew install minisign) or OpenSSL (brew install openssl).
USAGE
}

platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os/$arch" in
        Linux/x86_64) echo "linux-x86_64" ;;
        Linux/aarch64 | Linux/arm64) echo "linux-aarch64" ;;
        Darwin/arm64) echo "darwin-aarch64" ;;
        Darwin/x86_64)
            # Under Rosetta uname reports x86_64 on Apple Silicon.
            if [ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" = "1" ]; then
                echo "darwin-aarch64"
            else
                die "Intel Macs have no prebuilt binary yet. Build from source: https://github.com/$REPO#install-from-source"
            fi
            ;;
        *) die "no prebuilt binary for $os $arch. Build from source: https://github.com/$REPO#install-from-source" ;;
    esac
}

fetch() { curl --proto '=https' --tlsv1.2 -fsSL --max-time 600 -o "$2" "$1" || die "could not download $1"; }

sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
    else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

# A string field of the flat JSON the release tooling writes, e.g. "version": "0.9.1".
json_string() { sed -n "s/^[[:space:]]*\"$1\":[[:space:]]*\"\\([^\"]*\\)\".*/\\1/p" "$2" | head -n 1; }

# The sha256 of the manifest artifact called $1.
artifact_sha256() {
    awk -v n="$1" '
        /"name":/ { current = ($0 ~ "\"" n "\"") }
        current && /"sha256":/ { gsub(/.*"sha256":[[:space:]]*"/, ""); gsub(/".*/, ""); print; exit }
    ' "$2"
}

# Verifies a minisign signature (file signature and trusted comment) with OpenSSL 1.1.1+.
# Arguments: message-file signature-file.
verify_openssl() {
    local msg="$1" sigfile="$2" w ok=1 openssl_bin
    openssl_bin="$(find_openssl)" || return 2
    w="$(mktemp -d)"
    {
        printf '%s' "$PUBKEY_LINE" | base64 -d > "$w/pub.raw" 2>/dev/null &&
        [ "$(wc -c < "$w/pub.raw" | tr -d ' ')" -eq 42 ] &&
        [ "$(head -c 2 "$w/pub.raw")" = "Ed" ] &&
        tail -c 32 "$w/pub.raw" > "$w/key.raw" &&
        { printf '\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00'; cat "$w/key.raw"; } > "$w/pub.der" &&
        sed -n 2p "$sigfile" | base64 -d > "$w/sig.raw" 2>/dev/null &&
        [ "$(wc -c < "$w/sig.raw" | tr -d ' ')" -eq 74 ] &&
        [ "$(head -c 2 "$w/sig.raw")" = "ED" ] &&
        [ "$(dd if="$w/sig.raw" bs=1 skip=2 count=8 2>/dev/null | od -An -tx1 | tr -d ' \n')" = "$(dd if="$w/pub.raw" bs=1 skip=2 count=8 2>/dev/null | od -An -tx1 | tr -d ' \n')" ] &&
        tail -c 64 "$w/sig.raw" > "$w/sig.bin" &&
        "$openssl_bin" dgst -blake2b512 -binary "$msg" > "$w/hash.bin" &&
        "$openssl_bin" pkeyutl -verify -pubin -inkey "$w/pub.der" -keyform DER -rawin -in "$w/hash.bin" -sigfile "$w/sig.bin" >/dev/null 2>&1 &&
        { sed -n 3p "$sigfile" | sed 's/^trusted comment: //' | tr -d '\r\n' > "$w/comment.txt"; } &&
        sed -n 4p "$sigfile" | base64 -d > "$w/global.bin" 2>/dev/null &&
        [ "$(wc -c < "$w/global.bin" | tr -d ' ')" -eq 64 ] &&
        { cat "$w/sig.bin" "$w/comment.txt"; } > "$w/global.msg" &&
        "$openssl_bin" pkeyutl -verify -pubin -inkey "$w/pub.der" -keyform DER -rawin -in "$w/global.msg" -sigfile "$w/global.bin" >/dev/null 2>&1
    } && ok=0
    rm -rf "$w"
    return "$ok"
}

# An OpenSSL that can verify Ed25519 with -rawin and compute BLAKE2b (not macOS's LibreSSL).
find_openssl() {
    local candidate
    for candidate in openssl /opt/homebrew/opt/openssl@3/bin/openssl /usr/local/opt/openssl@3/bin/openssl; do
        if command -v "$candidate" >/dev/null 2>&1 && "$candidate" version 2>/dev/null | grep -q '^OpenSSL'; then
            command -v "$candidate"
            return 0
        fi
    done
    return 1
}

verify_signature() {
    if command -v minisign >/dev/null 2>&1; then
        minisign -V -P "$PUBKEY_LINE" -m "$1" -x "$2" >/dev/null 2>&1
    else
        verify_openssl "$1" "$2"
    fi
}

check_can_verify() {
    command -v minisign >/dev/null 2>&1 && return 0
    find_openssl >/dev/null 2>&1 && return 0
    die "cannot verify the download: install minisign (brew install minisign) or OpenSSL (apt install openssl, brew install openssl), then run this again"
}

main() {
    while [ $# -gt 0 ]; do
        case "$1" in
            --channel) [ $# -ge 2 ] || die "--channel needs a value"; CHANNEL="$2"; shift 2 ;;
            --prefix) [ $# -ge 2 ] || die "--prefix needs a value"; PREFIX="$2"; shift 2 ;;
            -h | --help) usage; exit 0 ;;
            *) die "unknown option: $1 (try --help)" ;;
        esac
    done
    printf '%s' "$CHANNEL" | grep -Eq '^[A-Za-z0-9_.-]+$' || die "--channel must be a plain name"
    for tool in curl tar awk sed base64; do
        command -v "$tool" >/dev/null 2>&1 || die "$tool is required"
    done
    check_can_verify

    local plat release name version manifest_url tail want url
    plat="$(platform)"
    release="$BASE_URL/$REPO/releases/download"
    WORK="$(mktemp -d)"
    trap cleanup EXIT

    say "Looking up the $CHANNEL release for $plat"
    fetch "$release/channel-$CHANNEL/channel-$CHANNEL.json" "$WORK/channel.json"
    fetch "$release/channel-$CHANNEL/channel-$CHANNEL.json.minisig" "$WORK/channel.json.minisig"
    verify_signature "$WORK/channel.json" "$WORK/channel.json.minisig" || die "the channel pointer signature does not verify"
    [ "$(json_string channel "$WORK/channel.json")" = "$CHANNEL" ] || die "the channel pointer is for a different channel"

    version="$(json_string version "$WORK/channel.json")"
    manifest_url="$(json_string manifest_url "$WORK/channel.json")"
    printf '%s' "$version" | grep -Eq '^[A-Za-z0-9._+-]+$' || die "unexpected version in the channel pointer"
    case "$manifest_url" in "$release"/*) ;; *) die "the channel pointer points outside $release" ;; esac
    tail="${manifest_url#"$release"/}"
    case "$tail" in
        "" | *..* | *//* | *\\* | *\?* | *\#* | *@* | *%2[eEfF]* | *%5[cC]* | *%00*) die "the channel pointer URL looks unsafe" ;;
    esac

    fetch "$manifest_url" "$WORK/manifest.json"
    fetch "$manifest_url.minisig" "$WORK/manifest.json.minisig"
    [ "$(sha256_of "$WORK/manifest.json")" = "$(json_string manifest_sha256 "$WORK/channel.json")" ] \
        || die "the manifest does not match the channel pointer"
    verify_signature "$WORK/manifest.json" "$WORK/manifest.json.minisig" || die "the manifest signature does not verify"
    [ "$(json_string version "$WORK/manifest.json")" = "$version" ] || die "the manifest version differs from the pointer"

    name="hq-$version-$plat.tar.gz"
    want="$(artifact_sha256 "$name" "$WORK/manifest.json")"
    printf '%s' "$want" | grep -Eq '^[0-9a-f]{64}$' \
        || die "release $version has no $plat binary yet. Try --channel main, or build from source: https://github.com/$REPO#install-from-source"

    say "Verified $version. Downloading $name"
    url="${manifest_url%/*}/$name"
    fetch "$url" "$WORK/$name"
    [ "$(sha256_of "$WORK/$name")" = "$want" ] || die "checksum mismatch for $name"
    [ "$(tar -tzf "$WORK/$name" | sed 's|^\./||')" = "hq" ] || die "$name must contain exactly one file named hq"
    mkdir "$WORK/x"
    tar -xzf "$WORK/$name" -C "$WORK/x"
    [ -f "$WORK/x/hq" ] && [ ! -L "$WORK/x/hq" ] || die "$name does not contain a regular file named hq"
    chmod 755 "$WORK/x/hq"
    "$WORK/x/hq" --version | grep -qF "$version" || die "the downloaded binary does not report version $version (wrong glibc or platform?)"

    mkdir -p "$PREFIX"
    mv -f "$WORK/x/hq" "$PREFIX/.hq.new"
    mv -f "$PREFIX/.hq.new" "$PREFIX/hq"
    say "Installed $PREFIX/hq: $("$PREFIX/hq" --version)"

    case ":$PATH:" in
        *":$PREFIX:"*) ;;
        *) echo; echo "Add $PREFIX to your PATH, for example: export PATH=\"$PREFIX:\$PATH\"" ;;
    esac
    cat <<NEXT

Next steps:
  hq install        scaffold your vault and config
  hq env            add an LLM API key (OpenRouter, Anthropic or Google)
  hq doctor         check the setup
  hq chat           talk to HQ, or: hq start all   (daemon, API and web UI on :5678)

Always-on server with signed automatic updates: https://github.com/$REPO/blob/main/deploy/README.md
NEXT
}

main "$@"
