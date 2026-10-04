#!/usr/bin/env bash
# Provisions a fresh Ubuntu LTS droplet as the isolated release runner.
# Run ON the droplet as root. See RUNBOOK.md.
#
#   bash provision.sh --repo OWNER/REPO [--admin-cidr CIDR] [--name NAME]   # installs everything, no registration
#   bash provision.sh --repo OWNER/REPO --register-only [--ephemeral]       # prompts for the token
#
# The registration token is never taken from argv, and config.sh gets it through
# ACTIONS_RUNNER_INPUT_TOKEN rather than --token so it does not show in `ps`. --register-only reads it
# from $RUNNER_TOKEN_FILE (a file you create with mode 600; it is deleted after
# use) or, if that is unset, prompts for it with echo off. It is handed to
# config.sh through the environment of one process and never writes it anywhere else. --ephemeral registers the runner to take exactly one job and
# then deregister (for a per-job or just-in-time model).
# The droplet holds no SSH keys for any other host, no tailnet, and no instance secrets.
set -euo pipefail

RUST_TOOLCHAIN="1.98.1"
RUSTUP_VERSION="1.29.0"
RUSTUP_SHA256="4acc9acc76d5079515b46346a485974457b5a79893cfb01112423c89aeb5aa10"
BUN_VERSION="1.3.14"
BUN_SHA256="951ee2aee855f08595aeec6225226a298d3fea83a3dcd6465c09cbccdf7e848f"
MINISIGN_VERSION="0.12"
MINISIGN_SHA256="9a599b48ba6eb7b1e80f12f36b94ceca7c00b7a5173c95c3efc88d9822957e73"
RUNNER_VERSION="2.337.0"
RUNNER_SHA256="70920811a4f8ad4328818682bca5c6469c1c942fab52448868071d0063816613"
RUNNER_LABEL="hq-release"
RUNNER_USER="runner"
RUNNER_HOME="/home/$RUNNER_USER"
RUNNER_DIR="$RUNNER_HOME/actions-runner"
TARGET_CAP_GB="40"
APT_DEPS=(build-essential pkg-config perl libssl-dev curl ca-certificates git jq unzip xz-utils gh cron ufw unattended-upgrades)

repo="" admin_cidr="" name="" register_only=0 ephemeral=0
die() { echo "provision.sh: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    case "$1" in
        --repo) [ $# -ge 2 ] || die "missing value for $1"; repo=$2; shift 2 ;;
        --admin-cidr) [ $# -ge 2 ] || die "missing value for $1"; admin_cidr=$2; shift 2 ;;
        --name) [ $# -ge 2 ] || die "missing value for $1"; name=$2; shift 2 ;;
        --register-only) register_only=1; shift ;;
        --ephemeral) ephemeral=1; shift ;;
        *) die "unknown argument $1" ;;
    esac
done

[ "$(id -u)" -eq 0 ] || die "run as root"
[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || die "--repo OWNER/REPO is required"
[ -z "$admin_cidr" ] || [[ "$admin_cidr" =~ ^[0-9a-fA-F:.]+/[0-9]+$ ]] || die "--admin-cidr must look like 203.0.113.7/32"
[ -n "$name" ] || name="release-$(hostname -s)"

fetch_verified() { # url sha256 dest
    curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 --output "$3" "$1"
    echo "$2  $3" | sha256sum -c - > /dev/null || die "checksum mismatch for $1"
}

read_token() {
    local token=""
    if [ -n "${RUNNER_TOKEN_FILE:-}" ]; then
        [ -f "$RUNNER_TOKEN_FILE" ] || die "RUNNER_TOKEN_FILE not found"
        [ "$(stat -c %a "$RUNNER_TOKEN_FILE")" = "600" ] || die "RUNNER_TOKEN_FILE must have mode 600"
        token=$(tr -d '[:space:]' < "$RUNNER_TOKEN_FILE")
        shred -u "$RUNNER_TOKEN_FILE" 2> /dev/null || rm -f "$RUNNER_TOKEN_FILE"
    else
        read -r -s -p "Runner registration token: " token < /dev/tty
        echo >&2
    fi
    [ -n "$token" ] || die "empty registration token"
    printf '%s' "$token"
}

register_runner() {
    if [ -f "$RUNNER_DIR/.runner" ]; then echo "provision.sh: runner already registered"; return 0; fi
    local token flags=""
    token=$(read_token)
    [ "$ephemeral" -eq 0 ] || flags="--ephemeral"
    # The token travels in the environment of this one child process only.
    printf '%s' "$token" | sudo -u "$RUNNER_USER" bash -c \
        "IFS= read -r ACTIONS_RUNNER_INPUT_TOKEN; export ACTIONS_RUNNER_INPUT_TOKEN; cd '$RUNNER_DIR' && ./config.sh --unattended --url 'https://github.com/$repo' --name '$name' --labels '$RUNNER_LABEL' --work _work --replace $flags"
    token=""
    systemctl enable --now hq-release-runner.service
}

if [ "$register_only" -eq 1 ]; then
    [ -d "$RUNNER_DIR" ] || die "runner not installed, run without --register-only first"
    register_runner
    exit 0
fi

# shellcheck disable=SC1091
. /etc/os-release
[ "$ID" = "ubuntu" ] || die "Ubuntu LTS only"

export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq --no-install-recommends "${APT_DEPS[@]}"

cat > /etc/apt/apt.conf.d/20auto-upgrades <<'APT'
APT::Periodic::Update-Package-Lists "1";
APT::Periodic::Unattended-Upgrade "1";
APT::Periodic::AutocleanInterval "7";
APT
systemctl enable --now unattended-upgrades > /dev/null

ufw --force reset > /dev/null
ufw default deny incoming
ufw default allow outgoing
if [ -n "$admin_cidr" ]; then
    ufw allow from "$admin_cidr" to any port 22 proto tcp
else
    echo "provision.sh: no --admin-cidr, SSH stays closed (use the provider console)" >&2
fi
ufw --force enable

id "$RUNNER_USER" > /dev/null 2>&1 || useradd --create-home --shell /bin/bash "$RUNNER_USER"
passwd --lock "$RUNNER_USER" > /dev/null

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
chmod 755 "$tmp"

fetch_verified "https://static.rust-lang.org/rustup/archive/$RUSTUP_VERSION/x86_64-unknown-linux-gnu/rustup-init" "$RUSTUP_SHA256" "$tmp/rustup-init"
chmod 755 "$tmp/rustup-init"
sudo -u "$RUNNER_USER" "$tmp/rustup-init" -y --no-modify-path --profile minimal --default-toolchain "$RUST_TOOLCHAIN"

fetch_verified "https://github.com/oven-sh/bun/releases/download/bun-v$BUN_VERSION/bun-linux-x64.zip" "$BUN_SHA256" "$tmp/bun.zip"
unzip -q -o "$tmp/bun.zip" -d "$tmp/bun"
install -m 755 "$tmp/bun/bun-linux-x64/bun" /usr/local/bin/bun

fetch_verified "https://github.com/jedisct1/minisign/releases/download/$MINISIGN_VERSION/minisign-$MINISIGN_VERSION-linux.tar.gz" "$MINISIGN_SHA256" "$tmp/minisign.tgz"
tar -xzf "$tmp/minisign.tgz" -C "$tmp"
install -m 755 "$tmp/minisign-linux/x86_64/minisign" /usr/local/bin/minisign

install -d -o "$RUNNER_USER" -g "$RUNNER_USER" "$RUNNER_DIR"
fetch_verified "https://github.com/actions/runner/releases/download/v$RUNNER_VERSION/actions-runner-linux-x64-$RUNNER_VERSION.tar.gz" "$RUNNER_SHA256" "$tmp/runner.tgz"
sudo -u "$RUNNER_USER" tar -xzf "$tmp/runner.tgz" -C "$RUNNER_DIR"

# The runner reads its environment from this file; the job steps find cargo here.
cat > "$RUNNER_DIR/.path" <<PATHFILE
$RUNNER_HOME/.cargo/bin:/usr/local/bin:/usr/bin:/bin
PATHFILE
chown "$RUNNER_USER:$RUNNER_USER" "$RUNNER_DIR/.path"

cat > /etc/systemd/system/hq-release-runner.service <<UNIT
[Unit]
Description=GitHub Actions release runner
After=network-online.target
Wants=network-online.target

[Service]
User=$RUNNER_USER
WorkingDirectory=$RUNNER_DIR
ExecStart=$RUNNER_DIR/run.sh
Restart=always
RestartSec=10
KillMode=process
KillSignal=SIGTERM
TimeoutStopSec=5min

NoNewPrivileges=true
PrivateTmp=true
ProtectSystem=full
ProtectHome=tmpfs
BindPaths=$RUNNER_HOME
ProtectKernelTunables=true
ProtectKernelModules=true
ProtectControlGroups=true
RestrictSUIDSGID=true
LockPersonality=true

[Install]
WantedBy=multi-user.target
UNIT
systemctl daemon-reload

cat > /usr/local/sbin/hq-runner-gc <<GC
#!/usr/bin/env bash
# Daily: when a cargo target dir under the runner workspace exceeds the cap,
# drop debug artifacts first, then everything. Skipped while a job is running.
set -euo pipefail
cap_kb=\$(( $TARGET_CAP_GB * 1024 * 1024 ))
pgrep -f Runner.Worker > /dev/null && exit 0
for t in $RUNNER_DIR/_work/*/*/target; do
    [ -d "\$t" ] || continue
    [ "\$(du -sk "\$t" | cut -f1)" -gt "\$cap_kb" ] || continue
    rm -rf "\$t/debug"
    [ "\$(du -sk "\$t" | cut -f1)" -gt "\$cap_kb" ] && rm -rf "\$t"
done
GC
chmod 755 /usr/local/sbin/hq-runner-gc
echo "17 4 * * * $RUNNER_USER flock -n /tmp/hq-runner-gc.lock /usr/local/sbin/hq-runner-gc" > /etc/cron.d/hq-runner-gc
chmod 644 /etc/cron.d/hq-runner-gc

echo "provision.sh: done"
if [ ! -f "$RUNNER_DIR/.runner" ]; then echo "Next: run register-only (RUNBOOK.md, "Register the runner")."; fi
