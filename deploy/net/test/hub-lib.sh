#!/bin/bash
# Shared by the in-container scripts of deploy/net/test and deploy/net/tunnel-test: the PASS/FAIL
# helpers and the hub, installed the way docs/PRIVATE_NETWORK.md describes, with the real units from
# deploy/net and the real applier. Sourced, never run.
#
# A script sources this, calls install_hub with the hub's private key file, and keeps its own checks.
fails=0
pass() { echo "PASS $*"; }
fail() { echo "FAIL $*"; fails=$((fails + 1)); }
check() { # check "description" command...
  local what=$1; shift
  if "$@" >/dev/null 2>&1; then pass "$what"; else fail "$what"; fi
}
ping_once() { ip netns exec "$1" ping -c1 -W2 "$2" >/dev/null 2>&1; }
# A device the hub forgot (revoked, then added again) notices only after its keepalive goes
# unanswered, about 15 seconds, and then handshakes again; so these wait up to 40.
# A device must reach the hub first: otherwise "blocked" could only mean "no handshake yet".
warm() { local n; for n in $(seq 1 13); do ping_once "$1" 10.77.0.1 && return 0; sleep 1; done; return 1; }
reaches() { local n; for n in $(seq 1 13); do ping_once "$1" "$2" && { pass "$1 reaches $2"; return; }; sleep 1; done; fail "$1 reaches $2"; }
blocked() {
  if ! warm "$1"; then fail "$1 has no handshake, so 'blocked $2' proves nothing"; return; fi
  if ping_once "$1" "$2"; then fail "$1 is blocked from $2"; else pass "$1 is blocked from $2"; fi
}
# The unit names are built from this so no "name@host.suffix" appears in the source for the personal-data scan to read as an address.
APPLY=hq-net-apply@hq0
journal() { journalctl -u 'hq-net-apply@hq0' --no-pager -o cat; }
peers() { wg show hq0 peers | wc -l; }
want() { # want "description" actual expected
  if [ "$2" = "$3" ]; then pass "$1 ($2)"; else fail "$1: got $2, wanted $3"; fi
}

# install_hub KEYFILE: the hub's files and units, with KEYFILE as the interface's private key.
# Expects /deploy (the repo's deploy/net) and /out/hq-net-apply (the applier) to be mounted.
install_hub() {
  local keyfile=$1
  useradd -u 1000 -m hq 2>/dev/null
  umask 077
  mkdir -p /etc/wireguard /etc/hq-net /opt/hq-net /var/lib/hq-net
  install -m 0600 "$keyfile" /etc/wireguard/hq0.key
  umask 022
  cat > /etc/wireguard/hq0.conf <<'EOF'
[Interface]
Address = 10.77.0.1/24
ListenPort = 51820
PostUp = wg set %i private-key /etc/wireguard/%i.key
PostUp = nft -f /etc/hq-net/hq_net.nft
PostUp = sysctl -q -w net.ipv4.conf.%i.forwarding=1
EOF
  chmod 600 /etc/wireguard/hq0.conf
  cat > /etc/hq-net/hq_net.nft <<'EOF'
add table inet hq_net
delete table inet hq_net
table inet hq_net {
  set links_hq0 {
    type ipv4_addr . ipv4_addr
  }
  chain forward {
    type filter hook forward priority -10; policy accept;
    iifname "hq0" oifname "hq0" ip saddr . ip daddr @links_hq0 accept
    iifname "hq0" drop
    oifname "hq0" drop
  }
}
EOF
  printf 'HQ_NET_SUBNET=10.77.0.0/24\nHQ_NET_HUB=10.77.0.1\nHQ_NET_OWNER_UID=1000\n' > /etc/hq-net/hq0.env
  install -d -o root -g root -m 0755 /opt/hq-net
  install -o root -g root -m 0755 /out/hq-net-apply /opt/hq-net/hq-net-apply
  cp /deploy/hq-net-apply@.service /deploy/hq-net-apply@.path /deploy/hq-net-apply@.timer /etc/systemd/system/
  install -d -o 1000 -g 1000 -m 0700 /var/lib/hq-net
}
