#!/usr/bin/env bash
# Turns a fresh Ubuntu 24.04 server into a private, self-updating Agent HQ.
# Run as root (cloud-init does). Safe to re-run: every step checks before it changes anything.
#
#   bash bootstrap.sh --repo OWNER/REPO --ref <40-hex commit> --hostname NAME
#   bash bootstrap.sh --plan [same flags]     # list the steps; changes nothing, needs no root
#
# It installs HQ from signed releases, generates the web token and MCP key on this server,
# installs Tailscale without joining it, locks SSH to keys, and leaves the server reachable only
# over SSH until `hq-join` puts it on the owner's tailnet. See docs/HETZNER.md.
set -euo pipefail

STATE_DIR="/var/lib/hq-bootstrap"
SRC_DIR="$STATE_DIR/src"
STATUS_FILE="$STATE_DIR/status.json"
CONF_DIR="/etc/hq"
MCP_ENV="$CONF_DIR/mcp.env"
HEALTH_URL="http://127.0.0.1:5678/health"
HEALTH_WAIT_SECS=180
KEY_BYTES=32
TAILSCALE_KEYRING="/usr/share/keyrings/tailscale-archive-keyring.gpg"
# Primary fingerprint of the Tailscale package signing key for Ubuntu noble.
TAILSCALE_KEY_FPR="2596A99EAAB33821893C0A79458CA832957F5868" # public Tailscale apt key fingerprint, not a secret; gitleaks:allow
TAILSCALE_BASE="https://pkgs.tailscale.com/stable/ubuntu"

repo="" ref="" hostname_arg="" plan=0 current_step="start"
die() { echo "bootstrap.sh: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --repo) [ $# -ge 2 ] || die "missing value for $1"; repo=$2; shift 2 ;;
        --ref) [ $# -ge 2 ] || die "missing value for $1"; ref=$2; shift 2 ;;
        --hostname) [ $# -ge 2 ] || die "missing value for $1"; hostname_arg=$2; shift 2 ;;
        --plan) plan=1; shift ;;
        *) die "unknown argument $1" ;;
    esac
done

[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || die "--repo OWNER/REPO is required"
[[ "$ref" =~ ^[0-9a-f]{40}$ ]] || die "--ref must be a full 40-character commit SHA"
[[ "$hostname_arg" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]] || die "--hostname must be a DNS label"

# Read before any step rewrites status.json, so a re-run after hq-join cannot undo the join.
joined=0
grep -qs '"state":"joined"' "$STATUS_FILE" && joined=1

write_status() { # state detail
    [ "$joined" -eq 0 ] || return 0
    mkdir -p "$STATE_DIR"
    printf '{"state":"%s","step":"%s","detail":"%s","at":"%s"}\n' \
        "$1" "$current_step" "$2" "$(date -u +%FT%TZ)" > "$STATUS_FILE.tmp"
    mv "$STATUS_FILE.tmp" "$STATUS_FILE"
}

# In --plan mode a phase prints its description and its function is never called.
phase() { # description function
    current_step="$1"
    if [ "$plan" -eq 1 ]; then echo "[plan] $1"; return 0; fi
    echo "==> $1"
    write_status running ""
    "$2"
}

check_host() {
    [ "$(id -u)" -eq 0 ] || die "run as root"
    # shellcheck disable=SC1091
    . /etc/os-release
    [ "${ID:-}" = ubuntu ] && [ "${VERSION_ID:-}" = "24.04" ] || die "Ubuntu 24.04 is required (found ${PRETTY_NAME:-unknown})"
}

harden_ssh() {
    # Refuse to turn off passwords when no key is installed, which would lock everyone out.
    grep -qs '^ssh-\|^ecdsa-\|^sk-' /root/.ssh/authorized_keys || die "no SSH key in /root/.ssh/authorized_keys; create the server with an SSH key"
    cat > /etc/ssh/sshd_config.d/10-hq-hardening.conf <<'CONF'
PasswordAuthentication no
KbdInteractiveAuthentication no
PermitRootLogin prohibit-password
CONF
    # Ubuntu 24.04 starts sshd lazily through its socket, so the privilege separation directory may not exist yet.
    mkdir -p /run/sshd
    sshd -t
    systemctl try-reload-or-restart ssh
}

install_deps() {
    export DEBIAN_FRONTEND=noninteractive
    # A fresh boot often has unattended-upgrades holding the dpkg lock; wait for it instead of failing.
    echo 'DPkg::Lock::Timeout "600";' > /etc/apt/apt.conf.d/99-hq-lock-timeout
    apt-get update -qq
    apt-get install -y -qq curl ca-certificates jq tar gnupg ufw openssl
}

fetch_source() {
    [ -f "$SRC_DIR/.ref" ] && [ "$(cat "$SRC_DIR/.ref")" = "$ref" ] && return 0
    rm -rf "$SRC_DIR"
    mkdir -p "$SRC_DIR"
    # A commit SHA is content addressed, so this tarball cannot change under us.
    curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 \
        "https://codeload.github.com/$repo/tar.gz/$ref" | tar -xz --strip-components=1 -C "$SRC_DIR"
    echo "$ref" > "$SRC_DIR/.ref"
}

install_hq() {
    bash "$SRC_DIR/deploy/install.sh" --repo "$repo" --channel stable --pubkey "$SRC_DIR/release/minisign.pub"
}

write_secrets() {
    # Generated here and only here: user-data, the Hetzner API and the wizard never hold either value.
    # /etc/hq is root-owned, unlike /opt/hq, so HQ's own agents cannot replace this file.
    install -d -m 0755 "$CONF_DIR"
    if [ ! -f "$MCP_ENV" ]; then
        (
            umask 077
            echo "AGENTHQ_API_KEY=$(openssl rand -hex "$KEY_BYTES")"
            echo "HQ_WEB_AUTH_TOKEN=$(openssl rand -hex "$KEY_BYTES")"
        ) > "$MCP_ENV"
    fi
    chmod 0600 "$MCP_ENV"
    install -d -m 0755 /etc/systemd/system/hq.service.d
    printf '[Service]\nEnvironmentFile=%s\n' "$MCP_ENV" > /etc/systemd/system/hq.service.d/mcp-key.conf
    systemctl daemon-reload
}

install_tailscale() {
    local tmp
    tmp="$(mktemp)"
    curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 -o "$tmp" "$TAILSCALE_BASE/noble.noarmor.gpg"
    gpg --show-keys --with-colons "$tmp" | awk -F: '/^fpr/ {print $10; exit}' | grep -qx "$TAILSCALE_KEY_FPR" \
        || { rm -f "$tmp"; die "Tailscale signing key fingerprint changed; refusing to trust it"; }
    install -m 0644 "$tmp" "$TAILSCALE_KEYRING"
    rm -f "$tmp"
    echo "deb [signed-by=$TAILSCALE_KEYRING] $TAILSCALE_BASE noble main" > /etc/apt/sources.list.d/tailscale.list
    apt-get update -qq
    DEBIAN_FRONTEND=noninteractive apt-get install -y -qq tailscale
    systemctl enable --now tailscaled
}

configure_firewall() {
    [ "$joined" -eq 0 ] || return 0 # a re-run must not reopen SSH after hq-join
    ufw default deny incoming
    ufw default allow outgoing
    ufw allow 22/tcp comment 'SSH until hq-join'
    ufw allow 41641/udp comment 'Tailscale'
    ufw --force enable
}

install_join_helper() {
    install -m 0755 "$SRC_DIR/deploy/hetzner/hq-join.sh" /usr/local/sbin/hq-join
    echo "$hostname_arg" > "$STATE_DIR/hostname"
    [ "$joined" -eq 0 ] || return 0
    printf '\nAgent HQ is installed but private. Run:  sudo hq-join\n\n' > /etc/motd
}

wait_for_health() {
    local waited=0
    systemctl restart hq
    until curl -fsS "$HEALTH_URL" > /dev/null 2>&1; do
        waited=$((waited + 5))
        [ "$waited" -lt "$HEALTH_WAIT_SECS" ] || die "HQ did not answer $HEALTH_URL within ${HEALTH_WAIT_SECS}s (see: journalctl -u hq)"
        sleep 5
    done
}

on_exit() { # die() exits without tripping ERR, so record failure from EXIT instead
    local code=$?
    [ "$code" -eq 0 ] || [ "$plan" -eq 1 ] || write_status failed "see /var/log/cloud-init-output.log"
}

[ "$plan" -eq 1 ] || check_host
trap on_exit EXIT
phase "Check the host is Ubuntu 24.04 and running as root" true
phase "Install apt dependencies" install_deps
phase "Fetch the pinned source at commit $ref" fetch_source
phase "Install HQ from signed releases with deploy/install.sh" install_hq
phase "Generate the web token and MCP key on this server (mode 0600)" write_secrets
phase "Install Tailscale from the signed apt repository (not joined)" install_tailscale
phase "Close all inbound ports except SSH and Tailscale" configure_firewall
phase "Restrict SSH to keys" harden_ssh
phase "Install the hq-join helper" install_join_helper
phase "Wait for HQ to answer its health check" wait_for_health
current_step="done"
[ "$plan" -eq 1 ] || write_status ready "waiting for hq-join over SSH"
