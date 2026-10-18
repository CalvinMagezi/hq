# Spike C: hub and device over a real network

Throwaway crate, outside the HQ workspace. It answers the questions in the plan's group N: does a
device reach the hub directly from a given network, how fast, and what happens when it cannot.
Spike A (`../spike`) proved the protocol on one machine; this one splits it into the two roles.

| Role | Runs on | What it does |
|---|---|---|
| `hub` | the server | Binds one fixed UDP port with a persistent key. Accepts only paired device ids, calls `host.status` down each live connection and prints round-trip times and whether the path is direct or relayed |
| `device` | the laptop | Runs a real `hq_host::Host`, dials the hub by id and `ip:port`, serves the host gate on that connection, reconnects with backoff, prints its own id |
| `stranger` | anywhere | Dials with a key nobody paired, to show what an unpaired peer gets |

Two filters sit in front of the protocol: QUIC address validation (a retry token, so a spoofed
source address costs the hub almost nothing) and a hook that refuses any peer id that is not in
the paired file as soon as the TLS handshake names it, before a protocol handler runs. The paired
file is read on every handshake, so pairing a device needs no restart. The QUIC handshake itself
completes for any key (the peer id is only known once it has), so "refused" means closed right
after it with code 403.

## Build

```
cd docs/plans/native-bridge/spike-c
cargo build --release          # about two minutes the first time; needs Rust 1.91 or newer
```

## Try it on one machine

```
BIN=target/release/bridgespike; D=$(mktemp -d); cd $D; touch paired.txt; chmod 600 paired.txt
$BIN hub --port 47831 --key hub.key --paired paired.txt --every 5 --calls 20 &    # prints "hub id: <ID>"
$BIN device --hub <ID> --addr 127.0.0.1:47831 --key dev.key &                      # prints "device id: <DEVICE>"
echo <DEVICE> >> paired.txt                                                        # pair it, no restart
$BIN stranger --hub <ID> --addr 127.0.0.1:47831                                    # refused with code 403
```

## Try it across a network

On the server, open the hub's UDP port to the one address you test from, and only for the
test. Replace the placeholders with your own values.

```
ufw allow from <your-public-ip> to any port <port> proto udp comment 'bridge spike'
./bridgespike hub --port <port> --key hub.key --paired paired.txt --every 10 --calls 50
```

On the laptop: `bridgespike device --hub <ID> --addr <server-public-ip>:<port> --key dev.key`,
then add the printed device id to the hub's `paired.txt`. Take it down with:

```
ufw delete allow from <your-public-ip> to any port <port> proto udp
rm -rf <the directory holding the keys>
```

A cloud provider's own firewall, if the server has one, needs the same rule added and removed.
Use `--relay none` (the default) to measure direct paths only. `--relay default` uses n0's
public relays, for comparison only: they are meant for development and testing.

## Loopback results (macOS, 2026-10-18)

| Check | Result |
|---|---|
| Unpaired device | Handshake completes in about 30 ms, then the hub closes it with 403 before any handler runs. The hub's "reached the protocol handler" counter stayed at 0 for it |
| Pairing | Appending the id to the paired file was enough; the device's next retry connected |
| Paired device, 20 calls of `host.status` | direct path, p50 54.3 ms, p95 55.4 ms. About 50 ms of that is the host's accept poll, not the transport |
| Stranger | Handshake completes, no answer, connection closed by the hub with code 403 and reason "not paired" |
| Hub restart on the same key | The device kept its dead connection for about 48 s before it noticed, then reconnected in about 30 ms with the same pairing. Dead-peer detection needs a shorter keep-alive; the cost is data on mobile (N5, N8) |

## Real-network results (home fibre in Uganda to a London VPS, 2026-10-18)

The hub's UDP port was open only to the test machine's public address, for under an hour, and
closed afterwards. Direct only (`--relay none`).

| Check | Result |
|---|---|
| Path | Direct in all 10 connect trials and in every status sample; never relayed |
| Connect time, 10 trials | 319 to 360 ms, median about 328 ms. That is two round trips: the retry token, then the handshake |
| Transport round trip (QUIC path RTT) | 158 to 166 ms. ICMP ping to the same server: 155 to 241 ms, mean 173 ms |
| `host.status` called down the connection, 250 calls | p50 174 to 176 ms, p95 220 to 277 ms, max 293 ms. About 15 ms above the transport round trip |
| Unpaired key (stranger) | Handshake completes (about 430 ms), then closed with 403 before any handler runs |
| Idle traffic, 2 minutes, hub sending nothing | About 90 kB in total, roughly 750 bytes per second, about 2.7 MB per hour. A single short sample; worth chasing before a mobile app (N8) |
| Active traffic, calls every 0.2 s | About 0.8 kB per call, both directions |
| Dead peer | A device whose process was killed, or a hub restarted on the same key, is noticed only after the QUIC idle timeout (about 30 to 48 s). Until then calls to it hang |

## Lessons for phase 1

- Keep one live connection per device. A device that reconnects while its dead connection has not
  timed out yet must replace it, and the old connection's cleanup must only remove its own entry.
  The first version of this hub got that wrong and lost its only live connection.
- Put a timeout on every call to a device and run devices concurrently, so one silent laptop
  cannot stall the others.
- Dead-peer detection needs a shorter keep-alive than the default. The price is data, which
  matters on mobile; the idle figure above says it is not free even now.
- The key check belongs in a post-handshake hook (it sees the peer id). The incoming filter only
  sees addresses, so it can ask for a retry token but cannot refuse by key.

Rows still to measure: a carrier hotspot, public wifi, UDP blocked with a relay, sleep and wake,
and a change of network mid-session (N1 to N5).
