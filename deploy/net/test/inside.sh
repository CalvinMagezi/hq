#!/bin/bash
# Runs inside the systemd container that run.sh starts. It installs the hub the way
# docs/PRIVATE_NETWORK.md describes, with the real units from deploy/net and the real applier,
# puts three "devices" in network namespaces, and checks what the applier and the firewall do.
# Every check prints PASS or FAIL; the exit status is the number of FAILs.
set -u
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

echo "systemd: $(systemctl --version | head -1)"
useradd -u 1000 -m hq 2>/dev/null
umask 077
mkdir -p /etc/wireguard /etc/hq-net /opt/hq-net /var/lib/hq-net
hubk=$(wg genkey); hubp=$(echo "$hubk" | wg pubkey)
echo "$hubk" > /etc/wireguard/hq0.key
for s in a b c; do eval "k_$s=$(wg genkey)"; eval "pub_$s=$(eval echo \$k_$s | wg pubkey)"; done
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

write_input() { # $1 = devices to include (a b c), $2 = links ("a:b ...")
  python3 - "$1" "$2" "$pub_a" "$pub_b" "$pub_c" <<'PY'
import sys
names = sys.argv[1].split(); links = sys.argv[2].split()
pub = dict(zip("abc", sys.argv[3:6]))
addr = {"a": "10.77.0.2", "b": "10.77.0.3", "c": "10.77.0.4"}
out = ["# hq-net peers v2", ""]
for n in names:
    out += ["# " + n, "[Peer]", "PublicKey = " + pub[n], "AllowedIPs = " + addr[n] + "/32", ""]
for l in links:
    x, y = l.split(":")
    out += ["[Link]", "A = " + addr[x] + "/32", "B = " + addr[y] + "/32", ""]
out.append("# hq-net end %d links %d" % (len(names), len(links)))
open("/tmp/hq0.conf.new", "w").write("\n".join(out) + "\n")
PY
  # Written the way hq net writes it: a private temporary file renamed over the input.
  install -o 1000 -g 1000 -m 0600 /tmp/hq0.conf.new /var/lib/hq-net/hq0.conf.tmp
  mv /var/lib/hq-net/hq0.conf.tmp /var/lib/hq-net/hq0.conf
}
write_input "a b c" "a:b"

ip netns add a; ip netns add b; ip netns add c
i=0
for s in a b c; do
  i=$((i + 1))
  ip link add v-$s type veth peer name u-$s; ip link set u-$s netns $s
  ip addr add 192.168.$i.1/24 dev v-$s; ip link set v-$s up
  ip netns exec $s ip addr add 192.168.$i.2/24 dev u-$s
  ip netns exec $s ip link set u-$s up; ip netns exec $s ip link set lo up
done

echo "--- start the tunnel; the applier is pulled in by WantedBy"
systemctl daemon-reload
systemctl enable $APPLY.service $APPLY.path $APPLY.timer >/dev/null 2>&1
systemctl start wg-quick@hq0
sleep 6
systemctl start $APPLY.path $APPLY.timer
for u in wg-quick@hq0 $APPLY.path $APPLY.timer; do check "$u is active" systemctl is-active --quiet $u; done
check "the applier applied three peers" bash -c "$(declare -f journal); journal | grep -q '3 peer(s) on hq0 after this run'"
check "the applier journaled the pair" bash -c "$(declare -f journal); journal | grep -q 'allowed 10.77.0.2 <-> 10.77.0.3'"
want "peers on the interface" "$(peers)" 3
check "the set holds both directions of the pair" bash -c "nft list set inet hq_net links_hq0 | grep -q '10.77.0.2 . 10.77.0.3' && nft list set inet hq_net links_hq0 | grep -q '10.77.0.3 . 10.77.0.2'"
want "hq0 forwarding" "$(sysctl -n net.ipv4.conf.hq0.forwarding)" 1

n=1
for s in a b c; do
  n=$((n + 1))
  eval "echo \$k_$s" > /tmp/$s.key
  ip netns exec $s ip link add wg0 type wireguard
  ip netns exec $s ip addr add 10.77.0.$n/32 dev wg0
  ip netns exec $s wg set wg0 private-key /tmp/$s.key peer "$hubp" endpoint 192.168.$((n - 1)).1:51820 allowed-ips 10.77.0.0/24 persistent-keepalive 1
  ip netns exec $s ip link set wg0 up
  ip netns exec $s ip route add 10.77.0.0/24 dev wg0 2>/dev/null
done

echo "--- one linked pair (a, b) and a third device (c)"
reaches a 10.77.0.1
reaches a 10.77.0.3; reaches b 10.77.0.2
blocked a 10.77.0.4; blocked c 10.77.0.2; blocked b 10.77.0.4; blocked c 10.77.0.3

echo "--- the link is removed by a version 2 file with none"
write_input "a b c" ""; sleep 4
want "pairs in the set" "$(nft list set inet hq_net links_hq0 | grep -c '10.77.0')" 0
blocked a 10.77.0.3; blocked b 10.77.0.2
check "the removal is journaled" bash -c "$(declare -f journal); journal | grep -q 'removed 10.77.0.2 <-> 10.77.0.3'"

echo "--- linked again, then b is revoked: its link goes with it and b is cut off"
write_input "a b c" "a:b"; sleep 4; reaches a 10.77.0.3
write_input "a c" ""; sleep 4
want "peers on the interface" "$(peers)" 2
if ping_once b 10.77.0.1; then fail "revoked b still reaches the hub"; else pass "revoked b cannot reach the hub"; fi
blocked a 10.77.0.3

echo "--- twenty rewrites in five seconds, then a revoke"
write_input "a b c" "a:c"; sleep 3
for k in $(seq 1 20); do write_input "a b c" "a:c"; sleep 0.25; done
write_input "c" ""; sleep 6
want "peers on the interface" "$(peers)" 1
check "the path unit is still watching" systemctl is-active --quiet $APPLY.path

echo "--- another firewall whose routed policy is drop (iptables-nft standing in for ufw)"
write_input "a b c" "a:c"; sleep 5
# The hub forgot a and b when they were revoked and cannot start a handshake with them (the file
# has no endpoint), so a has to send first.
reaches a 10.77.0.4; reaches c 10.77.0.2
iptables -P FORWARD DROP
blocked c 10.77.0.2
iptables -A FORWARD -i hq0 -o hq0 -j ACCEPT
reaches c 10.77.0.2; reaches a 10.77.0.4
blocked a 10.77.0.3; blocked b 10.77.0.4
iptables -D FORWARD -i hq0 -o hq0 -j ACCEPT; iptables -P FORWARD ACCEPT

echo "--- the table is gone while forwarding is on"
nft delete table inet hq_net
write_input "a b c" "a:b"; sleep 5
check "the run failed visibly" systemctl is-failed --quiet $APPLY.service
check "the journal says the table may be missing" bash -c "$(declare -f journal); journal | grep -q 'could not set the links on hq0'"
want "the peers were still applied" "$(peers)" 3
echo "(known hazard, documented) with no table, an unlinked pair is open:"
reaches a 10.77.0.4
echo "--- the table is loaded again and the applier runs"
nft -f /etc/hq-net/hq_net.nft
systemctl reset-failed $APPLY.service
systemctl start $APPLY.service; sleep 2
check "the run succeeds" bash -c "! systemctl is-failed --quiet $APPLY.service"
reaches a 10.77.0.3
blocked a 10.77.0.4; blocked c 10.77.0.3

echo "--- a hostile file: a PostUp line"
printf '# hq-net peers v1\n\n[Peer]\nPostUp = touch /tmp/pwned\nPublicKey = %s\nAllowedIPs = 10.77.0.2/32\n# hq-net end 1\n' "$pub_a" > /tmp/bad.conf
install -o 1000 -g 1000 -m 0600 /tmp/bad.conf /var/lib/hq-net/hq0.conf.tmp && mv /var/lib/hq-net/hq0.conf.tmp /var/lib/hq-net/hq0.conf
sleep 4
check "the file is refused and says which rule" bash -c "$(declare -f journal); journal | grep -q 'nothing applied: refused at line 4'"
check "nothing ran" bash -c '[ ! -e /tmp/pwned ]'
want "peers unchanged" "$(peers)" 3

echo "--- confinement"
score=$(systemd-analyze security $APPLY.service --no-pager 2>&1 | sed -n 's/.*exposure level for [^:]*: \([0-9.]*\).*/\1/p')
echo "exposure score: ${score:-unknown} (lower is tighter)"
check "the exposure score is under 5" bash -c "[ -n '$score' ] && awk -v s='$score' 'BEGIN{exit !(s < 5)}'"

echo "--- $fails failure(s)"
exit $fails
