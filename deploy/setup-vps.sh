#!/usr/bin/env bash
# One-time VPS provisioning for Agent-HQ
# Run: ssh <ssh-host> 'bash -s' < deploy/setup-vps.sh
set -euo pipefail

echo "=== Updating system ==="
apt update && apt upgrade -y

echo "=== Installing packages ==="
apt install -y ufw
# Linux OCR fallback (crates/hq-convert/src/ocr.rs) shells out to
# this binary for non-vision-model / non-vision-relay image handling.
apt install -y tesseract-ocr
# Scanned PDFs are rasterized with pdftoppm before tesseract reads them.
apt install -y poppler-utils

if ! command -v caddy >/dev/null; then
  # Caddy isn't in Ubuntu's default apt repos — add its official one first.
  apt install -y debian-keyring debian-archive-keyring apt-transport-https curl
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --batch --yes --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list
  apt update
  apt install -y caddy
fi

echo "=== Installing GitHub CLI ==="
# HQ shells out to `gh` for `git_pr` and the coding tools. Authentication is
# a token in /opt/hq/gh.env; see deploy/README.md, "GitHub access".
mkdir -p /etc/apt/keyrings && chmod 755 /etc/apt/keyrings
curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg -o /etc/apt/keyrings/githubcli-archive-keyring.gpg
chmod go+r /etc/apt/keyrings/githubcli-archive-keyring.gpg
echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" > /etc/apt/sources.list.d/github-cli.list
apt update
apt install -y gh

echo "=== Installing Tailscale ==="
curl -fsSL https://tailscale.com/install.sh | sh

echo "=== Installing Herdr (coding-agent runtime) ==="
# Herdr verifies its own download against a SHA-256 from its release manifest.
curl -fsSL https://herdr.dev/install.sh | HERDR_INSTALL_DIR=/usr/local/bin sh

echo "=== Creating hq user and directories ==="
useradd -r -s /usr/sbin/nologin -d /opt/hq hq || echo "User hq already exists"
mkdir -p /opt/hq/.vault /opt/hq/data
chown -R hq:hq /opt/hq

# So the agent can read its own daemon logs (journalctl -u hq) instead of
# hitting "insufficient permissions".
usermod -aG systemd-journal hq

echo "=== Configuring firewall ==="
ufw default deny incoming
ufw default allow outgoing
ufw allow 22/tcp comment 'SSH'
ufw allow 80/tcp comment 'HTTP redirect'
ufw allow 443/tcp comment 'HTTPS'
# Tailscale needs UDP 41641
ufw allow 41641/udp comment 'Tailscale'
echo "y" | ufw enable

echo "=== Writing HQ config ==="
cat > /opt/hq/config.yaml << 'YAML'
vault_path: /opt/hq/.vault
ws_port: 5678
instance:
  instance_type: cloud
relay:
  discord_enabled: false
  telegram_enabled: false
YAML
chown hq:hq /opt/hq/config.yaml

echo "=== VPS provisioning complete ==="
echo "Next steps:"
echo "  1. Clone the repo on this host and run deploy/install.sh (see deploy/README.md,"
echo "     'Pull-based updates'). It installs the hq binary, units and updater."
echo "     Building from source instead: copy the binary to /usr/local/bin/hq and"
echo "     install deploy/hq.service and deploy/herdr.service by hand."
echo "  2. Run: tailscale up"
echo "  3. Install a Caddyfile from deploy/"
