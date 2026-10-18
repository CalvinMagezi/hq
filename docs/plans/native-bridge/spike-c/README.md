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

Real-network rows (home fibre, a carrier hotspot, UDP blocked) are recorded in section 16 of the
plan note.
