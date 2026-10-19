# A private network for HQ, without Tailscale

Status, 2026-10-19: Phase 1a is built and on `main` (the signed TCP gate, `transport: tcp`, `hq host
gate-key`, `hq host authorize --gate-key`), and a WireGuard hub with one peer has been run by hand and
scanned from outside. Phase 1b (`hq net` and the root applier `hq-net-apply`, crate `hq-net`) is built and
open as a pull request; two independent read-only reviews of the applier are described in
[docs/security/NETWORK.md](../security/NETWORK.md) section 8 (what they covered and what they did not run); the owner steps are in
[docs/PRIVATE_NETWORK.md](../PRIVATE_NETWORK.md). Phases 2 and 3 are proposals for review. The threat model and the testable
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
- The hub does not forward between peers: a firewall rule drops hq0 to hq0 explicitly, and the
  default for routed traffic is deny. On a hub that already forwards for something else (the
  owner's server has `ip_forward` at 1 while Tailscale runs), those two rules are what hold; `hq net
  init` reports the sysctl value and does not assume it is 0. A stolen phone cannot reach the
  laptop's gate.
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
(`0.0.0.0`, `::`), and on any address that is not loopback or private (RFC 1918, carrier-grade NAT,
link-local, IPv6 unique-local), so that it can never listen on the public internet by accident. A
setting that fails these checks stops `hq host serve` before it starts anything. An address the
machine does not have yet (the tunnel comes up after login) is retried in the background, so the
host and its local agents still start.

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
16 connections at once (more are accepted and closed at once). There is deliberately no lockout after
failed authentications: keys cannot be guessed, and a lockout would let anyone who can reach the gate
from the one allowed address shut the real caller out. Reads are bounded; nothing is parsed before
the signature verifies. The log records source, key label and method, never params.

The protocol authenticates the caller, not the server, and encrypts nothing: the tunnel does both. That
is sound on a machine only the owner logs in to. On a shared machine another local user could listen
on the tunnel address while the host is down; use the ssh transport there. Signed replies with a
pinned server key (a v2 of this protocol) would remove the assumption and are tracked in TECHDEBT.md.

The authorized keys file and the key files must be plain files (no links) owned by the user running
HQ, checked on the open handle; the authorized keys file must not be writable by group or others.
The chat shell cannot read `~/.hq/gate/*` or write `~/.hq/gate_authorized_keys`.

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
  `from=` mismatch, source not allowed, denied method, slow client, the connection cap, repeated
  failures not locking out the real caller, a panicking handler not leaking a slot), a seeded randomized test that feeds the parser garbage and mutated valid exchanges (a
  libFuzzer target is a follow-up, see TECHDEBT.md), and refusal tests for unspecified and public bind addresses.
- An integration test that runs a real listener in a thread and drives it with `Transport::Tcp`.
- Exit: on the owner's machine, Remote Login is off, Tailscale is off, the host is reached over the
  tunnel, a session starts from the Sessions page, and a 24 hour soak with sleep and wake loses no
  call. The ssh transport stays available per host as the rollback.

## 7. What is deliberately not built in Phase 1a

No bridge, no QUIC, no relay, no new pairing protocol, no mobile app, no new daemon. Spikes B to D
stay in the repository as evidence and as the basis of the deferred optional transport.

## 8. Phase 1b: `hq net`, so a stranger can do it

Status 2026-10-19: built (crate `hq-net`, `hq net`, `hq-net-apply`, units in `deploy/net/`). "As built" at the end of this section lists where the code differs from the text.

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

As built, where it adds to or differs from the text above:

- `wg syncconf` takes its whole file as the interface's truth: wireguard-tools marks the private key,
  listen port and firewall mark as set before it reads a line, so a peers-only file would clear them.
  The applier therefore runs `wg showconf` first, keeps `PrivateKey`, `ListenPort` and `FwMark`, and
  puts the validated peers after them. An empty peer list uses `wg setconf`, which removes every peer;
  `syncconf` returns early for an empty list and is not relied on for the last revocation.
- The input is a spool file `/var/lib/hq-net/<interface>.conf` in a directory the HQ user owns,
  written by `hq net apply` (and by `peer add` and `peer revoke`). The subnet, the hub address and the
  owner's user id come from a root-owned `/etc/hq-net/<interface>.env`, never from the input. The units
  are templates, `hq-net-apply@<interface>.service`, `.path` and `.timer`, so they ship unchanged.
  The service starts after every start of the tunnel (`WantedBy=wg-quick@<interface>.service`), the path
  unit on a change, and the timer at boot and every five minutes: a path unit does not fire when it
  starts, is blind while its service runs, and dies if the service hits its start limit, so the service
  sets `StartLimitIntervalSec=0` and the timer is the backstop. The applier reads its input again after
  it applies and repeats (at most three rounds) if it changed. The unit uses `Requisite=` on the tunnel,
  not `Wants=`, so a file change never starts a tunnel that was stopped on purpose. It lists each applied
  peer's address and key fingerprint in the journal.
- The input must start with `# hq-net peers v1` and end with `# hq-net end <n>` (the number of peers),
  and each block ends with `AllowedIPs`, so an empty or truncated file applies nothing. A file with no
  peers is the header and `# hq-net end 0`.
- Opening is one component at a time with links refused at every step, `O_NONBLOCK` so a planted FIFO
  cannot hang root, then checks on the open handle: regular file, expected owner, mode without group or
  other bits, one name, at most 64 KiB, in a directory only root or the owner can write.
- The applier finds `wg` only in the distribution's directories (`/usr/bin`, `/usr/sbin`, `/bin`, `/sbin`)
  or at an explicit `--wg`, and only if root owns it and others cannot write it. It runs with an empty
  environment and receives text over a pipe that it rendered from the parsed peers. It is installed in
  `/opt/hq-net/hq-net-apply`, not under `/usr/local`, which a group can write on some systems.
- The applier limits what a peers file can do; it does not decide who is admitted. Anything running as
  the HQ user can add a device within the subnet. The journal lists every applied peer.
- `hq net init` does not print the hub's private key. It stages the key in `~/.hq/net/hub.key` (0600)
  and the printed `wg-quick` config reads it from `/etc/wireguard/hq0.key` with a `PostUp`; the owner
  moves the key with `install` and deletes the staging copy.
- `peer add` refuses a key with the clamped shape of a private key unless `--confirm-public` is given;
  `net.toml` and `peers.toml` are read with unknown fields refused; the retired addresses are a file of
  their own (`retired`) so damage to `peers.toml` cannot lose them; every value that is printed into a
  root command or a path is validated on load and shell-quoted; `hq net apply` refuses to drop a device
  the hub has unless it was revoked here, and `peer add` and `apply` refuse while the staged hub key is
  still in the HQ user's directory.
- `hq net peer add --psk` makes a preshared key, as the key lifecycle table in
  [docs/security/NETWORK.md](../security/NETWORK.md) expects.
- The QR code is the device config without a private key, as specified. The Android app refuses to
  import a config without a `PrivateKey` line (checked in its source; iOS not checked), so on a phone
  it is for reading values from a second screen; see TECHDEBT.md for the choice this leaves the owner.
- The applier ships as its own binary, built with `--no-default-features` so it links `base64` and
  `libc` only. It is not yet part of the signed release (TECHDEBT.md).
- `systemctl reload wg-quick@<interface>` must not be used on the hub: the stock `ExecReload` runs
  `wg syncconf` with a stripped config that has neither the key (read from its file by `PostUp`) nor
  the peers, and `syncconf` would clear both. Restart instead; the service re-applies the peers.

## 9. Phase 2: the front door

### 9.1 Two tiers, so a domain is optional

A domain is needed only for the installed web app and push alerts on phones, because browsers
switch those on only in a secure context (HTTPS with a trusted certificate, or `localhost`), and a
certificate is issued for a name, never for a private address.

| Tier | Needs a domain | What works |
|---|---|---|
| 0 | No | The tunnel, the host gate, MCP, and the web UI over plain HTTP on the tunnel address (WireGuard still encrypts it). On a laptop, `hq net proxy` listens on `127.0.0.1` and forwards to the tunnel address, so the page is on `localhost` and gets the secure-context features without a certificate. Phones use the web UI in a browser tab without install or push |
| 1 | Yes | The installed web app and push on phones, at `https://hq.example.com` |

The domain is per-install configuration. It is never in the code or the default config; the
documentation uses `hq.example.com`. The project does not run a naming service and gives nobody a
subdomain of its own domain. The `hq net proxy` listener is on loopback, so any local process can
reach it (the finding from Spike B): it uses a per-launch token in the request path or a unix socket,
and the web login stays the real guard.

### 9.2 Tier 1 mechanics

- Caddy on the hub binds only to `10.77.0.1:443`, with a certificate for the HQ name from Let's
  Encrypt using DNS-01 (HTTP-01 cannot reach a private address). It fails closed if the address is
  absent.
- **No credential that can write the whole zone ever sits on the server.** The server is reachable
  from the internet (ssh) and holds the hub key; a stolen zone credential would let an attacker
  rewrite everything the domain serves. Three ways to keep the credential narrow, in order of
  preference for a single-owner install:
  1. **Issue from the owner's workstation.** `_acme-challenge.<name>` is a CNAME to nothing the server
     controls; a short script on the owner's own machine (a lego `exec` hook or a few lines calling
     the DNS provider's API) creates the TXT record, completes the challenge, and `hq net cert push`
     sends the finished certificate and key to the server over the tunnel (files 0600, certificate
     client only). The provider token stays in the workstation's keychain and never on the server.
     Renewal needs the workstation awake once in each 30 day window.
  2. **acme-dns on the server.** Delegate `_acme-challenge.<name>` by CNAME to an acme-dns
     instance. Its credentials can write only that one TXT record. Cost: a small DNS server on
     public port 53.
  3. **A scoped token at a provider that supports it**, with `_acme-challenge.<name>` delegated by
     CNAME to a zone that holds nothing else.
- Name resolution inside the tunnel: a public A record from the HQ name to the hub's tunnel address
  is the simple default (it reveals a private address and, through certificate logs, the name; a
  wildcard certificate keeps individual names out of the logs). Some home routers drop public
  answers that point at private addresses (DNS rebinding protection); then use a hosts entry on
  the laptop, or have the hub answer DNS for the tunnel only, with the WireGuard `DNS` line.
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

A shared free name (sslip.io, nip.io) does not work: the TXT record cannot be set, HTTP-01 cannot
validate a private address, and those suffixes share one rate-limit bucket.

### 9.3 The first install: the project's own domain

Read-only inspection of the project's registrar and public DNS on 2026-10-18:

- The zone is hosted at the registrar on its default name servers. Records: `@` A and AAAA to the
  static site host, `www` CNAME to `@`, `deploy` A to the deploy wizard's host, one TXT for site
  verification. No MX record and **no CAA record**. No API token exists in the account, and I did
  not find a way to restrict a token to one domain, so assume a token covers the whole account.
- The same zone serves the install scripts and the deploy wizard. Anyone who can write it can
  hand new users a malicious installer. This is why a zone credential must never live on the
  server (9.2).
- The site, `www` and `deploy` certificates all come from Let's Encrypt, so a CAA record that names
  only Let's Encrypt cannot break their renewal.
- DNSSEC: the registrar's form accepts DS records, but the default name servers do not appear to
  sign the zone, so it is not turned on; revisit if the zone moves to a provider that signs.

For this install, with the owner's agreement at each step:

1. Add `CAA 0 issue "letsencrypt.org"` to the zone. Registrar lock and two-factor sign-in stay on.
2. Choose a neutral label for the HQ name (`hq` is free). An A record from it to the hub's tunnel
   address, and a `_acme-challenge` CNAME only if route 2 or 3 of 9.2 is used.
3. Issue the first certificate from the workstation (route 1), then `hq net cert push`.

### 9.4 Android phones and tablets

Source: the vault note `Android-On-Device-Assessment` (2026-10-18), checked against the code and, for
the build claims, against a real build on the owner's Mac. This section keeps only what changes the
network plan. HQ running on the phone (a hub, or a native client) is a separate track, Spike E of the
Native Bridge work, and is not part of this plan.

- An Android device joins as a spoke through the official WireGuard app. The key pair is made in the
  app and never leaves the device; the hub entry is `AllowedIPs` of the hub's single address.
  `PersistentKeepalive` is left off on a phone: the phone starts every exchange, and Web Push reaches it
  through the browser vendor's service, not through the tunnel. Android's "Block connections without
  VPN" must stay off for this tunnel, because it would cut off everything outside the hub address.
- Cost of an idle tunnel, measured on the owner's Mac with a keepalive of 25 seconds: 3,744 bytes of
  WireGuard payload in 603 seconds, about 16 MB a month, roughly 23 MB with IP and UDP headers
  (estimate). At the small-bundle price of about UGX 3,250 a GB that is about UGX 75 a month, against
  the 2 GB a month the Native Bridge client idled at. A phone on mobile data will rekey more often when
  its network changes; measure that on the tablet.
- The web UI needs Chrome 111 or newer on Android (Tailwind v4 uses `@property` and `color-mix()`).
  The installed web app and push need HTTPS, so they are Tier 1. A public certificate (route 1 in 9.2)
  is also what makes the page work on phones without installing a certificate authority on each one:
  Android apps and WebViews do not trust user-installed authorities by default, and Chrome shows a
  standing warning for them. That is from Android's documentation, not yet seen on the tablet.
- Budget phones from Tecno, Infinix and itel ship a per-app power manager that can stop background apps,
  a VPN service included. The Phase 2 phone task tests the tunnel on the Android tablet first and on a
  budget phone if one can be had, and the phone guide lists the per-brand setting that keeps it alive.
- A code rule that follows: everything that needs root (the privileged applier, its systemd path unit,
  the part of `hq net` that applies peers) is gated with `cfg(target_os = "linux")`, never `cfg(unix)`,
  so a build for Android or any other unix target never contains it. `hq-gate` is `cfg(unix)` and
  checks clean for `aarch64-linux-android`.

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

1. WireGuard as the private plane, iroh bridge deferred: **approved 2026-10-18.**
2. The domain: the project's own domain at its registrar, with a neutral label for the HQ name, and
   certificates issued from the owner's workstation (9.2 route 1) so no zone credential sits on the
   server: **proposed, agreed in principle.** The CAA record (9.3 step 1) waits for a yes at the time.
3. A permanent public `udp/51820` on the server (silent to scans): approved in principle, not yet
   opened.
4. The privileged applier as specified in section 8, or manual `wg` commands in Phase 1a only: **built as specified (with the additions listed under "As built"), pending the owner's decision to ship it.**
5. The connector's fenced public `443`, or dropping the connector (claude.ai cannot reach a private
   tunnel): open.
6. Who reviews the gate protocol and the applier before they ship: open.
