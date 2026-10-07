#!/usr/bin/env bash
# Idempotent instance installer for the pull-based update system.
#
#   sudo deploy/install.sh --repo <owner>/<repo> --channel stable --pubkey ./update.pub \
#        [--bootstrap-binary ./hq | --bootstrap-sha256 <hex>]
#
# Creates the hq user and directories, installs the systemd units from this
# checkout, writes /etc/hq/update.conf and /etc/hq/update.pub, bootstraps the
# first `hq` binary, runs the first signature-verified `hq update --apply`
# and enables the services and the updater timer. Safe to re-run.
set -euo pipefail

REPO=""
CHANNEL="stable"
PUBKEY_FILE=""
BASE_URL="https://github.com"
BOOTSTRAP_BIN=""
BOOTSTRAP_SHA256=""
START=1
CHANNEL_SET=0
BASE_URL_SET=0
FORCE_UNITS=0
FRESH_BOOTSTRAP=0
CURL_PROTO="=https"

HQ_USER=hq
HQ_HOME=/opt/hq
CONF_DIR=/etc/hq
LIB_DIR=/usr/local/lib/hq
STATE_DIR=/var/lib/hq-update
WEB_DIR=/usr/local/share/hq/web
BIN=/usr/local/bin/hq
UNIT_DIR=/etc/systemd/system
SRC_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TMP=""
PLATFORM=""

usage() {
    cat >&2 <<USAGE
usage: install.sh --repo <owner>/<repo> --pubkey <minisign.pub> [options]

  --repo <owner>/<repo>     GitHub repository that publishes releases (required)
  --pubkey <file>           minisign public key that signs releases (required)
  --channel <name>          main or stable (default: stable)
  --base-url <url>          release host (default: https://github.com)
  --bootstrap-binary <file> use this hq binary for the first install
  --bootstrap-sha256 <hex>  download the first binary and require this SHA-256
  --force-units             overwrite an existing hq.service (default: keep and show a diff)
  --no-start                install everything but do not run the first update
A re-run keeps channel and base_url from the existing update.conf unless you pass them again.
Without --bootstrap-binary the first release is downloaded and its manifest
signature is checked with the 'minisign' tool when present, else with OpenSSL (1.1.1 or
newer, already on Ubuntu 22.04), or pinned with --bootstrap-sha256.
USAGE
    exit 2
}

die() { echo "install.sh: $*" >&2; exit 1; }
log() { echo "==> $*"; }

cleanup() { [ -n "$TMP" ] && rm -rf "$TMP"; return 0; }
trap cleanup EXIT
trap 'echo "install.sh failed near line $LINENO; the host may be partly configured. Fix the cause and re-run, it is safe to repeat." >&2' ERR

while [ $# -gt 0 ]; do
    case "$1" in
        --repo) REPO="${2:?--repo needs a value}"; shift 2 ;;
        --channel) CHANNEL="${2:?--channel needs a value}"; CHANNEL_SET=1; shift 2 ;;
        --pubkey) PUBKEY_FILE="${2:?--pubkey needs a value}"; shift 2 ;;
        --base-url) BASE_URL="${2:?--base-url needs a value}"; BASE_URL_SET=1; shift 2 ;;
        --bootstrap-binary) BOOTSTRAP_BIN="${2:?--bootstrap-binary needs a value}"; shift 2 ;;
        --bootstrap-sha256) BOOTSTRAP_SHA256="${2:?--bootstrap-sha256 needs a value}"; shift 2 ;;
        --force-units) FORCE_UNITS=1; shift ;;
        --no-start) START=0; shift ;;
        -h|--help) usage ;;
        *) echo "unknown option: $1" >&2; usage ;;
    esac
done

# Release artifacts are named hq-<version>-<os>-<arch>.tar.gz. This installer sets up a
# systemd server, so Linux x86_64 and aarch64 are the only targets.
detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"
    case "$os" in
        Linux) ;;
        Darwin)
            die "macOS is not a server target for this installer (it needs systemd). To install the hq CLI on Apple Silicon run: npx agent-hq-cli"
            ;;
        *) die "unsupported OS $os; this installer supports Linux x86_64 and aarch64" ;;
    esac
    case "$arch" in
        x86_64|amd64) PLATFORM=linux-x86_64 ;;
        aarch64|arm64) PLATFORM=linux-aarch64 ;;
        *) die "unsupported CPU architecture $arch; releases are built for x86_64 and aarch64" ;;
    esac
}
detect_platform

[ "$(id -u)" -eq 0 ] || die "run as root"
[ -n "$REPO" ] && [ -n "$PUBKEY_FILE" ] || usage
printf '%s' "$REPO" | grep -Eq '^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$' || die "--repo must look like <owner>/<repo>"
printf '%s' "$CHANNEL" | grep -Eq '^[A-Za-z0-9_.-]+$' || die "--channel must be a plain name"
if printf '%s' "$BASE_URL" | grep -Eq '^https://[A-Za-z0-9.-]+(:[0-9]{1,5})?$'; then
    CURL_PROTO="=https"
elif printf '%s' "$BASE_URL" | grep -Eq '^http://(127\.0\.0\.1|localhost)(:[0-9]{1,5})?$'; then
    CURL_PROTO="=https,http"
else
    die "--base-url must be https://host[:port] with no path (plain http only for 127.0.0.1 or localhost)"
fi
[ -f "$PUBKEY_FILE" ] || die "public key file not found: $PUBKEY_FILE"
PUBKEY_LINE="$(grep -v '^untrusted comment:' "$PUBKEY_FILE" | grep -v '^[[:space:]]*$' | tail -n 1)"
printf '%s' "$PUBKEY_LINE" | grep -Eq '^RW[A-Za-z0-9+/]{54,}={0,2}$' || die "$PUBKEY_FILE is not a minisign public key"
[ -f "$SRC_DIR/deploy/hq.service" ] && [ -f "$SRC_DIR/deploy/update/hq-update.service" ] \
    || die "run this from a checkout of the repository (deploy/ templates not found under $SRC_DIR)"

# Fail before touching the system if the first download could not be verified.
# Verifies a minisign signature (file signature and trusted comment) with OpenSSL 1.1.1 or newer,
# so a host without the minisign tool (it is not packaged for Ubuntu 22.04) can still verify.
verify_minisign_openssl() {
    local msg="$1" sigfile="$2" publine="$3" w ok=1
    command -v openssl >/dev/null 2>&1 || return 2
    w="$(mktemp -d)"
    {
        printf '%s' "$publine" | base64 -d > "$w/pub.raw" 2>/dev/null &&
        [ "$(wc -c < "$w/pub.raw")" -eq 42 ] &&
        [ "$(head -c 2 "$w/pub.raw")" = "Ed" ] &&
        tail -c 32 "$w/pub.raw" > "$w/key.raw" &&
        { printf '\x30\x2a\x30\x05\x06\x03\x2b\x65\x70\x03\x21\x00'; cat "$w/key.raw"; } > "$w/pub.der" &&
        sed -n 2p "$sigfile" | base64 -d > "$w/sig.raw" 2>/dev/null &&
        [ "$(wc -c < "$w/sig.raw")" -eq 74 ] &&
        [ "$(head -c 2 "$w/sig.raw" | tr -d '\0')" = "ED" ] &&
        [ "$(dd if="$w/sig.raw" bs=1 skip=2 count=8 2>/dev/null | od -An -tx1 | tr -d ' \n')" = "$(dd if="$w/pub.raw" bs=1 skip=2 count=8 2>/dev/null | od -An -tx1 | tr -d ' \n')" ] &&
        tail -c 64 "$w/sig.raw" > "$w/sig.bin" &&
        openssl dgst -blake2b512 -binary "$msg" > "$w/hash.bin" &&
        openssl pkeyutl -verify -pubin -inkey "$w/pub.der" -keyform DER -rawin -in "$w/hash.bin" -sigfile "$w/sig.bin" >/dev/null 2>&1 &&
        { sed -n 3p "$sigfile" | sed 's/^trusted comment: //' | tr -d '\n' > "$w/comment.txt"; } &&
        sed -n 4p "$sigfile" | base64 -d > "$w/global.bin" 2>/dev/null &&
        [ "$(wc -c < "$w/global.bin")" -eq 64 ] &&
        { cat "$w/sig.bin" "$w/comment.txt"; } > "$w/global.msg" &&
        openssl pkeyutl -verify -pubin -inkey "$w/pub.der" -keyform DER -rawin -in "$w/global.msg" -sigfile "$w/global.bin" >/dev/null 2>&1
    } && ok=0
    rm -rf "$w"
    return "$ok"
}

# Uses the minisign tool when present, else OpenSSL. Arguments: message-file signature-file.
verify_signature() {
    if command -v minisign >/dev/null 2>&1; then
        minisign -V -P "$PUBKEY_LINE" -m "$1" -x "$2" >/dev/null 2>&1
    else
        verify_minisign_openssl "$1" "$2" "$PUBKEY_LINE"
    fi
}
can_verify() { command -v minisign >/dev/null 2>&1 || command -v openssl >/dev/null 2>&1; }

if [ ! -x "$BIN" ] && [ -z "$BOOTSTRAP_BIN" ] && [ -z "$BOOTSTRAP_SHA256" ] && ! can_verify; then
    die "cannot verify the first download: install openssl or minisign, or pass --bootstrap-sha256 or --bootstrap-binary"
fi

# The agent bash tool refuses to run without an OS sandbox, so a fresh host needs bubblewrap.
ensure_sandbox() {
    command -v bwrap >/dev/null 2>&1 && return 0
    if command -v apt-get >/dev/null 2>&1; then
        log "installing bubblewrap (the bash tool sandbox)"
        apt-get update -qq && apt-get install -y -qq bubblewrap && return 0
    fi
    echo "warning: bubblewrap is not installed, so the agent bash tool will refuse every command until it is." >&2
    echo "         Install it, or set governance.bash.sandbox to best_effort in the config to run unwrapped." >&2
}

ensure_packages() {
    local missing=()
    for tool in curl jq tar flock setpriv sha256sum systemctl; do
        command -v "$tool" >/dev/null 2>&1 || missing+=("$tool")
    done
    [ ${#missing[@]} -eq 0 ] && return 0
    command -v apt-get >/dev/null 2>&1 || die "missing tools (${missing[*]}) and no apt-get to install them"
    log "installing distribution packages for: ${missing[*]}"
    apt-get update -qq
    apt-get install -y -qq ca-certificates curl jq tar util-linux coreutils
    for tool in "${missing[@]}"; do
        command -v "$tool" >/dev/null 2>&1 || die "$tool is still missing after installing packages"
    done
}

ensure_user_and_dirs() {
    if ! id "$HQ_USER" >/dev/null 2>&1; then
        log "creating system user $HQ_USER"
        useradd -r -s /usr/sbin/nologin -d "$HQ_HOME" "$HQ_USER"
    fi
    getent group systemd-journal >/dev/null 2>&1 && usermod -aG systemd-journal "$HQ_USER"
    mkdir -p "$HQ_HOME/.vault" "$HQ_HOME/data/update-snapshots" "$CONF_DIR" "$LIB_DIR" "$STATE_DIR" "$WEB_DIR/dist"
    chown -R "$HQ_USER:$HQ_USER" "$HQ_HOME"
    # The updater (root) swaps files in these; the service user must not be able to repoint them.
    chown root:root "$CONF_DIR" "$LIB_DIR" "$STATE_DIR" /usr/local/share/hq "$WEB_DIR" "$WEB_DIR/dist"
    chmod 755 "$CONF_DIR" "$LIB_DIR" /usr/local/share/hq "$WEB_DIR"
    chmod 700 "$STATE_DIR"
}

write_if_changed() { # dest mode content-from-stdin
    local dest="$1" mode="$2" new
    new="$(mktemp)"
    cat > "$new"
    if [ -f "$dest" ] && cmp -s "$new" "$dest"; then
        rm -f "$new"
        return 0
    fi
    [ -f "$dest" ] && cp -p "$dest" "$dest.bak"
    install -m "$mode" -o root -g root "$new" "$dest"
    rm -f "$new"
    log "wrote $dest"
}

toml_escape() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'; }

# Sets a top-level `key = "value"` line, replacing it or adding it above any
# table, so hand edits to the rest of the file survive a re-run. Values reach
# awk through the environment, never through a pattern.
set_conf_key() { # file key value [only-if-absent]
    local file="$1" key="$2" value tmp
    value="$(toml_escape "$3")"
    if [ "${4:-}" = "only-if-absent" ] && grep -Eq "^${key}[[:space:]]*=" "$file"; then
        return 0
    fi
    tmp="$(mktemp)"
    KEY="$key" VAL="$value" awk '
        BEGIN { k = ENVIRON["KEY"]; line = k " = \"" ENVIRON["VAL"] "\"" }
        { if ($0 ~ "^" k "[ \t]*=") { if (!done) { print line; done = 1 } } else { rest[++n] = $0 } }
        END { if (!done) print line; for (i = 1; i <= n; i++) print rest[i] }
    ' "$file" > "$tmp"
    if ! cmp -s "$tmp" "$file"; then
        cp -p "$file" "$file.bak"
        install -m 0644 -o root -g root "$tmp" "$file"
        log "updated $key in $file"
    fi
    rm -f "$tmp"
}

# Port of the web server from the host config, else the default.
detect_health_url() {
    local port=""
    if [ -f "$HQ_HOME/config.yaml" ]; then
        port="$(sed -nE 's/^ws_port:[[:space:]]*"?([0-9]{1,5})"?[[:space:]]*$/\1/p' "$HQ_HOME/config.yaml" | head -n 1)"
    fi
    printf 'http://127.0.0.1:%s/health' "${port:-5678}"
}

install_config() {
    install -m 0644 -o root -g root "$PUBKEY_FILE" "$CONF_DIR/update.pub.new"
    mv -f "$CONF_DIR/update.pub.new" "$CONF_DIR/update.pub"
    local fresh=0
    if [ ! -f "$CONF_DIR/update.conf" ]; then
        fresh=1
        write_if_changed "$CONF_DIR/update.conf" 0644 <<'CONF'
interval = "10m"
CONF
    fi
    set_conf_key "$CONF_DIR/update.conf" repo "$REPO"
    if [ "$fresh" -eq 1 ] || [ "$CHANNEL_SET" -eq 1 ]; then
        set_conf_key "$CONF_DIR/update.conf" channel "$CHANNEL"
    fi
    if [ "$fresh" -eq 1 ] || [ "$BASE_URL_SET" -eq 1 ]; then
        set_conf_key "$CONF_DIR/update.conf" base_url "$BASE_URL"
    fi
    set_conf_key "$CONF_DIR/update.conf" health_url "$(detect_health_url)" only-if-absent
}

# Installs a template unless an edited copy exists; shows the difference then.
install_unit_keep_edits() {
    local src="$1" dest
    dest="$UNIT_DIR/$(basename "$1")"
    if [ -f "$dest" ] && ! cmp -s "$src" "$dest" && [ "$FORCE_UNITS" -ne 1 ]; then
        echo "note: keeping existing $dest (differs from the template; --force-units overwrites):" >&2
        diff -u "$dest" "$src" >&2 || true
        return 0
    fi
    install -m 0644 -o root -g root "$src" "$dest"
}

# hq serves the web tree from the root-owned directory; copy a legacy tree over
# so the UI is not blank until the first release brings its own.
seed_web() {
    if [ ! -f "$WEB_DIR/dist/index.html" ] && [ -f "$HQ_HOME/web/dist/index.html" ]; then
        log "seeding $WEB_DIR/dist from $HQ_HOME/web/dist"
        cp -a "$HQ_HOME/web/dist/." "$WEB_DIR/dist/"
        chown -R root:root "$WEB_DIR/dist"
    fi
}

install_units() {
    install -m 0755 -o root -g root "$SRC_DIR/deploy/update/hq-updater" "$LIB_DIR/hq-updater"
    install_unit_keep_edits "$SRC_DIR/deploy/hq.service"
    for unit in "$SRC_DIR/deploy/update/hq-update.service" "$SRC_DIR/deploy/update/hq-update.timer"; do
        install -m 0644 -o root -g root "$unit" "$UNIT_DIR/$(basename "$unit")"
    done
    mkdir -p "$UNIT_DIR/hq.service.d"
    write_if_changed "$UNIT_DIR/hq.service.d/web-static-dir.conf" 0644 <<DROPIN
[Service]
Environment=HQ_WEB_STATIC_DIR=$WEB_DIR/dist
DROPIN
    systemctl daemon-reload
}

fetch() { curl --proto "$CURL_PROTO" --proto-redir "=https" --tlsv1.2 -fsSL --max-time 600 -o "$2" "$1"; }

bootstrap_binary() {
    if [ -n "$BOOTSTRAP_BIN" ]; then
        [ -f "$BOOTSTRAP_BIN" ] || die "bootstrap binary not found: $BOOTSTRAP_BIN"
        install -m 0755 -o root -g root "$BOOTSTRAP_BIN" "$BIN"
        FRESH_BOOTSTRAP=1
        return 0
    fi
    TMP="$(mktemp -d)"
    local release="$BASE_URL/$REPO/releases/download"
    fetch "$release/channel-$CHANNEL/channel-$CHANNEL.json" "$TMP/channel.json"
    local manifest_url version
    manifest_url="$(jq -er .manifest_url "$TMP/channel.json")"
    version="$(jq -er .version "$TMP/channel.json")"
    case "$manifest_url" in "$release"/*) ;; *) die "channel pointer points outside $release" ;; esac
    local tail="${manifest_url#"$release"/}"
    case "$tail" in
        ""|*..*|*//*|*\\*|*\?*|*\#*|*@*|*%2[eEfF]*|*%5[cC]*|*%00*) die "channel pointer URL has traversal or odd characters" ;;
    esac
    printf '%s' "$version" | grep -Eq '^[A-Za-z0-9._+-]+$' || die "unexpected version string in channel pointer"
    fetch "$manifest_url" "$TMP/manifest.json"

    local name="hq-$version-$PLATFORM.tar.gz" want
    want="$(jq -er --arg n "$name" '.artifacts[] | select(.name == $n) | .sha256' "$TMP/manifest.json")" \
        || die "release $version has no $PLATFORM binary (see docs/UPDATE_SYSTEM.md for the platforms it publishes)"
    if can_verify; then
        fetch "$release/channel-$CHANNEL/channel-$CHANNEL.json.minisig" "$TMP/channel.json.minisig"
        verify_signature "$TMP/channel.json" "$TMP/channel.json.minisig" \
            || die "channel pointer signature does not verify against $PUBKEY_FILE"
        [ "$(jq -er .channel "$TMP/channel.json")" = "$CHANNEL" ] || die "channel pointer is for a different channel"
        [ "$(sha256sum "$TMP/manifest.json" | cut -d' ' -f1)" = "$(jq -er .manifest_sha256 "$TMP/channel.json")" ] \
            || die "manifest does not match the channel pointer"
        fetch "$manifest_url.minisig" "$TMP/manifest.json.minisig"
        verify_signature "$TMP/manifest.json" "$TMP/manifest.json.minisig" \
            || die "manifest signature does not verify against $PUBKEY_FILE"
        [ "$(jq -er .version "$TMP/manifest.json")" = "$version" ] || die "manifest version differs from the pointer"
    elif [ -n "$BOOTSTRAP_SHA256" ]; then
        want="$BOOTSTRAP_SHA256"
    else
        die "cannot verify the first download: install openssl or minisign, or pass --bootstrap-sha256 or --bootstrap-binary"
    fi

    fetch "${manifest_url%/*}/$name" "$TMP/$name"
    [ "$(sha256sum "$TMP/$name" | cut -d' ' -f1)" = "$want" ] || die "checksum mismatch for $name"
    [ "$(tar -tzf "$TMP/$name" | sed 's|^\./||')" = "hq" ] || die "$name must contain exactly one file named hq"
    mkdir "$TMP/x"
    tar -xzf "$TMP/$name" -C "$TMP/x" --no-same-owner
    [ -f "$TMP/x/hq" ] && [ ! -L "$TMP/x/hq" ] || die "$name does not contain a regular file named hq"
    install -m 0755 -o root -g root "$TMP/x/hq" "$BIN"
    FRESH_BOOTSTRAP=1
}

scaffold_vault() {
    [ -d "$HQ_HOME/.vault/_data" ] && return 0
    log "scaffolding the vault"
    runuser -u "$HQ_USER" -- env HOME="$HQ_HOME" HQ_VAULT_PATH="$HQ_HOME/.vault" \
        HQ_CONFIG_PATH="$HQ_HOME/config.yaml" "$BIN" install
}

enable_services() {
    systemctl enable hq.service
}

enable_timer() {
    if [ "$START" -eq 1 ]; then
        systemctl enable --now hq-update.timer
    else
        systemctl enable hq-update.timer
    fi
}

ensure_packages
ensure_sandbox
ensure_user_and_dirs
install_config
install_units
seed_web
if [ ! -x "$BIN" ] || [ -n "$BOOTSTRAP_BIN" ]; then
    bootstrap_binary
fi
scaffold_vault
enable_services
if [ "$START" -eq 1 ] && [ "$FRESH_BOOTSTRAP" -eq 1 ]; then
    log "first signature-verified update (first start can take a while: vault scaffolding and migrations)"
    # Reached only on a fresh bootstrap; a re-run does not reinstall over a working host.
    if ! HQ_UPDATE_HEALTH_TRIES=24 "$BIN" update --apply --force; then
        echo "install.sh: the first update did not complete. The host is configured but may not be running." >&2
        echo "Check: journalctl -u hq ; $BIN update --check ; then re-run this script or: $BIN update --apply --force" >&2
        exit 1
    fi
fi
enable_timer
log "done. Status: $BIN update --check ; systemctl list-timers hq-update.timer"
