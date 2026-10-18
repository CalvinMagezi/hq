# A private network for HQ, without Tailscale

Status: proposal for review, 2026-10-18. Nothing here is built. The threat model and the testable
security requirements are in [docs/security/NETWORK.md](../security/NETWORK.md). Use placeholders
(`example.com`, `10.77.0.0/24`) everywhere; nothing in this file names a real host.

## 1. The problem

HQ runs on a server and serves a web UI and an MCP endpoint. Owners reach them from a laptop, from
phones, and from MCP clients, and the server drives coding agents on the laptop. Today all of that
rides on Tailscale: a second account, a VPN app on every device, node keys that expire, and a
hosted control plane. The owner wants the web UI and MCP to be **unreachable from the public
internet**, reachable only by devices the owner has admitted, with no third-party service.

Because HQ is open source, every default must stay safe when the code is public, and a stranger
must be able to set it up from the docs.

## 2. Requirements

- R1. The web UI, the MCP endpoint and the host control channel are not reachable by anyone who is
  not an admitted device. An internet scan of the server learns as little as possible.
- R2. No third-party account or control plane. Everything is open source with a licence that can
  be documented from an MIT project.
- R3. Phones work through a browser or a standard app. There is no HQ mobile app and none is planned.
- R4. The web app's PWA features (service worker, install, Web Push) keep working, so the web UI is
  served over HTTPS with a certificate browsers trust.
- R5. Laptop agents reach HQ's MCP endpoint; HQ reaches the laptop host to start and drive sessions.
- R6. Admitting and removing a device is explicit, auditable and immediate.
- R7. As little new security-critical code as possible. Prefer a mature protocol over a new one.
- R8. Idle cost small enough for small data bundles and for battery.
- R9. The claude.ai connector stays public by necessity; it must be fenced at the network layer.

## 3. What was evaluated

Research was done on 2026-10-18 from primary sources (project docs, repositories, licence files,
advisories, RFCs, CA documentation); the full notes are in
[private-network-research.md](private-network-research.md). Anything a source could not confirm is
marked unconfirmed.

| Option | Decisive finding | Verdict |
|---|---|---|
| **WireGuard**, hub and spoke | Does not answer unauthenticated packets, so a scan sees nothing. Official Android (Apache-2.0) and iOS (MIT) apps, no account. Idle cost near zero (keepalive off by default). No core CVE found for 2023 to 2026. No fallback when UDP is blocked. Enrolment is manual | **Chosen** |
| Reticulum | The licence added a field-of-use clause and an AI and machine-learning clause on 2025-04-15; it is not OSI approved and the clauses travel with every copy, which does not fit an MIT AI hub. No usable HTTP or WebSocket path (the maintainer discourages tunnelling TCP; MTU 500 bytes). No audit found. Initiators are anonymous, so a listener cannot refuse an unpaired peer at the handshake. No first-party iOS client | Reject. Borrow ideas only: a pre-auth packet MAC, identity allow-lists, announce and ingress rate limits |
| iroh bridge (Spikes B to D) | Works: direct path 10 of 10 from home fibre, an unpaired key is cut at the handshake. But the UDP port completes a QUIC handshake for any caller, so it is visible to a scan. Three GitHub advisories on 2026-10-05 (fixed in 1.1.0); the QUIC stack (noq) forked Quinn and inherits Quinn advisories that `cargo audit` cannot see. Idle traffic about 2.7 MB an hour. No phone client; iroh 1.3.0 does not compile for iOS without patches | Defer. Keep as an optional transport for networks that block UDP |
| NetBird, self-hosted | Needs a domain, public TCP 80 and 443 and UDP 3478; AGPL server, GPL-3.0 mobile apps (document, never bundle); a client privilege-escalation advisory in July 2026 and an install-script default-password CVE in 2025 | Runner-up |
| Headscale with Tailscale clients | One BSD-3 binary, but keeps Tailscale's apps, and the iOS app is closed source | Not the goal |
| Nebula | Needs a public lighthouse; two CVEs in 2025 and 2026; no audit found | Skip |
| Tor onion service with client authorisation | No inbound port, but Android Tor Browser failed on client-authorised services in October 2024 with no fix found; iPhone needs two apps | No |
| Caddy with client certificates | Scanners complete TLS and read the certificate-logged name; installed-PWA behaviour with client certificates on iOS and Android unconfirmed | Not for phones |
| OpenZiti, Yggdrasil, I2P, ssh tunnels | Heavy, routable by default, unconfirmed on phones, or answers scanners | No |
| Public HTTPS with strong login | Fails R1 | No |
| Cloudflare Tunnel with Access | Closed service, needs an account, TLS ends at the vendor | Fails R2 |

Sources: WireGuard protocol and limitations pages (wireguard.com/protocol, /known-limitations,
/quickstart); github.com/WireGuard/wireguard-android and wireguard-apple; the Reticulum LICENSE and
commit e7dacee, its manual and discussions 290 and 790; the iroh security advisories and RustSec
pages for quinn-proto; docs.netbird.io; headscale.net; Let's Encrypt challenge-types, rate-limit and
certificates-for-localhost pages; MDN Secure Contexts; WebKit's Web Push for web apps post. The
protocol analysis literature for WireGuard (Donenfeld and Milner 2018; Lipp, Blanchet and Bhargavan
2019) is cited from memory and was not re-checked in this research.

## 4. Decision

**Use WireGuard as the private plane for everything that must not be public.** The server is the
hub. Every device is a spoke with exactly one peer, the hub. The server's web UI and MCP endpoint
are served on the hub's tunnel address; laptops and phones reach them there. HQ owns the setup and
the lifecycle (`hq net`, later); the wire protocol is WireGuard, not something HQ invents.

Why this beats the iroh bridge for this goal:

1. It is the only option that is silent to a scan and works for phones with an official app and no
   account. The bridge cannot reach a phone browser at all (Spike D).
2. It removes most of the new security-critical code. No pairing protocol, no hub registry, no
   QUIC server on the public internet: the data path is a protocol with a long record.
3. One mechanism covers the web UI, MCP and the host channel, instead of a bridge for laptops plus
   something else for phones.
4. Idle cost is far lower: no keepalive at all for devices that only initiate (phones), a 25 second
   keepalive for the laptop that the server calls.

What it costs, said plainly:

- No fallback when a network blocks UDP. Owners on such networks need a TCP wrapper (documented,
  not built) or the deferred iroh relay path.
- Setup needs root once on the server (WireGuard is a kernel interface), and a small privileged
  step whenever a device is added or removed. Section 8 specifies it so that HQ itself stays
  unprivileged.
- A phone has one VPN slot. It replaces the Tailscale app, it does not add to it.
- Enrolment is by public key, not by a friendly pairing code. `hq net` has to make that bearable.

## 5. Architecture

```
 phone (WireGuard app) ──┐                                       public internet
 laptop (WireGuard app) ─┼─ UDP 51820 ─► [ server ]──────────────────────────────
 laptop host + agents ───┘              wg interface hq0, 10.77.0.1
                                        │  Caddy, bound to 10.77.0.1:443 only, certificate for
                                        │  hq.example.com (DNS-01), proxies to 127.0.0.1:<web>
                                        │  HQ web and MCP listen on 127.0.0.1 only
                                        └─ optional: tcp 443 on the public address, for the
                                           claude.ai connector only, allow-listed by source range
```

- Peers: hub `10.77.0.1`, devices `10.77.0.2` and up. On a device, `AllowedIPs` is the hub's `/32`
  only (never `0.0.0.0/0`): no other traffic is tunnelled. On the hub, each peer's `AllowedIPs` is
  its own `/32`.
- The hub does not forward between peers (`ip_forward` stays 0, and a firewall rule drops hq0 to
  hq0), so a stolen phone cannot reach the laptop's gate.
- The only public listeners: `udp/51820` (silent to unauthenticated packets), `tcp/22` (key-only,
  already there), and optionally the connector's `tcp/443` fenced by a source allow-list.
- Addressing: the subnet is chosen at `hq net init`, avoiding any subnet already routed on the
  machine and `100.64.0.0/10` (Tailscale's range), so both can coexist during the cutover.
- Names: `hq.example.com` resolves to `10.77.0.1`, either by a public A record (leaks only a
  private address and, through certificate logs, the name) or by split-horizon DNS. Use a wildcard
  certificate if the name itself should stay out of certificate logs.

## 6. Phase 1a: the Mac host over the tunnel (the first cutover)

Goal: HQ drives the laptop host with no Tailscale and no sshd on the laptop.

Prerequisite for the owner (documented by hand first, tooled in Phase 1b): the server has WireGuard
configured with the laptop as a peer, and the laptop runs the official app with the hub as its only
peer and `PersistentKeepalive = 25` (the server calls the laptop, so the laptop must hold the path
open).

### 6.1 The TCP gate

`hq host serve` gains a listener that speaks the same gate API as `hq host gate` today
(`hq_host::gate::parse_request` and `forward`, so the same 23-method allow-list), over TCP.

Configuration on the laptop (`~/.hq/config.yaml`):

```yaml
agent_host:
  gate_listen: 10.77.0.2:47801        # must be an address on the tunnel interface; absent = off
  gate_allowed_sources: [10.77.0.1]   # source addresses allowed to connect
```

The listener is off unless `gate_listen` is set. It refuses to start on an unspecified address
(`0.0.0.0`, `::`) so that it can never listen on the home network by accident.

Protocol, version 1. Everything is ASCII lines, no length-prefixed binary.

1. Server to client, immediately on accept: `HQGATE1 <nonce>\n`, where `<nonce>` is 32 random bytes
   from the operating system, base64url without padding. The nonce is per connection and is never
   reused.
2. Client to server, line 1: `<public-key> <signature>\n`. The public key is the client's Ed25519
   key, base64url. The signature is Ed25519 over the bytes `"hq-gate-v1\0" || nonce || SHA-256(line 2 without
   its newline)`.
3. Client to server, line 2: the request, exactly what `hq host gate` reads on stdin today (a JSON
   array of two strings, the method and its params as JSON text), at most `MAX_GATE_INPUT` bytes,
   terminated by `\n`.
4. Server: checks the source address against `gate_allowed_sources`, looks the public key up in
   `~/.hq/gate_authorized_keys`, verifies the signature, and only then hands line 2 to
   `parse_request`. A denied method or malformed request gets a JSON error line, like the ssh gate.
5. Server to client: one reply line, `{"result": ...}` or `{"error": {...}}`, then the connection
   closes. One request per connection, like the ssh gate's one process per call.

Limits: 5 seconds for the whole authentication exchange, 10 seconds to read the request, at most
16 connections at once (more are accepted and closed at once), at most 3 failed authentications per
source per minute before that source is ignored for 60 seconds. Reads are bounded; nothing is
parsed before the signature verifies. The log records source, key label and method, never params.

Why a signature and not a shared token: there is no secret to put in a join code or a chat message,
a captured exchange cannot be replayed (fresh nonce, bound to the request), and revoking a server
is deleting one line.

### 6.2 Authorising a server

`~/.hq/gate_authorized_keys`, mode 0600, one entry per line:
`<public-key> [from=<ip>[,<ip>...]] # <label>`. `hq host authorize --gate-key <public-key> --from
10.77.0.1` appends one. The file is read on every connection, so a removal takes effect at once.

### 6.3 The server side

`RemoteHostConfig` gets `transport: ssh | tcp`, default `ssh`, so existing configs keep working:

```yaml
agent_host:
  hosts:
    mac:
      transport: tcp
      addr: 10.77.0.2:47801
      # the server's key for this host: ~/.hq/gate/mac.key (Ed25519, mode 0600), made by `hq host add`
```

`Transport` gets a `Tcp` variant next to `Ssh`. It is plain `std::net::TcpStream` with the same hard
deadlines as the ssh path, and it returns the same `RawOutput`, so `NativeBackend::call_remote` is
unchanged. No async runtime, no shared state, no broker: every process makes its own short
connection, as it does with ssh today.

### 6.4 MCP for laptop agents

`agent_mcp_url` points at `https://hq.example.com/mcp`, which now resolves to the hub's tunnel
address from any admitted device. No forwarder is needed. (Until the front door exists in Phase 2,
the Mac keeps using the current URL.)

### 6.5 Tests and exit criteria

- Unit tests for the line parser (oversized, truncated, extra lines, bad base64, wrong signature,
  signature over a different request, replay of a captured exchange on a new connection, unknown key,
  `from=` mismatch, source not allowed, denied method, slow client, the connection cap, the failure
  rate limit), a fuzz target for the parser, and a refusal test for unspecified bind addresses.
- An integration test that runs a real listener in a thread and drives it with `Transport::Tcp`.
- Exit: on the owner's machine, Remote Login is off, Tailscale is off, the host is reached over the
  tunnel, a session starts from the Sessions page, and a 24 hour soak with sleep and wake loses no
  call. The ssh transport stays available per host as the rollback.

## 7. What is deliberately not built in Phase 1a

No bridge, no QUIC, no relay, no new pairing protocol, no mobile app, no new daemon. Spikes B to D
stay in the repository as evidence and as the basis of the deferred optional transport.

## 8. Phase 1b: `hq net`, so a stranger can do it

Commands (hub side unless noted); each prints exactly what it will change before it changes it.

- `hq net init [--subnet CIDR] [--port 51820] [--endpoint host:port]`: generates the hub key pair,
  picks the subnet, prints the WireGuard config and the firewall and sysctl commands for the owner
  to review and run as root. It never writes outside `~/.hq/net/` itself.
- `hq net peer add <label> --pubkey <key> [--kind laptop|phone]`: allocates the next address and
  appends to `~/.hq/net/peers.toml`; prints the device's config without a private key (the device
  keeps its own; the WireGuard app generates it on the phone) and a QR code of that same text.
- `hq net peer revoke <label>`; `hq net peer list`; `hq net status` (last handshake per peer).
- `hq net apply`: renders `peers.toml` into the privileged applier's input.

The privileged applier is a small root-owned program run by a systemd path unit when `peers.toml`
changes, so the HQ daemon never needs root. It accepts only: `PublicKey = <44 base64 characters>`,
`AllowedIPs = <address in the subnet>/32` (never the hub's own address or an address already used),
an optional `PresharedKey`, and comments. Everything else, including `PostUp`, `PreUp`, `Table` and
any other interface line, is rejected and nothing is applied. At most 64 peers. It then runs
`wg syncconf`. The path unit and the applier are part of the repository and are covered by tests that
feed them hostile files.

## 9. Phase 2: the front door

- Caddy on the hub binds only to `10.77.0.1:443`, with a certificate for `hq.example.com` from
  Let's Encrypt using DNS-01 (HTTP-01 cannot reach a private address). The DNS credential is
  narrow: delegate `_acme-challenge.hq` by CNAME to a zone only the certificate client may write, or
  use a provider token scoped to that zone. It is readable by the certificate client only.
- `web_allowed_origins` and `agent_mcp_url` move to the new name. HQ itself keeps listening on
  `127.0.0.1`.
- Firewall: allow `51820/udp` and `22/tcp` publicly; allow `443/tcp` and the web port only on `hq0`;
  default deny.
- The claude.ai connector: its public `443` site is separate from the tunnel site, serves only the
  existing path-filtered routes, and is fenced by a firewall allow-list of the connector's published
  source ranges (to be fetched from the vendor's page when this is built, and refreshed on a
  schedule), so a scanner cannot connect at all. The consent page moves from the tailnet to the
  tunnel site.
- Phones: install the official WireGuard app, add the peer, open `https://hq.example.com`.

Needs from the owner: a domain, and a DNS provider that supports the DNS-01 method or CNAME
delegation. A shared free name (sslip.io, nip.io) does not work: the TXT record cannot be set,
HTTP-01 cannot validate a private address, and those suffixes share one rate-limit bucket.

## 10. Phase 3: cutover and removal

Run both paths for a week: Tailscale untouched, the tunnel carrying the Mac host, then the web UI and
MCP. For each device: confirm it works over the tunnel, then remove it from the tailnet. Then turn
off `tailscale serve` and Funnel, close what they opened, stop Tailscale on the server and the Mac,
and uninstall after two stable weeks. Anyone else on the tailnet is told before it is removed.
Rollback at any step is putting the old route back; nothing is deleted until the end.

## 11. Deferred and rejected

- iroh as an optional transport for networks that block UDP, with a self-hosted relay and an
  allow-list: revisit after the tunnel is in use. Its Spike C and D results stand as evidence.
- A native phone app: not needed for this design.
- Reticulum: rejected, see section 3.

## 12. Decisions for the owner

1. Approve WireGuard as the private plane, with the iroh bridge deferred.
2. A domain, and the DNS provider for it.
3. A permanent public `udp/51820` on the server (silent to scans).
4. The privileged applier as specified in section 8, or manual `wg` commands in Phase 1a only.
5. The connector's fenced public `443`, or dropping the connector (claude.ai cannot reach a private tunnel).
6. Who reviews the gate protocol and the applier before they ship.
