#!/bin/bash
# Runs inside the systemd container that run.sh starts. The hub is the real one (units, applier,
# nftables, kernel WireGuard) set up with the real `hq net` commands, and the device under test is
# the real `hq net tunnel run` in its own network namespace, driven the way the phone app will drive
# it: the key on its stdin, control lines after it, a status file read back. Three more devices
# (b linked to the phone, c not linked, and the hub itself) are the other ends.
#
# Every check prints PASS or FAIL; the exit status is the number of FAILs.
#   FAST=1       shorten the rekey stream to one rekey
#   SOAK=1       after the checks, run the long soak (DURATION seconds, default 3600) with delay and loss
set -u
. /hub-lib.sh

REKEY_SECS=${REKEY_SECS:-400}
[ -n "${FAST:-}" ] && REKEY_SECS=150
SOAK_SECS=${DURATION:-3600}
PHONE_HOME=/srv/phone
TD=$PHONE_HOME/tunnel
ENDPOINT=192.168.1.1:51820
SERVE_PORT=4751
STRICT_PORT=4752
ECHO_PORT=4700
PEER_ECHO_PORT=4701
REACH_PORT=4801
REACH_BLOCKED_PORT=4802
FLOOD_SECS=30
# Seconds a device may take to come back after something broke. A kernel peer that was forgotten
# notices after its keepalive, so these are generous; a result prints how long it took.
RECOVER_SECS=40

asu() { local u=$1; shift; setpriv --reuid="$u" --regid="$u" --init-groups env HOME="/home/$u" "$@"; }
hqhub() { asu hq hq net "$@"; }
hqphone() { ip netns exec phone setpriv --reuid=phone --regid=phone --init-groups env HOME="$PHONE_HOME" hq net tunnel --tunnel-dir "$TD" "$@"; }
export -f hqphone
export TD PHONE_HOME
in_ns() { local ns=$1; shift; if [ "$ns" = host ]; then "$@"; else ip netns exec "$ns" "$@"; fi; }
probe() { in_ns "$1" helper.py probe "$2" "$3" "${4:-3}" >/dev/null 2>&1; }
status_field() { python3 -c "import json,sys;v=json.load(open('$TD/status.json'))['$1'];print('' if v is None else v)" 2>/dev/null; }
# wait_for DESCRIPTION SECONDS COMMAND...: passes at once when the command does, and says how long it took.
wait_for() {
  local what=$1 secs=$2 start=$SECONDS; shift 2
  while [ $((SECONDS - start)) -lt "$secs" ]; do
    if "$@" >/dev/null 2>&1; then pass "$what (after $((SECONDS - start)) s)"; return 0; fi
    sleep 1
  done
  fail "$what (not within $secs s)"; return 1
}
state_is() { [ "$(status_field state)" = "$1" ]; }
echoes() { probe "$1" "$2" "$3" 3; }
never_answers() { ! probe "$1" "$2" "$3" "${4:-4}"; }
counter() { status_field "$1"; }

# --- the supervisor: what the app does with the child
PHONE_PID=
phone_start() {
  rm -f "$PHONE_HOME/ctl"; mkfifo "$PHONE_HOME/ctl"; chown phone:phone "$PHONE_HOME/ctl"
  # The subshell becomes the process it starts (each step execs the next), so $! is hq's own pid.
  ( exec ip netns exec phone setpriv --reuid=phone --regid=phone --init-groups env HOME="$PHONE_HOME" hq net tunnel --tunnel-dir "$TD" run ) < "$PHONE_HOME/ctl" > "$PHONE_HOME/run.out" 2>&1 &
  PHONE_PID=$!
  exec 7<>"$PHONE_HOME/ctl"
  cat "$TD/key" >&7
}
ctl() { echo "$1" >&7; }
phone_wait() { wait "$PHONE_PID" 2>/dev/null; local rc=$?; exec 7>&-; PHONE_PID=; return $rc; }
phone_quit() { ctl quit; phone_wait; }
phone_kill() { kill -9 "$PHONE_PID" 2>/dev/null; phone_wait >/dev/null; }

echo "systemd: $(systemctl --version | head -1)   kernel: $(uname -r)"
install -m 0755 /out/hq /usr/local/bin/hq
install -m 0755 /helper.py /usr/local/bin/helper.py
useradd -u 1000 -m hq 2>/dev/null
useradd -u 1001 -m -d "$PHONE_HOME" phone 2>/dev/null
install -d -o phone -g phone -m 0700 "$PHONE_HOME"

echo "--- the hub, made with the real commands"
if ! hqhub init --endpoint "$ENDPOINT" > /tmp/init.out 2>&1; then cat /tmp/init.out; exit 1; fi
hubp=$(sed -n 's/^hub_public_key *= *"\(.*\)"/\1/p' /home/hq/.hq/net/net.toml)
check "hq net init made the hub's key and settings" test -n "$hubp"
install_hub /home/hq/.hq/net/hub.key
rm -f /home/hq/.hq/net/hub.key   # step 1 of the init output: the key leaves the HQ user's directory
sysctl -q -w net.ipv4.conf.all.rp_filter=0 net.ipv4.conf.default.rp_filter=0

i=0
for s in phone b c; do
  i=$((i + 1))
  ip netns add $s
  ip link add v-$s type veth peer name u-$s; ip link set u-$s netns $s
  ip addr add 192.168.$i.1/24 dev v-$s; ip link set v-$s up
  ip netns exec $s ip addr add 192.168.$i.2/24 dev u-$s
  ip netns exec $s ip link set u-$s up; ip netns exec $s ip link set lo up
done

systemctl daemon-reload
systemctl enable $APPLY.service $APPLY.path $APPLY.timer >/dev/null 2>&1
systemctl start wg-quick@hq0
sleep 6
systemctl start $APPLY.path $APPLY.timer
check "the hub is up" systemctl is-active --quiet wg-quick@hq0

echo "--- the phone: key, pairing, maps"
phone_pub=$(hqphone keygen | sed -n 's/^Public key: //p')
check "keygen printed a public key" test -n "$phone_pub"
hqhub peer add phone --pubkey "$phone_pub" --kind phone --keepalive 25 --no-qr > /tmp/add_phone.txt 2>&1
want "the phone got the first address" "$(grep -o 'at 10\.77\.0\.[0-9]*' /tmp/add_phone.txt | head -1 | cut -d' ' -f2)" 10.77.0.2
install -m 0600 -o phone -g phone /tmp/add_phone.txt "$PHONE_HOME/added.txt"
out=$(hqphone import "$PHONE_HOME/added.txt" 2>&1); rc=$?
want "import of a new hub waits for its key (exit status)" "$rc" 2
check "import with the hub's key succeeds" hqphone import "$PHONE_HOME/added.txt" --confirm-hub "$hubp"
for d in b c; do
  eval "k_$d=$(wg genkey)"; eval "pub_$d=$(eval echo \$k_$d | wg pubkey)"
  # wg genkey does not avoid the one public key in sixteen that looks like a private one.
  hqhub peer add $d --pubkey "$(eval echo \$pub_$d)" --kind laptop --no-qr --confirm-public > /tmp/add_$d.txt 2>&1
done
want "b's address" "$(grep -o 'at 10\.77\.0\.[0-9]*' /tmp/add_b.txt | head -1 | cut -d' ' -f2)" 10.77.0.3
want "c's address" "$(grep -o 'at 10\.77\.0\.[0-9]*' /tmp/add_c.txt | head -1 | cut -d' ' -f2)" 10.77.0.4
hqhub link add phone b --applier-updated > /tmp/link.out 2>&1
cat > "$TD/maps.toml" <<MAPS
[[serve]]
port = $SERVE_PORT
to = "127.0.0.1:$ECHO_PORT"
from = ["10.77.0.1", "10.77.0.3"]

[[serve]]
port = $STRICT_PORT
to = "127.0.0.1:$ECHO_PORT"
from = ["10.77.0.1"]

[[reach]]
listen = "127.0.0.1:$REACH_PORT"
to = "10.77.0.3:$PEER_ECHO_PORT"

[[reach]]
listen = "127.0.0.1:$REACH_BLOCKED_PORT"
to = "10.77.0.4:$PEER_ECHO_PORT"
MAPS
chown phone:phone "$TD/maps.toml"
check "hq net tunnel check accepts the join text, the maps and the key" hqphone check
n=1
for d in b c; do
  n=$((n + 1))
  eval "echo \$k_$d" > /tmp/$d.key
  ip netns exec $d ip link add wg0 type wireguard
  ip netns exec $d ip addr add 10.77.0.$((n + 1))/32 dev wg0
  ip netns exec $d wg set wg0 private-key /tmp/$d.key peer "$hubp" endpoint 192.168.$n.1:51820 allowed-ips 10.77.0.0/24 persistent-keepalive 5
  ip netns exec $d ip link set wg0 up
  ip netns exec $d ip route add 10.77.0.0/24 dev wg0 2>/dev/null
done
ip netns exec phone helper.py echo 127.0.0.1 $ECHO_PORT &
ip netns exec b helper.py echo 10.77.0.3 $PEER_ECHO_PORT &
ip netns exec c helper.py echo 10.77.0.4 $PEER_ECHO_PORT &
sleep 2

echo "--- 1. the phone starts, shakes hands and reports it"
phone_start
wait_for "the status says up" 15 state_is up
check "hq net tunnel status agrees" bash -c "hqphone status | grep -q 'state: up'"
check "the hub sees the phone's handshake" bash -c "wg show hq0 latest-handshakes | awk -v k=\"$phone_pub\" '\$1 == k && \$2 > 0 {found = 1} END {exit !found}'"

echo "--- 2. the serve map"
wait_for "the hub reaches the phone through the tunnel" 15 echoes host 10.77.0.2 $SERVE_PORT
wait_for "b, linked and listed, reaches it" 15 echoes b 10.77.0.2 $SERVE_PORT
sleep 6
check "the status counted the connections" bash -c "[ \"$(counter accepted)\" -ge 2 ]"

echo "--- 3. the two layers that keep others out"
# c has to be up for "gets nothing" to mean anything.
if warm c; then pass "c has a handshake with the hub"; else fail "c has no handshake, so 'gets nothing' proves nothing"; fi
wait_for "the hub reaches c's own echo, so c is up" 15 echoes host 10.77.0.4 $PEER_ECHO_PORT
if never_answers c 10.77.0.2 $SERVE_PORT 4; then pass "c, not linked, gets nothing (the hub's firewall)"; else fail "c reached the phone"; fi
before=$(counter dropped)
check "b, linked but not listed for $STRICT_PORT, gets no answer at all" in_ns b helper.py silent 10.77.0.2 $STRICT_PORT 3
echoes host 10.77.0.2 $STRICT_PORT && pass "the hub, listed, still reaches $STRICT_PORT" || fail "the hub cannot reach $STRICT_PORT"
sleep 6
check "the peer's own fence counted the drop" bash -c "[ \"$(counter dropped)\" -gt \"${before:-0}\" ]"

echo "--- 4. the reach map"
wait_for "a loopback listener on the phone reaches b through the hub" 15 echoes phone 127.0.0.1 $REACH_PORT
if never_answers phone 127.0.0.1 $REACH_BLOCKED_PORT 5; then pass "the reach entry towards c, not linked, gets nothing"; else fail "the phone reached c"; fi

echo "--- 5. the phone moves to another network address"
ip netns exec phone ip addr del 192.168.1.2/24 dev u-phone
ip netns exec phone ip addr add 192.168.1.9/24 dev u-phone
sleep 3
if never_answers host 10.77.0.2 $SERVE_PORT 5; then pass "before the control line the old path is dead"; else fail "the path survived the address change"; fi
ctl network-changed
wait_for "after network-changed the phone is reachable again" $RECOVER_SECS echoes host 10.77.0.2 $SERVE_PORT
check "the event log recorded the change" grep -q "network changed" "$TD/events.log"

echo "--- 6. the phone's link goes away for a while"
w0=$(counter wakeups)
ip netns exec phone ip link set u-phone down
ctl network-changed
sleep 20
w1=$(counter wakeups)
check "no spin while there is no network ($((w1 - w0)) wakeups in 20 s)" bash -c "[ -n '$w0' ] && [ -n '$w1' ] && [ $((w1 - w0)) -lt 40 ]"
ip netns exec phone ip link set u-phone up
ip netns exec phone ip addr add 192.168.1.9/24 dev u-phone 2>/dev/null
ip netns exec phone ip route add 192.168.1.0/24 dev u-phone 2>/dev/null
ctl network-changed
wait_for "the phone is reachable again" $RECOVER_SECS echoes host 10.77.0.2 $SERVE_PORT
wait_for "the status says up" 10 state_is up

echo "--- 7. the hub revokes the phone, then admits it again"
before=$(peers)
hqhub peer revoke phone > /tmp/revoke.out 2>&1
wait_for "the applier removed the peer" 15 bash -c "[ \"\$(wg show hq0 peers | wc -l)\" -lt $before ]"
# The phone's session is up to two minutes old, then its rekey goes unanswered for 30 seconds.
wait_for "the phone ends up down" 200 state_is down
hqhub peer add phone2 --pubkey "$phone_pub" --kind phone --keepalive 25 --no-qr > /tmp/add_phone2.txt 2>&1
want "the new entry got a new address; the old one is never reused" "$(grep -o 'at 10\.77\.0\.[0-9]*' /tmp/add_phone2.txt | head -1 | cut -d' ' -f2)" 10.77.0.5
hqhub link add phone2 b > /tmp/link2.out 2>&1
install -m 0600 -o phone -g phone /tmp/add_phone2.txt "$PHONE_HOME/added2.txt"
out=$(hqphone import "$PHONE_HOME/added2.txt" 2>&1); rc=$?
want "the same hub needs no confirmation (exit status)" "$rc" 0
check "and keeps the maps" test -s "$TD/maps.toml"
phone_quit
phone_start
wait_for "the phone is up on its new address" 20 state_is up
wait_for "the hub reaches the new address" 20 echoes host 10.77.0.5 $SERVE_PORT
wait_for "b reaches it" 20 echoes b 10.77.0.5 $SERVE_PORT
SERVE_ADDR=10.77.0.5

echo "--- 8. the supervisor's view: killed, restarted, stopped, abandoned"
phone_kill
check "a fresh status still reads up right after the kill" bash -c "hqphone status | grep -q 'state: up'"
# The peer promises a rewrite at least every max_age_s (30 with this keepalive); the command allows
# 10 s more before it calls the peer gone.
sleep 45
check "45 seconds later it says the peer is not running" bash -c "hqphone status | grep -q 'not running'"
phone_start
wait_for "restarted, it is up again" 20 state_is up
phone_quit; rc=$?
want "quit ends it cleanly (exit status)" "$rc" 0
want "and the status says stopped" "$(status_field state)" stopped
phone_start
wait_for "up once more" 20 state_is up
exec 7>&-   # the app goes away: the end of its input
phone_wait; rc=$?
want "the end of its input stops it (exit status)" "$rc" 0
check "and it says why" grep -q -- "--no-control" "$PHONE_HOME/run.out"
phone_start
wait_for "and it can be started again" 20 state_is up

echo "--- 9. the hub restarts"
systemctl restart wg-quick@hq0
systemctl restart $APPLY.service
# The restarted hub knows no endpoint for the phone, so the phone has to notice and shake hands
# again. A phone with traffic of its own notices when an answer fails to come (about 15 seconds). An
# idle one does not: its keepalives are not data, here as in the kernel, so it notices when its
# session expires, about three minutes. The traffic below stands in for the app using the reach map.
( for _ in $(seq 1 75); do probe phone 127.0.0.1 $REACH_PORT 2; sleep 1; done ) &
traffic=$!
wait_for "the phone is reachable after the hub came back" 60 echoes host $SERVE_ADDR $SERVE_PORT
wait $traffic

echo "--- 10. a flood from an address that is not the hub's endpoint"
port=$(ip netns exec phone ss -H -uan | awk '{print $4}' | awk -F: '{print $NF}' | head -1)
check "found the phone's UDP port" test -n "$port"
pid=$PHONE_PID
cpu() { awk '{print $14 + $15}' /proc/$pid/stat; }
ticks=$(getconf CLK_TCK)
# The same stream without a flood first: the stream's own lines wake the peer, so the flood is
# judged against that, not against zero. The status is rewritten every 5 s, hence the sleeps.
# UdpNoPorts counts datagrams that reached the host and found no socket that matched: the connected
# socket does not match a stranger's source, so this is how the flood is seen to arrive.
noports() { ip netns exec phone nstat -az UdpNoPorts | awk '/UdpNoPorts/ {print $2}'; }
kill -0 "$pid" 2>/dev/null || fail "the peer is not running, so the flood checks prove nothing"
sleep 6; wb0=$(counter wakeups)
helper.py stream $SERVE_ADDR $SERVE_PORT $FLOOD_SECS > /dev/null
sleep 6; wb1=$(counter wakeups)
c0=$(cpu); np0=$(noports)
helper.py flood 192.168.1.9 "$port" $FLOOD_SECS >/tmp/flood.out &
flood_pid=$!
w0=$(counter wakeups)
summary=$(helper.py stream $SERVE_ADDR $SERVE_PORT $FLOOD_SECS)
wait $flood_pid
c1=$(cpu); np1=$(noports)
sleep 6; w1=$(counter wakeups)
echo "  flood sent $(cat /tmp/flood.out) datagrams; stream: $summary"
check "the flood reached the host and was turned away by the socket (UdpNoPorts $np0 then $np1)" bash -c "[ -n '$np0' ] && [ -n '$np1' ] && [ $((np1 - np0)) -gt 1000 ]"
want "the stream saw no error during the flood" "$(echo "$summary" | python3 -c 'import json,sys;print(json.load(sys.stdin)["errors"])')" 0
check "the peer used under a tenth of a core ($(( (c1 - c0) * 1000 / ticks )) ms of CPU in $FLOOD_SECS s)" bash -c "[ $(( (c1 - c0) * 10 / ticks )) -lt $FLOOD_SECS ]"
check "the kernel's filter kept it asleep ($((w1 - w0)) wakeups with the flood, $((wb1 - wb0)) without)" bash -c "[ -n '$w0' ] && [ -n '$w1' ] && [ -n '$wb0' ] && [ -n '$wb1' ] && [ $((w1 - w0)) -le $((wb1 - wb0 + 30)) ]"

# From the hub's own address AND port the kernel lets the datagrams through, and the peer's receive
# path takes them: at most 128 a wake, and boringtun turns them away. This is the cost the threat
# model admits to (T33): the peer stays up and the stream keeps going. The CPU is printed, not
# judged: one loop thread saturates a core under this flood (task: throttle refusals).
SPOOF_SECS=15
c0=$(cpu)
helper.py spoof 192.168.1.9 "$port" 192.168.1.1 51820 $SPOOF_SECS >/tmp/spoof.out &
spoof_pid=$!
summary=$(helper.py stream $SERVE_ADDR $SERVE_PORT $SPOOF_SECS)
wait $spoof_pid
c1=$(cpu)
echo "  spoofed flood sent $(cat /tmp/spoof.out) datagrams from the endpoint's address and port; stream: $summary; the peer used $(( (c1 - c0) * 1000 / ticks )) ms of CPU in $SPOOF_SECS s"
check "datagrams were sent" bash -c "[ "$(cat /tmp/spoof.out)" -gt 1000 ]"
want "the stream saw no error during the spoofed flood" "$(echo "$summary" | python3 -c 'import json,sys;print(json.load(sys.stdin)["errors"])')" 0
kill -0 "$pid" 2>/dev/null && pass "the peer is still running" || fail "the peer died under the spoofed flood"
wait_for "and still answers" 15 echoes host $SERVE_ADDR $SERVE_PORT

echo "--- 11. one connection held across rekeys ($REKEY_SECS s)"
hs() { wg show hq0 latest-handshakes | awk -v k="$phone_pub" '$1 == k {print $2}'; }
helper.py stream $SERVE_ADDR $SERVE_PORT $REKEY_SECS > /tmp/stream_serve.json &
sp=$!
ip netns exec phone helper.py stream 127.0.0.1 $REACH_PORT $REKEY_SECS > /tmp/stream_reach.json &
rp=$!
seen=""; last=""
end=$((SECONDS + REKEY_SECS))
while [ $SECONDS -lt $end ]; do
  now=$(hs); [ "$now" != "$last" ] && { seen="$seen $now"; last=$now; }
  sleep 5
done
wait $sp $rp
rekeys=$(( $(echo $seen | wc -w) - 1 ))
want_min=$((REKEY_SECS / 150))
check "the session was renewed at least $want_min time(s) ($rekeys seen)" bash -c "[ $rekeys -ge $want_min ]"
for which in serve reach; do
  echo "  $which stream: $(cat /tmp/stream_$which.json)"
  want "no error on the $which stream" "$(python3 -c "import json;print(json.load(open('/tmp/stream_$which.json'))['errors'])")" 0
  check "no stall over 2 s on the $which stream" bash -c "python3 -c \"import json,sys;sys.exit(0 if json.load(open('/tmp/stream_$which.json'))['worst_ms'] < 2000 else 1)\""
done

echo "--- 12. what the process holds"
sleep 6
pid=$PHONE_PID
kill -0 "$pid" 2>/dev/null || fail "the peer is not running, so what follows proves nothing"
key=$(cat "$TD/key"); keyhex=$(echo "$key" | base64 -d | od -An -tx1 | tr -d ' \n')
leaks=0
look() { # look NAME CONTENT: fails if the key shows, base64 or hex
  case "$2" in *"$key"*|*"$keyhex"*) fail "$1 holds the key"; leaks=$((leaks + 1));; esac
}
look "the command line" "$(tr '\0' ' ' < /proc/$pid/cmdline)"
look "the environment" "$(tr '\0' '\n' < /proc/$pid/environ)"
look "the descriptor targets" "$(ls -l /proc/$pid/fd 2>/dev/null)"
look "the output" "$(cat "$PHONE_HOME/run.out")"
for f in "$TD"/*; do [ "$f" = "$TD/key" ] || look "$(basename "$f")" "$(cat "$f")"; done
[ "$leaks" -eq 0 ] && pass "the key is in no argument, variable, output or file but its own"
check "the status reports memory and descriptors" bash -c "[ -n \"$(status_field rss_kb)\" ] && [ -n \"$(status_field open_fds)\" ]"
echo "  rss $(status_field rss_kb) KB, $(status_field open_fds) descriptors, $(status_field wakeups) wakeups"

if [ -n "${SOAK:-}" ]; then
  echo "--- soak: $SOAK_SECS s with 170 ms round trip and 1 percent loss"
  for dev in "host v-phone" "phone u-phone"; do
    set -- $dev; in_ns "$1" tc qdisc add dev "$2" root netem delay 85ms 15ms loss 0.5%
    if in_ns "$1" tc qdisc show dev "$2" | grep -q netem; then pass "netem is on $2"; else fail "netem is not on $2, so the soak has no delay or loss"; fi
  done
  helper.py stream $SERVE_ADDR $SERVE_PORT $SOAK_SECS > /tmp/soak_stream.json &
  sp=$!
  log=/tmp/soak.log; : > $log
  probes=0; failed=0; first_rss=""; max_rss=0; first_fds=""; end=$((SECONDS + SOAK_SECS)); next_sample=$SECONDS
  while [ $SECONDS -lt $end ]; do
    for kind in "host $SERVE_ADDR $SERVE_PORT" "phone 127.0.0.1 $REACH_PORT"; do
      set -- $kind; probes=$((probes + 1))
      if ! probe "$1" "$2" "$3" 5; then failed=$((failed + 1)); echo "$(date +%T) probe failed: $kind" >> $log; fi
    done
    if [ $SECONDS -ge $next_sample ]; then
      rss=$(status_field rss_kb); fds=$(status_field open_fds)
      [ -z "$first_rss" ] && [ $((SOAK_SECS - (end - SECONDS))) -ge 120 ] && { first_rss=$rss; first_fds=$fds; }
      [ "${rss:-0}" -gt "$max_rss" ] && max_rss=$rss
      echo "$(date +%T) rss=$rss fds=$fds wakeups=$(counter wakeups) handshake_age=$(status_field last_handshake_age_s) state=$(status_field state)" >> $log
      next_sample=$((SECONDS + 60))
    fi
    sleep 10
  done
  wait $sp
  tail -5 $log
  echo "  stream: $(cat /tmp/soak_stream.json)   probes: $probes, failed: $failed"
  want "no reset on the held connection" "$(python3 -c "import json;print(json.load(open('/tmp/soak_stream.json'))['errors'])")" 0
  check "under 2 percent of the probes failed" bash -c "[ $((failed * 50)) -le $probes ]"
  check "memory grew by under 2 MB after the first two minutes (first ${first_rss:-?}, max $max_rss KB)" bash -c "[ -n '$first_rss' ] && [ $((max_rss - ${first_rss:-0})) -lt 2048 ]"
  check "descriptors stayed flat (first ${first_fds:-?}, last $(status_field open_fds))" bash -c "[ -n '$first_fds' ] && [ $(( $(status_field open_fds) - ${first_fds:-0} )) -le 2 ]"
  cp $log /results/soak.log
fi

phone_quit >/dev/null 2>&1
cp /tmp/stream_*.json /tmp/flood.out /results/ 2>/dev/null
echo "--- $fails failure(s)"
exit $fails
