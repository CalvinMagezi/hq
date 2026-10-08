#!/usr/bin/env bash
# Puts this server on your tailnet and publishes HQ there. Run over SSH as root, once:
#   sudo hq-join [--close-ssh]
# Safe to repeat. It prints the sign-in link at the end. Public SSH stays open in ufw by default
# so a lapsed tailnet key can be fixed; the Hetzner Cloud Firewall (your IP only) is what blocks it.
# Pass --close-ssh to also close it here once you have confirmed SSH works over the tailnet.
set -euo pipefail

HQ_HOME="/opt/hq"
STATE_DIR="/var/lib/hq-bootstrap"
MCP_ENV="/etc/hq/mcp.env"
CONFIG="$HQ_HOME/config.yaml"
HEALTH_URL="http://127.0.0.1:5678/health"
HEALTH_WAIT_SECS=180
SERVE_PORT=8443
UPSTREAM="http://127.0.0.1:5678"

close_ssh=0
die() { echo "hq-join: $*" >&2; exit 1; }
case "${1:-}" in
    "") ;;
    --close-ssh) close_ssh=1 ;;
    *) die "usage: hq-join [--close-ssh]" ;;
esac

[ "$(id -u)" -eq 0 ] || die "run as root (sudo hq-join)"
[ -f "$MCP_ENV" ] && [ -f "$STATE_DIR/hostname" ] || die "bootstrap has not finished; check $STATE_DIR/status.json"
grep -qs '"state":"\(ready\|joined\)"' "$STATE_DIR/status.json" || die "bootstrap is still running or failed; check $STATE_DIR/status.json"
ts_host="$(cat "$STATE_DIR/hostname")"

echo "==> Joining your tailnet. Open the link tailscale prints and sign in to your account."
tailscale up --hostname "$ts_host"

fqdn="$(tailscale status --json | jq -r '.Self.DNSName // empty' | sed 's/\.$//')"
[ -n "$fqdn" ] || die "no tailnet DNS name; enable MagicDNS in the Tailscale admin console and re-run"
origin="https://$fqdn:$SERVE_PORT"

echo "==> Allowing $origin as a browser origin"
tmp="$(mktemp)"
# Drop any earlier web_allowed_origins block, then append ours.
[ -f "$CONFIG" ] && awk '/^web_allowed_origins:/ {skip=1; next} skip && /^([[:space:]]|-|$)/ {next} {skip=0; print}' "$CONFIG" > "$tmp"
printf 'web_allowed_origins:\n  - "%s"\n' "$origin" >> "$tmp"
install -o hq -g hq -m 0600 "$tmp" "$CONFIG"
rm -f "$tmp"

echo "==> Publishing HQ to your tailnet only"
tailscale serve --bg --https="$SERVE_PORT" "$UPSTREAM"

systemctl restart hq
waited=0
until curl -fsS "$HEALTH_URL" > /dev/null 2>&1; do
    waited=$((waited + 2))
    [ "$waited" -lt "$HEALTH_WAIT_SECS" ] || die "HQ did not come back; see: journalctl -u hq"
    sleep 2
done

ufw allow in on tailscale0 to any port 22 proto tcp > /dev/null
if [ "$close_ssh" -eq 1 ]; then
    echo "==> Closing public SSH in ufw (still reachable over the tailnet)"
    ufw delete allow 22/tcp > /dev/null || true
fi

token="$(sed -n 's/^HQ_WEB_AUTH_TOKEN=//p' "$MCP_ENV")"
printf '{"state":"joined","step":"hq-join","detail":"%s","at":"%s"}\n' "$origin" "$(date -u +%FT%TZ)" > "$STATE_DIR/status.json"
: > /etc/motd
printf '\nOpen HQ (tailnet devices only):\n\n  %s/vault#token=%s\n\n' "$origin" "$token"
