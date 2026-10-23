# hq edge: reaching a phone from the internet

`hq edge` is a small public forwarder. It lets a device that cannot be reached from the internet (a
phone on mobile data) answer HTTPS on a name you own, which is what claude.ai needs to use HQ as a
connector. It does not hold your data, your tokens or any device key, and it cannot read the traffic.

You run it yourself, on a machine you control, for your own devices. It is off by default and there is
no project-run edge. The reasoning is in [plans/native-bridge-sprint.md](plans/native-bridge-sprint.md);
what it defends against, and what it does not, is in [security/EDGE.md](security/EDGE.md).

## How it works

```
claude.ai ── HTTPS, name <device>.edge.example.com ──▶ hq edge (public machine)
                                                          │ reads only the TLS server name
                                                          │ one iroh stream per connection
                                                          ▼
                                                      the device ends TLS and serves HTTP
```

- One TCP listener (443) reads the server name from each TLS ClientHello and goes no further. It does
  not decrypt, it holds no certificate for your devices, and a name that belongs to no registered
  device is closed with no answer.
- The name is `<label>.<edge domain>`. The label is the first 8 bytes of the device's iroh endpoint id
  as 16 lowercase hex characters (`hq_edge::device_label`); the full id is too long for a DNS label.
- The edge dials the device by key over iroh and opens one bidirectional stream per connection on the
  protocol `hq/edge/1`. The stream carries the client's raw TLS bytes from the first byte, with no
  framing. The device gets its own certificate (Let's Encrypt, TLS-ALPN-01 or DNS-01); a TLS-ALPN-01
  challenge for its name reaches it through the edge like any other connection.
- A device behind a NAT is reached through a relay it keeps a connection to. The edge can host that
  relay for your household (`edge.relay`), so no third party carries even the encrypted bytes.

## Set it up

You need a machine with a public IP address that you control, a domain whose DNS you can edit, and
the `hq` binary on that machine (`deploy/install.sh`, or the Docker image below).

1. **DNS.** Point a wildcard at the machine: `*.edge.example.com  A  <public IP>`. If you host the
   relay, `relay.edge.example.com` is covered by the same wildcard.
2. **Config.** Add to `config.yaml` (on that machine):

   ```yaml
   edge:
     enabled: true
     domain: edge.example.com
     # iroh_bind: "0.0.0.0:7843"   # a fixed UDP port is easier to open in a firewall
     relay:
       enabled: true                       # host the household relay (off by default)
       hostname: relay.edge.example.com
       acme_email: you@example.com
       acme_production: false              # staging: a dry run; set true to use the relay
   ```

   Let's Encrypt staging certificates are trusted by no client, so with `acme_production: false`
   the relay is a dry run of issuance: devices cannot use it until you set it to `true`.

   Everything else has a default; `hq edge --help` lists the commands, and the limits are described
   under "Limits" below. `listen` is IPv4 only by default; set `listen: "[::]:443"` to accept IPv6
   as well (and add `AAAA` records).
3. **Open the ports.** TCP 443 (the forwarder and, through it, the relay). If the relay is on, also
   UDP 7842 (the relay's address discovery, `edge.relay.quic_bind`) and the UDP port of `iroh_bind`.
   Nothing else listens: there is no admin port and no metrics port.
4. **Run it.** As a service: `deploy/edge/hq-edge.service` (a dedicated user, port 443 without root,
   the sandboxing the service needs). In Docker: `deploy/edge/Dockerfile`, with host networking (the header of the file has the `docker run` line; mount the config at `/etc/hq-edge/config.yaml`, not inside `/data`, and manage devices with `docker exec hq-edge hq edge allow <id>`).
   In a terminal: `hq edge run`.
5. **Register a device.** Two ways. The simple one is a QR code: on the edge machine run

   ```
   hq edge invite             # shows a QR code and a link for one device, valid for 10 minutes
   ```

   and scan it with the device (the device side, the app and `hq bridge register <link>`, arrives with the next change; until then a device that wants a QR registration needs the library call in `docs/PAIRING.md`). The device
   shows six digits; type them on the edge machine and the device is added to the list and told the
   name it is reached at. The invitation is the pairing protocol of `docs/PAIRING.md` under another
   protocol name, so the digits, the one-time code and the limits are the same, and the edge's own
   endpoint is not involved (the invitation runs on a throwaway one). It needs a build with the
   `bridge` feature; without it, use the second way.

   The invitation is direct: the edge's own relay admits only registered devices, so a device that
   is about to register cannot meet the invitation there. The QR code lists this machine's
   addresses, and the device must reach the invitation's UDP port while it is open. By default that
   is a random port; set `edge.invite_bind` to a fixed port (and open it in the firewall) for a
   firewalled machine, and list the public `ip:port` in `edge.invite_advertise` when the machine
   sits behind NAT (a cloud machine whose public address is not on its own interface).

   A device that names a home relay other than the edge's own is dialed through it only if you
   listed that relay in `edge.device_relays`; any other is ignored (the edge connects to whatever
   is recorded, and a device chose it).

   The second way is by hand: run the device's HQ, note its endpoint id, and on the edge machine:

   ```
   hq edge id                 # the edge's own id: give it to the device, which accepts only this peer
   hq edge allow <device id>  # add a device; add a relay URL if the device uses another relay
   hq edge list               # each device with the name it is reached at
   hq edge revoke <device id> # cut it off; open connections end within a few seconds
   ```

   Run these as the user that runs the edge (for the systemd unit:
   `sudo -u hq-edge env HOME=/var/lib/hq-edge HQ_CONFIG_PATH=/var/lib/hq-edge/.hq/config.yaml hq edge list`).
   The list is a text file (`~/.hq/edge/devices.txt`, mode 0600, owned by the user running the edge,
   in a directory only that user can write to, one `<endpoint id> [relay URL]` per line). Change it
   with `hq edge allow` and `hq edge revoke`, or replace it atomically (write a new file, then `mv`
   it into place): an editor that saves in place can leave a half-written file for a moment. While
   the list cannot be read completely, the edge refuses new connections and leaves open ones alone;
   once it reads again, a device that is no longer on it is cut. Run one `hq edge` list command at a
   time: allow, revoke and invite read the file and write it back without a lock. A list with a bad line, a wrong mode or owner, or two
   devices that share a name label is refused as a whole, and then nothing is forwarded. The forwarder
   reads the list through the `hq_edge::Registry` trait, whichever way a device got onto it.

## What the device must do

This is the contract W2 (the device's TLS and path filter) builds on.

- Accept connections only from the edge's endpoint id on ALPN `hq/edge/1`; close any other peer.
- Treat each incoming bidirectional stream as one TCP connection carrying TLS; end TLS on it with its
  own certificate for `<label>.<edge domain>`. Closing the stream closes the client.
- Use the edge's relay as its home relay (or have the relay URL on its `hq edge allow` line), so the
  edge can find it by key. The co-hosted relay admits the edge and registered devices only.
- Do not enable TLS early data (0-RTT). After the edge replaces a connection that stopped answering,
  the client's first flight can reach the device twice, once on each connection; a handshake without
  early data makes that harmless.
- A stream the device finishes or resets without a byte is a refusal of that client: the edge closes
  the client and keeps its connection to the device.

## Limits

All defaults are in `edge.limits`; a connection that breaks a limit is closed, never queued.

| Setting | Default | What it bounds |
|---|---|---|
| `hello_timeout_secs` | 10 | Total time from accept to a complete ClientHello (a client that drips bytes is cut at this deadline, not per read) |
| `max_pending` | 256 | Connections that have not finished their ClientHello, in total |
| `max_pending_per_source` | 16 | The same from one address (one /64 for IPv6); also the most relay connections from one source |
| `max_connections_per_device` | 16 | Concurrent forwarded connections per device |
| `bytes_per_second`, `burst_bytes` | 2 MiB, 1 MiB | A device's traffic, both directions together |
| `idle_timeout_secs` | 120 | No bytes either way |
| `dial_timeout_secs` | 10 | Setting up one forwarded connection: reaching the device, opening a stream, sending the ClientHello (also charged to the device's byte budget) and getting the first bytes of its answer, redialing included |
| `reply_timeout_secs` | 3 | How long a connection the edge already holds gets to answer before it is replaced by a fresh one (at most half of `dial_timeout_secs` is used). A path that died, such as a phone that changed network, leaves the connection looking alive for far longer |
| `max_relay_connections` | 128 | Concurrent connections to the co-hosted relay |

The ClientHello is also capped at 18 KiB. The limits are per process and per device, not per client:
whoever reaches a device through the edge shares that device's budget, which is the point of an
invite-only edge.

## Logs and privacy

The edge logs start-up, stop, and warnings. A forwarded connection logs at debug level with the first
six characters of the device label and nothing else. It never logs a payload, a full name, or the
name a refused client asked for. The transport libraries (`iroh`, `iroh_relay`) log whole endpoint
ids at info level, so `hq edge` starts with their filter at warn; set `RUST_LOG` to change it.

## Not in this version

Open registration, per-client limits and a hosted edge are out of scope on purpose.
