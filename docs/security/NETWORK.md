# Network threat model: the private plane

Companion to [docs/plans/private-network.md](../plans/private-network.md). Status: proposal,
2026-10-18. It covers the WireGuard tunnel between an HQ server and the owner's devices, the TCP
host gate, the name and certificate for the web UI, and the connector's public site. The web login
itself is in [WEB_AUTH.md](WEB_AUTH.md) and stays as a second layer behind everything here.

## 1. What is protected

| Asset | Why it matters |
|---|---|
| A1. Command execution on the owner's laptop (the host gate, `agent.spawn`) | A paired server is as powerful as a shell inside the agent sandbox |
| A2. The web UI and its data (sessions, vault, tasks) | Private work, and the ability to start agents |
| A3. MCP and API credentials | Full tool access to HQ |
| A4. The hub's WireGuard private key | Impersonating the server to every device |
| A5. Device WireGuard private keys | Impersonating one device to the server |
| A6. The server's gate signing keys (one per host) | Calling a laptop's host gate |
| A7. The DNS credential used for certificates | Issuing certificates for the HQ name |

## 2. Who is trying

| Actor | Can | Cannot |
|---|---|---|
| Internet scanner | Send packets to every public port | Learn that HQ is there, from the tunnel port |
| Internet attacker who knows the server address | Flood the tunnel port; attack public services | Open a tunnel without an admitted key |
| On-path network attacker (café wifi, ISP) | Observe and drop UDP; see that a tunnel exists | Read or alter tunnel traffic |
| Holder of a stolen admitted device | Reach what that device reaches | Reach other devices; pass the web login unaided |
| A compromised agent on the laptop | Run as the owner inside the agent sandbox | Read the keys; reach the tunnel except HQ's MCP endpoint |
| A compromised server | Call every paired laptop's gate | Anything beyond the gate allow-list on the laptop |
| A malicious DNS provider or certificate authority | Mislead name resolution | Impersonate the hub: devices pin its key |
| The operator on a bad day | Misconfigure a firewall or a file | See section 5 |

## 3. Threats and mitigations

| # | Threat | Mitigation | Residual risk |
|---|---|---|---|
| T1 | A scan finds and probes HQ | HQ's web, MCP and gate listen on private addresses only. WireGuard does not answer a packet that is not from a key it knows, and a handshake needs the hub's public key. Public ports: tunnel UDP, ssh, and optionally the fenced connector port | The tunnel port itself exists. A scan can see "no answer", not "WireGuard" |
| T2 | Tunnel flood or handshake spam | Kernel WireGuard sends cookie replies under load (its built-in handshake DoS control); rate limits at the firewall | A large volumetric flood still saturates the link. Accepted |
| T3 | A stolen or lost device | Remove the peer (effective at once, existing session keys die), sign the device out of the web UI, rotate any token it held | Until noticed, the holder is limited to this device's reach and still needs the web login |
| T4 | A compromised device attacks other devices | The hub does not forward between peers (hq0 to hq0 is dropped explicitly and routed traffic is denied by default; `ip_forward` is not relied on, and is 1 on the owner's server while Tailscale runs); each device's `AllowedIPs` is the hub's single address; the laptop gate also checks the source address | A bug in forwarding configuration. Checked by SR-N5 |
| T5 | The gate is called by someone who is not the server | Source address allow-list, then an Ed25519 signature over a per-connection nonce and the request, then the existing 23-method allow-list. No secret travels in a join code | Compromise of the server's gate key (T9) |
| T6 | Replay or tampering of a gate request | A fresh nonce per connection bound into the signature together with the request digest; one request per connection | None known on this path; the tunnel adds its own protection |
| T7 | Resource exhaustion of the gate (slow client, many connections, huge request) | Auth deadline 5 s, read deadline 10 s, request size cap, 16 connections, a slot released even when a handler panics, nothing parsed before the signature verifies. No failure lockout, which would let the allowed source lock out the real caller | Whoever can connect from the allowed address can still hold 16 slots briefly, so the allow-list is only the server's tunnel address |
| T8 | The gate listens on the wrong interface | Refuses unspecified and non-private addresses; absent configuration means no listener; invalid configuration stops the host; SR-N3 | The owner sets the address of the home network by mistake: the source allow-list and signature still apply |
| T9 | The server's gate key or the hub key is stolen | Keys are mode 0600 under a dedicated service user. Recovery is written down: remove the line from `gate_authorized_keys` on each laptop, replace the hub key and re-add peers | Until noticed, the thief can call every paired laptop. This is the crown jewel |
| T10 | A malicious agent on the laptop steals the device key or reaches the tunnel | The agent sandbox hides `~/.hq` and allows only the egress proxy and HQ's MCP endpoint | A same-user process outside the sandbox can read a 0600 key. The OS keychain does not change this for an unsigned CLI |
| T11 | The web UI is reached by a rebinding or cross-site page | Unchanged: origin guard, Host check, web token, device tokens (WEB_AUTH.md) | See WEB_AUTH.md |
| T12 | Certificate misuse or DNS credential theft | The credential is scoped to challenge records (CNAME delegation or a zone-scoped token), readable by the certificate client only | A thief can obtain certificates for the name, but cannot get packets to a client without the tunnel |
| T13 | The name leaks | A public A record shows a private address; certificate logs show the name | Metadata only. A wildcard certificate keeps individual names out of the logs |
| T14 | Hub restarts or the tunnel interface is down at boot and Caddy falls back to a public address | Caddy binds only to the tunnel address; start order requires the interface first; if the address is absent it fails to start. Never fall back to all interfaces | Operator override. Checked by SR-N4 |
| T15 | The connector's public port is attacked | Firewall allow-list of the connector's published source ranges, a separate site that serves only the filtered routes, no tunnel services on it | The connector vendor's ranges are broad; the application layer (OAuth, path filter) still applies |
| T16 | Hostile input to the privileged applier | The applier is a separate root program with a strict grammar and no shell; it opens the input without following links, validates every line, and refuses everything else, including interface and hook lines | A parser bug is root. Hence tests with hostile files and an outside reviewer (SR-N9). The applier limits what a peers file can do, not who is admitted: anything running as the HQ user can add a device within the subnet. Every applied peer is written to the journal (address and key fingerprint) so a change is visible |
| T17 | Weak or reused enrolment of a device | Devices generate their own keys and send only public keys; the operator checks the label and the key's fingerprint; optional per-peer preshared key | Social engineering of the operator |
| T18 | Supply chain | WireGuard from the distribution and the official stores; `cargo deny` and `cargo audit` stay in CI; no bundled third-party VPN code | Normal dependency risk |
| T19 | UDP blocked | Documented limit. Owners on such networks need a TCP wrapper or the deferred relay path | Availability, not secrecy |
| T21 | A DNS credential on the server is used to rewrite the domain (installers, site, deploy wizard) | No credential that can write the whole zone is ever stored on the server: certificates are issued from the owner's workstation, or the challenge name is delegated by CNAME to a place that holds only validation records. A CAA record limits which authority may issue | A stolen narrow credential can mint certificates for the HQ name only |
| T22 | A local process uses the laptop's loopback proxy (`hq net proxy`) | Per-launch token in the request path or a unix socket; the web login remains the real guard; the proxy forwards only to the hub's tunnel address, never to another destination | Same-user processes can still read the token from the process list unless a unix socket is used |
| T20 | The old path stays open after cutover | Phase 3 checklist closes Funnel, `serve` and the tailnet; verified from outside | Operator error |
| T23 | On a shared machine, another local user listens on the tunnel address while the host is down and receives the server's calls | The gate authenticates the caller, not the server, and the tunnel supplies server identity; documented as single-user only, the ssh transport stays for shared machines | A local user on a shared laptop can still impersonate the gate until signed replies with a pinned server key exist (TECHDEBT.md) |

## 4. Security requirements (each one testable)

- SR-N1. With the tunnel down, the web UI, MCP and the gate answer nothing from any public address.
  Test: scan the server from outside the tunnel for all TCP ports and for UDP; only the listed
  public ports respond.
- SR-N2. No private key is ever sent over the network, printed to a log, or placed in a join code,
  a QR code or a config an operator is asked to share.
- SR-N3. The gate refuses to bind an unspecified or public address and defaults to off.
- SR-N4. The server's web proxy binds to the tunnel address only and fails closed when it is absent.
- SR-N5. No forwarding between peers: a device cannot reach another device's gate. Test from a
  second device.
- SR-N6. On a device, the hub is the only peer and `AllowedIPs` is the hub's single address; `hq
  net` never emits a full-tunnel configuration.
- SR-N7. Gate authentication happens before any parsing of the request; a captured exchange fails
  when replayed on a new connection; all limits in section 6.1 of the plan are enforced and tested.
- SR-N8. Removing a peer or a gate key takes effect on the next connection, and the documentation
  gives the revocation steps for a stolen device and for a stolen hub key.
- SR-N9. The privileged applier is covered by tests that feed it hostile files, runs without a
  shell, and is reviewed by someone other than its author before it ships.
- SR-N10. Defaults are safe: nothing listens publicly until the owner enables it, the example
  files use placeholders, and no real hostname, address or key appears in the repository.
- SR-N11. The connector's public port is reachable only from the published source ranges and serves
  no tunnel route.
- SR-N12. After the Tailscale cutover, an outside scan finds no Funnel or `serve` endpoint left.
- SR-N13. No credential that can write a whole DNS zone is stored on the server; the project's own
  zone has a CAA record naming the authority that issues its certificates.
- SR-N14. `hq net proxy` forwards only to the hub's tunnel address and is not usable by a process
  that lacks the per-launch token.

## 5. Failure modes of operation

Changes that touch the firewall or the tunnel print what they will do first and need the owner to
run them (or to approve the applier). A mistake that opens a port is caught by the outside scan in
SR-N1, which the documentation tells the owner to run after every network change.

## 6. Key lifecycle

| Key | Made by | Stored | Rotated | Revoked by |
|---|---|---|---|---|
| Hub WireGuard key | `hq net init` | Server, root-owned, 0600 | On suspicion; every device re-enrols | Replacing it |
| Device WireGuard key | The device's WireGuard app or `wg genkey` | On the device | On suspicion | `hq net peer revoke` |
| Optional preshared key | `hq net peer add --psk` | Both ends | With the peer | With the peer |
| Server gate key (per host) | `hq host add` | `~/.hq/gate/<host>.key`, 0600, service user | On suspicion | Deleting its line in the laptop's `gate_authorized_keys` |
| Certificate and its key | The ACME client | The proxy's store | Automatic | Let's Encrypt revocation if the key leaks |
| DNS credential | The owner | The ACME client only | On suspicion | The DNS provider |

## 7. Known limits

- A tunnel that needs UDP is not for every network (T19).
- The server is a single point of trust: whoever controls it controls every paired laptop's gate
  (within the allow-list and the agent sandbox).
- An attacker who is already a member of the tunnel and knows the web login is not stopped by this
  layer; the layers behind it are.
- The WireGuard protocol analysis cited in the plan is from the literature and was not redone here.

## 8. Review

Before Phase 1a ships: a second reviewer reads the gate code and this file; the fuzz target for the
line parser has run for a stated time; the outside scan in SR-N1 is recorded in the pull request.
Before Phase 1b ships: the applier is reviewed and tested as in SR-N9.

Phase 1b status, 2026-10-19: the applier (`crates/hq-net/src/grammar.rs`, `applier.rs`, `wgconf.rs`) has hostile-file and seeded randomized tests, an integration test of the renderer against the grammar, and the units are checked against the program's argument parser. It was read by a reviewer other than its author; the findings and what was done about each are in the pull request. Not yet done: a run on a live server (see [docs/PRIVATE_NETWORK.md](../PRIVATE_NETWORK.md), "What is not done yet"), and a second human reviewer for the shipped release.
