# Edge threat model: the public forwarder

Companion to [docs/EDGE.md](../EDGE.md) and [docs/plans/native-bridge-sprint.md](../plans/native-bridge-sprint.md).
Status: proposal, 2026-10-22. It covers `hq edge`: the one public TCP listener that forwards TLS
connections to registered devices, the co-hosted iroh relay, and the device list. The device's own
TLS, path filter and OAuth are the connector's model ([CLAUDE_CONNECTOR.md](../CLAUDE_CONNECTOR.md))
and stay a second layer behind everything here.

## 1. What is protected

| Asset | Why it matters |
|---|---|
| A1. The content of connector traffic (tool calls, vault text, OAuth codes and tokens) | The edge sits on its path |
| A2. A device's TLS private key | Impersonating the device to claude.ai |
| A3. The edge's iroh key | Dialing devices as the edge; being admitted to the relay |
| A4. The device list | Who may use the edge, the relay and a device's budget |
| A5. The edge machine and its bandwidth | A public machine other people can aim traffic at |
| A6. Which devices exist and when they are used | Metadata about the owner |

## 2. Who is trying

| Actor | Can | Cannot |
|---|---|---|
| Internet scanner | Connect to 443 and send bytes | Get an answer: a connection that is not a ClientHello for a registered name is closed silent |
| Someone who knows or guesses a device name | Make the edge open a stream to that device, within its caps | Pass the device's TLS, OAuth or path filter; use another device |
| A slow or hostile client | Hold a connection open | Hold more than the caps allow, or past the deadline |
| Another iroh user | Try to use the household relay | Be admitted: the relay accepts the edge and registered devices only |
| A registered device (a stolen phone) | Use its own name, its own budget and the relay | Reach another device; stay connected after `hq edge revoke` |
| On-path observer (carrier, ISP, the edge's host network) | See who connects to the edge, when and how much | Read content: TLS ends on the device |
| The edge operator (the owner) | Read the edge's memory, add and remove devices, read metadata | Read content of a device that keeps its key on the device, as long as the operator is honest (see R2) |
| A compromised edge | Drop, delay or replay-open connections; learn metadata; get certificates for names it controls DNS for (R2) | Read content of connections already encrypted to a device key it does not hold |
| Certificate Transparency watchers | Read every device name that gets a certificate | See traffic |

## 3. Threats and mitigations

| # | Threat | Mitigation | Residual risk |
|---|---|---|---|
| T1 | A scan or probe learns that an edge is there | A connection that is not a TLS ClientHello with a name of a registered device gets no byte back: the socket is dropped. An unknown name, a name outside the domain, no name, plaintext and garbage are all treated the same | The listener exists; a scan sees an open port that closes without a handshake. Timing differs (R3) |
| T2 | A malformed or hostile ClientHello crashes or confuses the parser | rustls' `Acceptor` parses it (no keys derived, fragmented hellos handled); a hard cap of 18 KiB on bytes read; a hello that never completes is cut by the deadline; the parser is tested on every prefix and on mutated hellos | A parser bug in rustls. The dependency is pinned and audited in CI |
| T3 | Slow-loris: connections opened and fed one byte at a time | One total deadline from accept to a complete ClientHello (10 s), not a per-read timeout. Before the deadline a connection holds one of `max_pending` slots, and one of `max_pending_per_source` per address (per /64 for IPv6) | Many sources (a botnet) can still fill `max_pending` for the length of the deadline. Accepted: the pending pool is separate from forwarded connections, so devices already connected are unaffected |
| T4 | A flood of connections exhausts file descriptors or memory | Pending connections are capped globally and per source; forwarded connections per device; relay connections globally and per source; the systemd unit raises `LimitNOFILE`; each connection has one 16 KiB buffer per direction | A volumetric flood saturates the link. Accepted (A5) |
| T5 | One device, or traffic aimed at it, starves the others | Each device has its own connection cap and its own byte-rate bucket shared by its connections; an over-cap connection is closed, not queued | A device's cap is also a way to deny that device service: whoever knows its name can fill its slots (R4). The device remains reachable once they close |
| T6 | An unregistered name is forwarded | The label must be 16 hex characters under the configured domain and found in the registry. The name is lowercased; trailing dots, extra labels, other domains and IP literals are refused | Registry correctness (T7, T8) |
| T7 | The device list is tampered with or replaced | The list must be a plain file (no symlink), owned by the user running the edge, not writable by group or others. Otherwise the registry is empty and nobody is forwarded; the failure is logged | Anything running as the edge's user can edit the list. Run the edge as its own user |
| T8 | A device is registered under another's name | Two entries sharing a 64-bit label make the whole list invalid at start-up and refuse `hq edge allow`. Grinding a key to collide with a victim's label costs about 2^64 work and would still be refused | None known |
| T9 | Revocation does not take effect | The list is reread at most once a second and every forwarded connection rechecks it each second, so a removal cuts open connections in about 2 s, and a removed device is never looked up again, so no new stream is opened to it | A removed device keeps a connection that is mid-request for up to the next check |
| T10 | The edge reads or tampers with content | It never terminates TLS and holds no device key; the device's certificate is issued to the device (ACME TLS-ALPN-01 is answered through the edge, so the key never leaves the device). A tampering edge breaks the TLS handshake | The edge can refuse or delay service. It can also obtain a certificate if it controls the domain's DNS (R2) |
| T11 | Someone other than the edge opens streams on a device | The device accepts `hq/edge/1` only from the edge's endpoint id (iroh authenticates the peer's key in the handshake). The harness device in the tests does this; the device side is W2's contract | The device must implement it. A device that accepts any peer exposes its HTTP server to anyone with its id |
| T12 | The household relay becomes an open relay | The relay's access control admits the edge and registered devices only, using the same registry. It binds loopback and is reached through the edge listener by server name; it has no metrics listener | A registered device can use the relay for its own traffic. Relay connections are capped globally and per source |
| T13 | The edge dials a device to attack it, or is used to reflect traffic | It dials only registered ids, over iroh's authenticated QUIC; the stream carries bytes the client sent | A client can make the edge open streams to a registered device within its caps (T5) |
| T14 | The edge's iroh key is stolen | Mode 0600, owned by the service user, refused otherwise. A stolen key lets the thief dial devices (which then see a stranger once the key is replaced) and use the relay | Replace the key (`rm edge.key`, restart, give devices the new `hq edge id`). Until then the thief can open streams to devices. Defence in depth: the device's TLS, path filter and OAuth |
| T15 | Logs leak names, ids or content | No payload is ever logged. A forwarded connection logs the first six characters of the device label at debug level. The name asked for by a refused client is never logged. The iroh libraries, which log full endpoint ids at info, start at warn | An operator who raises `RUST_LOG` gets full ids |
| T16 | An admin or metrics surface is exposed | There is none: one listener, no HTTP API, no metrics port. Counters exist in memory for tests and one summary line at stop | None |
| T17 | A bug in the relay (a large third-party server) exposes the edge process | The relay runs in the edge process on loopback, behind the listener's caps; its TLS is its own. `iroh-relay` is pinned (`=1.4.1`) | A relay bug runs with the edge's privileges. The systemd unit drops everything except binding 443 |
| T18 | Let's Encrypt validation for a device name is blocked | `acme-tls/1` connections pass through like any other, in both directions, with the device's ALPN visible to it (tested) | Let's Encrypt's per-domain limits cap how many devices one edge domain can enrol per week |
| T19 | Idle connections pile up | A forwarded connection with no bytes either way for `idle_timeout_secs` (120) is closed. A dead client holds its slot until then if the device is silent | Up to 120 s of one slot |

## 4. Residual risks, stated plainly

- **R1. Certificate Transparency logs publish each device name.** A device that gets a certificate for
  `<label>.<domain>` appears in public logs, and the label is derived from its endpoint id. The name
  is not a secret and the edge treats it as public: it is a routing key, not authentication.
- **R2. "Never decrypts" holds for an honest operator, not an absolute one.** Whoever controls the
  edge domain's DNS can obtain a certificate for `<label>.<domain>` (from any CA, by pointing the name
  at a machine they run) and read that traffic. The owner runs both the DNS and the edge, so this is
  the owner. A hosted edge run by a third party would not have this property, which is one reason the
  project does not run one. A CAA record limits which authorities may issue.
- **R3. Timing reveals registration.** An unknown name is closed at once; a registered device that is
  offline is closed after the dial timeout. A scanner who learns a label can tell if the device is
  registered and online. Names are public (R1) so this adds the online bit only.
- **R4. Anyone who knows a device name can use up that device's connection and rate caps.** The caps
  protect the device and the edge; they do not authenticate callers. Authentication is the device's
  TLS, path filter and OAuth, which the edge cannot and does not replace.
- **R5. The edge sees metadata.** Which names are asked for, from which addresses, when, and how many
  bytes. It writes none of this to disk beyond what the operator's log level and the OS produce.
- **R6. The allowlist is a file.** It is a stand-in for pairing-based registration, which will let a
  device register by scanning the edge's QR code. Until then, `hq edge allow` is the only way in and
  it needs a shell on the edge machine.
- **R7. iroh and the relay are third-party code.** `iroh` and `iroh-relay` are pinned to one version
  and run in the same process as the forwarder. A vulnerability in either runs with the edge's
  privileges. The default relay (n0's public relays) is for development only; replacing it is a
  release gate in the sprint plan.
- **R8. Encrypted ClientHello.** A client that sends ECH hides the real name from the edge; the edge
  routes on the outer name and refuses what it does not know. claude.ai does not use ECH.
- **R9. One process, one listener.** There is no high availability: if the edge stops, devices are
  unreachable until it restarts. The systemd unit restarts it after five seconds.

## 5. Operator rules

- Run the edge as its own user, never as root; the unit grants only the capability to bind port 443.
- Keep `edge.key` and `devices.txt` mode 0600 in a directory only that user can enter. The edge
  refuses a key that others can read and a list that others can write.
- Point only `*.<edge domain>` at the machine, and keep the zone's DNS credentials off it.
- Leave `RUST_LOG` at its default unless debugging, and never keep debug logs.
- When a device is lost: `hq edge revoke <id>`, then revoke its sessions on the device side.
- Keep the relay off unless a household needs it (`edge.relay.enabled`), and start with Let's Encrypt
  staging.

## 6. Where each guard is tested

Tests are in `crates/hq-edge/tests/edge.rs` (end to end on loopback: a TLS client, the edge, a device
with no IP transport so it is reachable only through the relay) and in the unit tests beside each
module. Each guard below was removed in turn and a test failed; the list is in the pull request that
introduced this document.

| Guard | Test |
|---|---|
| Forwarding works for a device behind a NAT | `a_tls_client_reaches_the_device_behind_the_nat_through_the_edge` |
| TLS-ALPN-01 passes through | `a_tls_alpn_01_connection_passes_through_to_the_device` |
| Unknown, foreign and nameless connections get no answer | `an_unregistered_or_foreign_name_is_closed_without_any_answer` |
| Garbage and plaintext are closed at once | `garbage_is_closed_at_once_with_no_answer`, `hello::tests::*` |
| ClientHello byte cap | `a_hello_that_would_outgrow_the_cap_is_cut_off_by_the_cap_not_by_the_deadline`, `hello::tests::the_byte_cap_stops_a_client_that_keeps_sending` |
| Total ClientHello deadline | `a_slow_loris_client_is_closed_at_the_deadline_not_per_byte`, `a_client_that_sends_nothing_is_closed_at_the_deadline` |
| Pending caps, global and per source | `the_global_pre_hello_cap_closes_the_extra_connection_at_once`, `the_per_source_pre_hello_cap_stops_one_address` |
| Per-device connection cap and release | `a_device_cannot_exceed_its_connection_cap`, `finished_connections_free_their_slot_for_the_next` |
| Per-device byte rate | `the_byte_rate_limit_slows_a_device_down` |
| Idle timeout | `an_idle_forwarded_connection_is_closed_after_the_idle_timeout` |
| Dial timeout | `a_registered_device_that_is_offline_is_given_up_on_at_the_dial_timeout` |
| Revocation | `revoking_a_device_cuts_its_open_connections_and_refuses_new_ones` |
| Relay admits registered devices only | `the_relay_admits_registered_devices_and_the_edge_and_nobody_else` |
| Device accepts only the edge | `only_the_edge_may_open_streams_on_the_edge_protocol` |
| List mode, owner, symlink, collision, bad lines | `registry::tests::*` |
| Key file mode | `identity::tests::a_key_others_can_read_is_refused` |
| Everything off by default | `config::tests::everything_is_off_by_default` |
