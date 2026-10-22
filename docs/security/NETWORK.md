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
| T4 | A compromised device attacks other devices | The hub does not forward between peers, except between pairs the owner links (T26 to T28): hq0 to hq0 is dropped explicitly and routed traffic is denied by default; `ip_forward` is not relied on, and is 1 on the owner's server while Tailscale runs; each device's `AllowedIPs` is the hub's single address plus the addresses of devices linked to it; the laptop gate also checks the source address | A bug in forwarding configuration. Checked by SR-N5 and SR-N15 |
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
| T26 | Whoever controls the HQ user's files links two devices that should not talk, or re-keys a linked address | A link joins two addresses that belong to admitted peers in the same file; nothing else about forwarding can be said in the grammar (no ports, ranges, interfaces or rules); the applier replaces the contents of a set the owner created and never writes a rule; each added or removed pair is written to the journal, and so is a new key at an address that is in a link | A pair names an address, not a key. A process running as the HQ user can put its own key at a linked device's address and then reaches the other end of the pair on every port. What limits it is the other end's own `AllowedIPs` (the HQ user cannot edit a device's config) and the service's own login; so a service across a link must authenticate. Beyond this the HQ user can already admit a device, which reaches only the hub |
| T27 | A missing table, a failed or hung `nft`, or a stale set leaves more forwarding open than the file says | The owner's `PostUp` loads the table before it turns forwarding on, and `wg-quick` stops if it fails; the table is also included from the system's nftables configuration so a reload cannot drop it; an empty set forwards nothing; the applier runs `nft` first, in one transaction that changes nothing if any line fails, with a 10 second deadline, and a failed `nft` never stops `wg` but fails the run with a journal line that says forwarding may be wider than the file | If the table is removed while `hq0` forwarding is 1 (a `flush ruleset` reload not covered by the include), every device can reach every other until it is loaded again; every applier run then fails visibly in the journal but cannot repair it. After a failed `nft`, a pair already removed from the file stays open until the timer's next run (five minutes). Another firewall that allows `hq0` to `hq0` makes the table the only gate. Verified on a Linux 7.0 kernel with nftables 1.0.6 (see PRIVATE_NETWORK.md) |
| T28 | The hub reads or alters traffic between linked devices | None at the hub: it decrypts and encrypts again | Accepted. Every service across a link must be TLS or otherwise end to end encrypted; the host gate authenticates but does not encrypt, so it must not cross a link until it has an encrypted channel |

## 4. Security requirements (each one testable)

- SR-N1. With the tunnel down, the web UI, MCP and the gate answer nothing from any public address.
  Test: scan the server from outside the tunnel for all TCP ports and for UDP; only the listed
  public ports respond.
- SR-N2. No private key is ever sent over the network, printed to a log, or placed in a join code,
  a QR code or a config an operator is asked to share.
- SR-N3. The gate refuses to bind an unspecified or public address and defaults to off.
- SR-N4. The server's web proxy binds to the tunnel address only and fails closed when it is absent.
- SR-N5. No forwarding between peers except linked pairs: a device cannot reach another device's
  gate unless the owner linked the two. Test from a second device that is in no pair.
- SR-N6. On a device, the hub is the only peer and `AllowedIPs` is the hub's address, plus only
  the addresses of devices the owner linked to it; `hq net` never emits a full-tunnel
  configuration.
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
- SR-N15. The hub forwards a packet between devices only if its source and destination are a listed
  pair. Test: with one link, the two ends reach each other and a third device reaches neither;
  after `hq net link remove` neither direction works; after revoking one end, a new device given
  the next free address does not reach the other end.

## 5. Failure modes of operation

Changes that touch the firewall or the tunnel print what they will do first and need the owner to
run them (or to approve the applier). A mistake that opens a port is caught by the outside scan in
SR-N1, which the documentation tells the owner to run after every network change.

## 6. Key lifecycle

| Key | Made by | Stored | Rotated | Revoked by |
|---|---|---|---|---|
| Hub WireGuard key | `hq net init`, staged in `~/.hq/net/hub.key` (0600, the HQ user's) | `/etc/wireguard/<interface>.key`, root-owned, 0600, after the owner moves it with `install`; `hq net peer add` and `hq net apply` refuse while the staged copy exists (a revoke does not) | On suspicion; every device re-enrols | Replacing it |
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

Phase 1b status, 2026-10-19: the applier (`crates/hq-net/src/grammar.rs`, `applier.rs`, `wgconf.rs`) has hostile-file and seeded randomized tests, an integration test of the renderer against the grammar, and the units are checked against the program's argument parser.

Two reviews were done by readers other than the author, both read-only:

1. A review agent with no part in writing the code read the grammar, the file opener, `wgconf`, the units and the tests, with the wireguard-tools and systemd sources fetched for comparison. It found no parser or file-opener hole, and found problems in the units (a start limit that would stop the path unit, writes lost while the service runs, no apply at boot, `Wants=` starting a stopped tunnel). It compiled and ran only the macOS-visible tests: it did not run the Linux-only code, a live systemd, a real `wg` or a kernel.
2. A second independent review of the pull request found one blocker (a peer applied in a first round was not recorded if the second round failed) and seven items on the units, the stored files, the pasted-key case, the hub key's location, the revoke message and the trigger rate. It read the code and ran nothing as root.

Every finding of both reviews is closed in the pull request or listed in TECHDEBT.md with the reason. Not yet done: a run on a live server or a spare VM (the ignored real-interface test and the first apply of the units; see [docs/PRIVATE_NETWORK.md](../PRIVATE_NETWORK.md), "What is not done yet"), and a human reviewer other than the author for the shipped release.

## 9. The userspace peer inside the app (`hq-tunnel`)

Status, 2026-10-21: the library (`crates/hq-tunnel`) and the commands that run it
(`hq net tunnel keygen|import|check|run|status`, built with the `tunnel` feature) exist. The Android
shell that starts and supervises the peer comes later. A phone joins the plane as a WireGuard peer made of boringtun (the protocol) and smoltcp
(TCP) inside the app, with no `VpnService`, so it takes no system VPN slot and needs no second app. It
is a peer with exactly two maps and no routing:

- a **serve** entry hands one port on the device's tunnel address to one loopback port, for the
  tunnel addresses it names;
- a **reach** entry forwards one loopback port to one fixed tunnel destination.

It does not route other apps into the tunnel and has no mode that forwards to an address nobody
listed. The design and the measurements behind it are in the vault notes `WireGuard-In-App-Design`
and `WireGuard-In-App-Phase2-Results`.

What the join text is: the trust root. Its hub key, endpoint and addresses decide who the peer talks
to, and a hub can put any source address on a packet it forwards. Both source checks below compare
tunnel source addresses, so they hold exactly as far as the hub is trusted (T28). The serve and reach
maps are local settings, not part of the join text: they live in `maps.toml` beside the join text,
written by the owner, and importing a join text only ever removes that file, when the hub is a new one
(SR-N22). Nothing starts on its own: the peer runs only when `hq net tunnel run` is started, and the
commands exist only in a build with the `tunnel` feature.

The files, all under `~/.hq/tunnel` (or the directory the app passes), in a 0700 directory that every
command refuses to use if it is a link, belongs to another user or is open to others (a directory
that was open may hold files others put there, so it is not quietly tightened). The files the
commands write are 0600 and are read back only if they belong to the user, and `join.conf` and `key`
only if they are closed to others too: `join.conf` (the join
text, which may hold a preshared key), `maps.toml`, `key` (desktop only; an app passes the key on
stdin), `status.json` and `events.log`. The model's file tools are denied the whole directory
(`HOME_SECRET_PATHS`) and `~/.hq/net`, and its shell runs in a sandbox that masks the key files and
the join text and keeps both directories read-only (`bash_sandbox.rs`), so a `cd` and a relative
path, which the path check cannot follow, still cannot read a secret or plant a map. No setting
reaches the directory either: the maps are not in `config.yaml` and so cannot be set through
`config_manage` or an `HQ_` environment variable. The sandbox covers the directories as they exist:
a model that makes `~/.hq/tunnel` before the owner has can plant a `maps.toml`, and the first
`import` of a hub then removes it (SR-N22), because a first hub is a new hub.

### Threats

| # | Threat | Mitigation | Residual risk |
|---|---|---|---|
| T29 | A memory-safety or logic bug in boringtun or smoltcp, reached from the network | Both are Rust. The UDP socket is connected to the hub's address, so the kernel drops datagrams from anyone else, and the receive loop checks the source address again, so a socket whose connect failed (for instance during a network change) is not open to strangers: with no connected endpoint nothing is sent or believed until a lookup connects again. smoltcp sees only packets that already decrypted under a session key, and a packet is dropped before it unless it is plain, unfragmented TCP over IPv4 to this device from a source that may speak to its port. The peer is meant to run as its own process so a panic cannot take down the relays. Versions of both are pinned exactly | A flaw in the handshake parser is reachable by a sender who can spoof the hub's address and port; boringtun checks the MAC first, as WireGuard does |
| T30 | Another app on the phone connects to a reach listener | Loopback only (a listener on another address is refused at load); each forwarded service must authenticate itself; per-entry and total connection caps and an idle timeout | Any app on a phone can connect to loopback, and a local app or a web page can open enough connections to use up a reach entry's cap. A web page can also try `fetch` to the port, so the destination service must check `Origin` and `Host`. The service's own login is the guard |
| T31 | Confused deputy: a tunnel peer reaches a loopback port it was not meant to | The serve map is fixed: no wildcard, no range, a non-empty list of allowed sources per entry, targets loopback only. The source is checked three times: at ingress against the list of the port the packet is going to (so an unlisted source cannot even start a connection that holds a slot), again at accept, and by never connecting anywhere but loopback whatever the connector is asked | The target sees every tunnel peer as `127.0.0.1`: a target with a "localhost is trusted" shortcut must not sit behind a serve entry. A hub that spoofs a listed address passes the checks (T28) |
| T32 | A hostile or tampered join text | The parser takes only the lines `hq net` writes, one `[Interface]` then one `[Peer]`, one `/32` for the device and one for the hub, a plain a.b.c.d or DNS name for the endpoint, a keepalive of 0 or 10 to 300 seconds, and refuses everything else (including any `PrivateKey`, `DNS`, `PostUp`). Importing never writes the serve or reach maps. `hq net tunnel import` takes a hub it has not seen only when the caller names the hub's key (`--confirm-hub`), and a new hub takes the old maps away (SR-N22) | A person who confirms a key without comparing it with the hub's own output has confirmed nothing; the command prints both and says what to compare. A text that points the phone at another hub gives that hub the trust root above |
| T33 | A sender floods the UDP port to drain the battery or block the handshake | Datagrams from other addresses never arrive (connected socket). From the hub's address, at most 128 are taken per wake, a refusal costs a counter and not a log line, and the source address is handed to boringtun so its handshake rate limiter can answer with cookies | A sender who can spoof the hub's address and port can still cost CPU |
| T34 | A key or a setting ends up in a log, a status file, a command line or the environment | The private key is wrapped (`Secret32`: wiped on drop, redacted in `Debug`), handed over once, and the copy in the options is wiped at once; boringtun wipes its own. The status file and event log record states and counters only. The key arrives as the first line of stdin (read a byte at a time, so nothing buffered keeps a copy, and refused from a terminal) or from a file that must belong to the user, be closed to others and not be a link; no flag, argument or environment variable carries it, and `keygen --stdout` writes straight to the descriptor and refuses a terminal | A preshared key sits in the join text string while it is parsed, and the key generator's own copy of a new private key is not wiped; same-user processes on the phone can read the peer's memory. A killed peer leaves its last status behind, which `hq net tunnel status` reports as not running after 15 seconds |
| T35 | The peer spins and keeps the CPU awake | The loop sleeps until the earliest of smoltcp's timer, the next WireGuard deadline, the stack's own housekeeping and a ready descriptor; a descriptor with nothing to wait for is left out of the poll set; resolution runs on its own thread. An established idle tunnel wakes about once per keepalive, or every two minutes with none. A handshake nobody answers is retried every second for 10 seconds, every 5 seconds until a minute, then every 15 seconds. Time the device spends asleep does not time out a TCP connection, as with the kernel's own TCP | A tunnel whose hub is unreachable still wakes every 15 seconds, and one with no endpoint retries the lookup every 5 seconds. A network change retries at once |
| T36 | A peer holds connection slots without using them | A half-open or never-attached serve connection is torn down after 10 seconds; at most four connections are handed to their targets per pass; a quiet reach connection is closed after its idle time; every entry has a cap and there is a cap on all of them together | A listed source can still use up its entry's cap while it is busy |
| T37 | A path under the status directory is a link, or is replaced by another process | The files are created fresh with the mode set on the open file, never through a link (a link at the path is removed, and what it pointed at is neither read nor written); a temporary name is per file | The directory must be private to the app. A shared directory could still be raced |

### Security requirements

- SR-N16. A connection from a source the entry does not list never reaches the target, and an
  unlisted source never holds a slot. Tests: `a_packet_gets_in_only_from_a_source_that_may_speak_to_its_port`,
  `a_syn_from_an_unlisted_source_takes_no_slot`, `a_source_the_entry_does_not_name_never_reaches_the_target`,
  `a_source_listed_only_for_another_entry_is_dropped_at_the_door`.
- SR-N17. Serve targets and reach listeners are loopback only, the maps hold no wildcard, and the
  connector refuses a non-loopback address. Tests: `nothing_that_widens_the_maps_is_accepted`,
  `the_connector_refuses_anything_but_loopback_whatever_it_is_asked`. With the `tunnel` feature off the
  commands do not exist (CI compiles and lints the default build and the feature build separately),
  and with it on no socket is opened until `hq net tunnel run` is started
  (`tunnel_commands_parse_and_nothing_takes_a_key_on_the_command_line`).
- SR-N18. No key of the peer appears in a status file, an event log, a refusal message or `Debug`
  output. Tests: `the_status_and_event_files_never_hold_a_key` (private keys and a preshared key, in
  base64 and hex, after refusals and a network change), `a_secret_is_redacted_in_debug_output`,
  `a_refusal_names_the_line_and_never_the_value`. Absence from the command line, the environment and
  the output of the real binary: `tunnel_commands_parse_and_nothing_takes_a_key_on_the_command_line`,
  `the_peer_reads_its_key_from_stdin_speaks_to_the_hub_and_stops_on_quit` (stdout, stderr, the
  status file, the event log and every other file in the directory, base64 and hex, and that
  nothing at all is printed, since a log on a pipe nobody reads would block the peer),
  `the_key_is_the_first_line_and_what_follows_is_left_for_the_control_channel`,
  `what_is_not_a_key_is_refused_without_repeating_it`, `a_terminal_is_refused`,
  `a_key_file_must_be_private_and_not_a_link`, `a_generated_key_is_private_new_and_its_public_half_matches`,
  `keygen_prints_a_key_pair_to_a_pipe_and_writes_a_private_file_once`,
  `a_private_key_is_never_printed_to_a_terminal` (a pseudo-terminal as stdout).
  The decode of a key goes through a buffer on the stack and the output of `keygen --stdout` is built
  in one wiped buffer.
- SR-N19. The join parser refuses every line `hq net` does not write, and never panics on hostile
  input. Tests: `anything_hq_net_does_not_write_is_refused`, `garbage_never_panics` (join) and `garbage_settings_never_panic` (config),
  `the_text_the_hub_renderer_writes_is_accepted`, `the_keepalive_range_and_plain_addresses_are_accepted`.
- SR-N20. The status channel is a private file replaced atomically and never through a link, not a
  listener. The peer's own sockets are one UDP socket (replaced on a network change), the reach
  listeners the configuration names, and the lookup of the endpoint's name. Tests:
  `the_status_file_is_private_whole_and_versioned`, `a_link_at_the_path_is_replaced_not_followed`,
  `datagrams_from_anyone_but_the_endpoint_never_reach_the_session`,
  `a_link_at_the_event_log_is_replaced_and_what_it_pointed_at_is_never_read_or_written`,
  `a_log_far_past_its_limit_is_started_again_not_read_whole`.
- SR-N21. Connections are capped per entry and in total, a quiet reach connection is closed after its
  idle time, a never-completed serve connection is torn down, and a closed or hung-up stream does not
  keep the loop awake. Tests: `many_connections_stop_at_the_limit_and_recover_when_they_close`,
  `a_reach_entry_stops_at_its_own_cap`, `an_idle_reach_connection_is_closed_after_its_idle_time`,
  `a_connection_that_never_completes_is_torn_down`,
  `a_target_that_closes_at_once_does_not_make_the_loop_spin`,
  `a_closed_connection_with_bytes_its_target_has_not_taken_does_not_keep_the_loop_awake`,
  `a_flood_from_the_endpoint_costs_the_log_one_summary_not_a_line_each`.
- SR-N22. Importing a join text for a hub the device has not seen (a different key, host or port;
  an unreadable old join text counts as no hub) writes nothing unless the caller names the hub's key,
  and then removes `maps.toml`. The same hub again keeps the maps. A hostile text changes nothing.
  Tests: `a_first_hub_is_not_taken_until_its_key_is_named`,
  `the_same_hub_again_keeps_the_maps_and_needs_no_confirmation`,
  `another_hub_key_or_endpoint_needs_its_key_and_takes_the_maps_away`,
  `a_join_text_that_cannot_be_read_back_counts_as_no_hub`, `a_hostile_join_text_changes_nothing`,
  `a_wrong_key_is_refused_even_for_the_hub_already_known`,
  `a_map_that_cannot_be_removed_stops_the_import_before_the_hub_changes` (the maps go first, so a
  stop between the two steps leaves the old hub with no maps, which serves nothing),
  `the_join_text_is_private_and_a_link_at_its_path_is_replaced`,
  `the_join_text_is_found_inside_what_peer_add_prints`, and through the binary
  `a_new_hub_is_confirmed_by_its_key_and_takes_the_serve_map_with_it` and
  `a_hostile_join_text_is_refused_and_leaves_the_old_one_in_force`,
  `the_output_of_peer_add_can_be_imported_as_it_is`. The maps are also checked against
  the join text whenever they are loaded (`the_maps_are_read_with_the_same_strictness_as_the_config`).
- SR-N23. No file in the directory is reached through a link, and the directory is private. Tests:
  `a_file_is_replaced_whole_private_and_never_through_a_link`,
  `reading_refuses_links_other_modes_and_long_files`,
  `the_directory_is_made_private_and_a_link_to_one_is_refused`,
  `a_new_file_never_overwrites_one_that_exists`,
  `nothing_in_a_directory_that_is_open_to_others_is_trusted`,
  `a_tunnel_directory_that_is_open_to_others_is_refused_by_every_command`,
  `a_hand_written_map_file_needs_to_be_yours_but_not_closed_to_others`.
- SR-N24. The model's tools cannot read the directory or `~/.hq/net`, and its shell cannot read the
  secrets in them or write into them. Tests in `hq-agent`: `home_credential_stores_are_always_secret`,
  `the_private_planes_secrets_are_masked_and_its_directories_are_read_only` (both backends' rules) and,
  on macOS against the real `sandbox-exec`,
  `seatbelt_lets_a_command_read_a_read_only_directory_but_not_write_into_it`. The bubblewrap rule is
  built the same way and its argument order is tested, but it has not been run here. A shell without a
  sandbox (`governance.bash.sandbox` set to `off`) has none of this.
