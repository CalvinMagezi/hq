# Pairing threat model: the Native Bridge

Companion to [docs/plans/native-bridge-sprint.md](../plans/native-bridge-sprint.md) (workstream W3)
and [docs/plans/zero-setup-pairing.md](../plans/zero-setup-pairing.md). Status: implemented in
`crates/hq-bridge` on 2026-10-22; the first review's findings are in section 8, the second
review's in section 9. The measurements the design rests on (a real phone, a Mac, n0's public
relay) are in [the spike's README](../plans/native-bridge/spike-f/README.md). How to use it is in
[docs/PAIRING.md](../PAIRING.md).

Pairing is how two machines of one person become peers: the first shows a QR code, the second
scans it, the person types six digits on the first, and from then on each reaches the other by key
alone. The same exchange, under another protocol name, registers a device with an `hq edge`.

## 1. What is protected

| Asset | Why it matters |
|---|---|
| A1. The paired list (`bridge_peers`) | Whoever is on it can open streams to this machine's bridge service, which the host gate and the connector will ride on |
| A2. The long-term endpoint key (`bridge.key`) | Impersonating this machine to every peer |
| A3. The long-term endpoint id | Not secret, but it should not be a target: it is never in a ticket and never sent before the person accepted |
| A4. The person's attention at the host | Six digits typed by hand are the only thing standing between a stranger who has seen the QR code and the paired list |
| A5. The ticket | A bearer for "this is the host I meant". It lets its holder try to pair, and spend or burn the ticket; it never lets them pair without the typed digits |

## 2. Parties, keys and what travels

- **Long-term endpoint** (`Bridge`): one key per machine, in a file only its owner can read
  (refused otherwise). It serves one protocol, `hq/bridge/1`, to peers on the paired list. It does
  not speak any pairing protocol at all.
- **Throwaway endpoint** (`OpenTicket`): a key made for one ticket, kept in memory, never written
  to disk. It speaks only the pairing protocol of the ticket's purpose. It is closed when the
  ticket is spent, expired or burned and no exchange is still running, so its port stops answering.
- **Ticket**: the throwaway endpoint's id, a random 64-bit code, optional relay URLs and addresses
  of the throwaway endpoint, the purpose and the protocol versions the host speaks. No secret key.
  It travels as `hqpair://ticket/<base64url json>` (the scheme is one constant, `LINK_SCHEME`) in
  the QR code and as copy-paste text; the older `hqpair1.<same body>` form is still read.
- **Joiner**: the scanning machine. It dials with its own long-term key, so the host learns its id
  from the authenticated connection, not from anything it says.

## 3. Listeners

Nothing in `hq-bridge` starts on its own, and the `hq` binary does not call it at start-up.
`hq bridge` exists only when built with the `bridge` feature (off by default, Unix only).

| Listener | Exists | Accepts | From |
|---|---|---|---|
| Long-term endpoint, one UDP socket | From `Bridge::start` with a handler until `shutdown`. Without a handler it only dials out and accepts no protocol | `hq/bridge/1` | Peers on the paired list; others are closed at the handshake (403) |
| Throwaway endpoint, one UDP socket per open ticket | From `Bridge::open_ticket` (or `OpenTicket::open`) until the ticket ends | `hq/pair/<v>` or `hq/register/<v>`, one purpose per ticket | Anyone who has the ticket, until it is used, expires, is burned, or the source is shut out |

Both bind the addresses the caller gives (the CLI defaults to every address, any port). The
throwaway endpoint always uses port 0 so it never collides with the long-term one. With a relay
configured, each endpoint also keeps an outbound connection to the relay.

## 4. The protocol

**Version negotiation.** The protocol name (ALPN) carries the purpose and the version:
`hq/pair/3`, `hq/register/3`. The ticket lists the versions the host speaks (`proto`). The joiner
takes the newest version both speak, offers it first and the older common ones after it, and the
host's TLS stack picks. With none in common the joiner stops before dialing and says which side is
old (`negotiate`). The hello repeats the version, and a host that sees a different number than the
name it negotiated refuses with a clear message, which costs the guesser a strike but not the
ticket an attempt. A ticket from the spike (layout 1) is refused as "from an older version"; a
newer layout as "from a newer version".

**Exchange**, one JSON object per line on one stream, joiner first (`wire.rs`):

1. `Hello`: version, the ticket's code, and a commitment `H(commit-context, alpn, nonce_j)`.
2. The host checks the code in constant time and spends it. It answers `Reply` with its nonce
   `nonce_h` and a binding `H(bind-context, host_id, nonce_h, k)` of its long-term id, where `k` is
   a fresh secret. Neither the id nor `k` is sent yet, so the binding cannot be used to test a
   guessed id.
3. `Reveal`: the joiner opens its commitment (the host checks it) and adds a name for the person
   to read. Both sides compute six digits from the protocol name, the throwaway id, the host's
   binding, the joiner's id, the code and both nonces.
4. The joiner shows the digits. The person at the host types them. The host asks the registrar to
   record the joiner only if they match, the answer came inside the decision window and the joiner
   is still connected.
5. `Final`: the outcome and, on acceptance, the host's card (long-term id, name, how to dial it)
   and `k`. The joiner checks the card's id and `k` against the binding from step 2, makes the
   card safe to store (one spelling of the id, a printable name, parsed and capped lists) and
   stores it.
6. `Details`: only now does the joiner tell the host how to dial it back (its relays and
   addresses). A host that was never going to accept, such as a rogue QR code, learns the joiner's
   id and chosen name but none of its addresses.

Why these pieces: the digits cannot be ground for by a machine in the middle because the joiner
commits before it sees the host's nonce. Binding the host's long-term id before the person
compares digits means the digits cover the identity that gets stored, while a stranger who only
burns the ticket learns nothing about it. A purpose or version change alters the commitment and the
digits, so a registration can never finish as a pairing or the other way round, even if one
endpoint accepted both.

## 5. In front of the protocol

| Layer | What it does | Where |
|---|---|---|
| Protocol name | An endpoint registers only its own purpose's names; the long-term one only `hq/bridge/1`. A dial with any other name fails the handshake | `Router` in `host.rs` and `bridge.rs` |
| Gate: ticket window | While the ticket is spent, expired or burned, every pairing connection is closed at the handshake (410), before any handler runs | `PairGate` |
| Gate: source | A source that used up its strikes is closed at the handshake (403) | `PairGate`, `limits.rs` |
| Gate: paired list | The long-term endpoint closes any key not on the list (403); a read error counts as "not paired" | `ServiceGate` |
| Concurrency | At most 2 exchanges per source and 8 in all; a third from one source is closed at once (429), not after the timeout | `Limiter::admit` |
| Deadlines | 10 s to open a stream and for each read and each write; 120 s for the person to answer; 4096 bytes per line. The whole exchange is also capped (`Tuning::exchange_budget`, 194 s by default), so no stalled wait can hold a slot | `host.rs`, `wire.rs` |
| Strikes | A wrong code, a bad reveal, a bad version or a stalled hello is one strike. 5 for an address, 20 for everything that comes through one relay; then the source is closed for the rest of the ticket. The table remembers 4096 sources and forgets the quietest idle one to make room, so a flood of sources cannot lock the door; it only resets their own counts, and the backstop below still bounds the guessing | `Limiter::strike` |
| Global backstop | 1000 wrong codes from all sources burn the ticket for everyone | `Window` |

A ticket lives at most 24 hours whatever the caller asks for (`MAX_TTL`).

A **source** is the network address of the path the connection arrived on (an IPv6 address counts
as its /64). Over a relay the address is the relay's, so the source is the relay URL.

## 6. After pairing

The service handler registers each connection in a live registry before it reads the list, and
reads the list again on every new stream. Connections this machine opened to a peer (`connect`) are
registered too. `Bridge::revoke` writes the revoke and then closes every registered connection of
that peer, in either direction, with no polling. The order closes the race between the gate and
the registry: a revoke that lands in between is either seen by the read or finds the connection in
the registry. A revoke made in another process (`hq bridge revoke`) only changes the database; the
running bridge stops serving that peer at its next request or handshake, and closes its connections
at once the next time its own code calls `Bridge::revoke` for the same id.

## 7. Attack notes

| # | Attacker | Tries | Result | Guard (test) |
|---|---|---|---|---|
| 1 | Stranger who learned the long-term id | Dials the service, or any pairing name | Closed at the handshake, no handler ran; pairing names do not exist on that endpoint | `ServiceGate`, `Router` (`a_key_nobody_paired_is_closed_before_any_handler`, `the_long_term_endpoint_does_not_speak_the_pairing_protocol`) |
| 2 | Holder of a photographed ticket | Pairs before the real device | Spends the ticket. The person types the digits of the real device; the attacker's differ; declined | SAS, typed digits (`a_declined_device_is_not_paired_and_cannot_reconnect`) |
| 3 | The same | Learns the host's long-term id | Nothing sent before acceptance; a wrong code gets only a reason | binding (`a_wrong_code_learns_nothing_about_the_long_term_identity`) |
| 4 | Holder of the ticket's id only | Guesses the 64-bit code | 5 strikes per address, 1000 in all; odds near 5e-17 | `Limiter`, `Window` (`one_source_that_guesses_is_shut_out_while_another_source_still_pairs`, `the_global_backstop_burns_the_ticket_for_the_real_device_too`) |
| 5 | The same, from one address | Burn the ticket | One address is shut out; the ticket stays open for others | same |
| 6 | The same, from many addresses | Burn the ticket | Burns at 1000. The person shows a new QR code. Accepted | `Window` |
| 7 | The same | Hold every slot with silent connections | 2 per source, 8 in all; the extra one from a source is closed at once; each silent one is closed after 10 s and costs a strike | (`one_source_cannot_fill_the_concurrency_cap`, `a_pairing_connection_that_never_speaks_is_closed`) |
| 8 | Two devices | Race for one ticket | The code is spent by the first; exactly one pairs | (`two_devices_racing_for_one_ticket_pair_exactly_one`) |
| 9 | Late joiner or photograph after use | Reuses the ticket | The throwaway endpoint is gone; nothing answers | `lifecycle` (`a_ticket_works_once_and_its_endpoint_is_gone_afterwards`) |
| 10 | Machine in the middle between the two | Chooses nonces so the digits match | Cannot: the joiner commits first. A reveal that does not open the commitment closes the connection and never reaches the person | `commit`, `opens` (`a_reveal_that_does_not_match_the_commitment_never_reaches_the_person`) |
| 11 | Host that swaps its identity after the digits | Hands the joiner a different long-term id | The joiner checks it against the binding it got before the digits and stores nothing | `bind_host` (`a_host_that_swaps_its_identity_after_the_digits_is_refused`) |
| 12 | Slow person, or a device that leaves | A yes that arrives after the window, or after the device is gone | Nothing recorded; the joiner is told "timeout" or "gone" | `record` (`a_late_answer_pairs_nobody`) |
| 13 | Revoked device with a connection open | Keeps using it | Closed at once by `Bridge::revoke`; a revoke from another process stops the next request and the next handshake | `Live`, `still_paired` (`taking_a_device_off_the_list_closes_the_connection_it_already_has_at_once`, `a_revoke_made_elsewhere_stops_the_next_request_and_the_next_handshake`) |
| 14 | Anyone, with the database broken | Passes while the list cannot be read | Refused: a read error is "not paired" | (`a_database_that_fails_closes_the_gate`) |
| 15 | The other machine | Text with a terminal escape sequence as its name or refusal reason | Replaced and capped before it is shown | `printable` (`a_name_with_a_terminal_escape_reaches_the_person_clean`) |
| 16 | A ticket for one purpose | Completes the other | No door: the protocol name differs, and so do the commitment and the digits | `Purpose::alpn`, `commit`, `sas` (`a_device_registers_with_an_edge_and_the_two_purposes_never_cross`) |
| 17 | Old or newer build | Mismatched versions | A clear refusal naming the old side, before a dial where possible | `negotiate` (`an_unsupported_version_is_refused_without_costing_an_attempt`) |
| 18 | Anyone, over the network | Oversize input | Ticket text over 2000 characters, lines over 4096 bytes and address lists over 8 are refused | `Ticket::parse`, `read_line` |
| 19 | Local user | Reads the key file | A key readable by group or others is refused at start | `load_or_create_key` |
| 20 | A client that connects and never reads (a zero flow-control window) | Holds an exchange slot forever, so the throwaway endpoint never closes and other joiners are turned away | Every write has the step deadline and the whole exchange has a budget; the connection is closed and the endpoint goes down with the ticket | `say`, `exchange_budget` (`a_client_that_never_reads_cannot_hold_a_slot_or_keep_the_endpoint_up`) |
| 21 | Ticket holder with thousands of addresses | Fills the source table so that new sources, including the real device, are refused | The table forgets the quietest idle source; a new source is never refused for lack of room | `Limiter::make_room` (`a_full_table_forgets_the_quietest_idle_source_and_never_refuses_a_new_one`) |
| 22 | A rogue QR code left in a public place | Harvests the addresses of everyone who scans it | The joiner sends its id and name only; its addresses go after the host proved its identity | `Details` (`a_host_that_swaps_its_identity_or_its_key_after_the_digits_is_refused`, `a_host_that_keeps_its_promise_is_stored_and_only_then_told_how_to_dial_back`) |
| 23 | Ticket holder who knows a candidate host id | Tests it offline against the binding | The binding includes a secret released only on acceptance | `bind_host` (`the_host_binding_hides_the_id_and_depends_on_the_nonce_and_the_key`) |
| 24 | The other machine | A name made of bidirectional overrides or zero-width characters to disguise the prompt | Replaced like control characters | `printable` |
| 25 | A host that spells its id another way | Stores an id the gate never matches, or breaks `revoke` | The card is stored in one spelling; `connect` and `revoke` parse their argument first | `normalize_card` (`a_peer_can_be_dialed_by_any_spelling_of_its_id`) |
| 26 | A paired peer that was revoked | Keeps a connection this machine opened to it | Closed too | `Bridge::connect` (`revoking_a_peer_also_closes_the_connections_we_opened_to_it`) |

## 8. First review (the spike) and what became of it

An independent reviewer attacked the spike on 2026-10-22. Findings and their fate:

| Finding | Fate |
|---|---|
| The digits were a nonce-free hash the host also sent, so a habitual "y" paired a device | Fixed in the spike (commit then reveal, typed digits); carried over |
| A late "y" paired a device that had been told "timeout" | Fixed in the spike; carried over (#12) |
| Removing a device left its open connection working | Fixed in the spike by polling; here pushed (#13) |
| Pairing connections were unbounded | Spike: 4 at a time. Here: per-source and global caps (#7) |
| The paired list was a text file anyone could edit | Spike: owner and mode checks. Here: the database, with the check on every handshake and request, failing closed (#14) |
| Text from the other machine could carry terminal escapes | Carried over (#15) |
| Four idle pairing connections held every slot, and fifty wrong codes burned the ticket, for a stranger who knows only the host's id | Fixed: a throwaway key per ticket (the long-term id is not a door at all), per-source limits, a large backstop (#4 to #7) |
| Check then open | Fixed: the registry is written first and the list read after |
| Revocation was polled | Fixed: pushed through the live registry |
| A device that vanishes just as it is accepted can end up on the list | Narrowed, not closed: see section 10 |

## 9. Second review

A separate reviewer (a fresh agent given the code and the design goals, not this file, and no
shell to build with) read the crate, the migration, the CLI and the iroh sources on 2026-10-22. It
reported one must-fix and ten should-fix items. Each was checked against the code.

| # | Finding | Verdict | Fate |
|---|---|---|---|
| 1 | Must-fix. Writes had no deadline: a client with a zero receive window never reads, the reply never leaves, the slot is never released and the throwaway endpoint never closes. Eight such connections turn every joiner away. Also, in the CLI, the first ctrl-c handler is permanent, so the wait after an answer could not be interrupted | Confirmed by reading `Slot` and `lifecycle` | Every write is under the step deadline and the exchange under `exchange_budget`; the CLI waits with ctrl-c in a `select`. Test with a zero-window client |
| 2 | The joiner sent its dial-back addresses before the person typed anything, so a rogue QR harvested them | Confirmed | Moved to `Details`, after the card is checked (#22) |
| 3 | 1024 sources with one strike each locked out every new source without touching the global budget | Confirmed | Eviction of the quietest idle source; 4096 entries (#21) |
| 4 | Revoke did not reach connections this machine opened | Confirmed | Registered in `Live` (#26) |
| 5 | The host binding let a ticket holder test guessed ids offline | Confirmed (no secret in it) | A secret released only in `Final` (#23) |
| 6 | The card was stored unnormalized; iroh reads hex and base32 and prints hex, so a base32 id passed the check and never matched the gate. `revoke` and `connect` had the same gap | Confirmed in the iroh source | `normalize_card`; `connect` and `revoke` parse first (#25) |
| 7 | `printable` stripped only control characters | Confirmed | Also bidirectional and zero-width characters (#24) |
| 8 | The link (with the code) on the command line; an unbounded `--ttl` overflowed | Confirmed | `join` reads standard input by default; `--ttl` is 10 to 3600 s and tickets are capped at 24 h |
| 9 | The long-term service gate does a full handshake and a database read per attempt, with no per-source budget | Accepted, not changed | Residual risk below; tracked as NATIVE-BRIDGE-010 |
| 10 | Weak tests: the long-term-endpoint test, "endpoint gone", the purpose check shadowed by the protocol name; no test for #1, #3 or a dropped ticket | Confirmed for the first two and the missing ones | Tests tightened or added. The purpose check in the gate stays as a second layer and is covered together with the protocol name |
| 11 | Unconfirmed: two path snapshots in the gate and the handler; a garbage `Reveal` gave no strike | The snapshots only affect the `refused` counter; strikes use the one captured source | The garbage reveal now strikes |

The reviewer also listed what it found sound: what the digits bind, the commit-before-nonce order,
the single-use code, the revoke ordering, `Notify` use, release of slots on cancellation, the line
bound, the key file creation. A second look at the changes made for this section follows in the
pull request.

## 10. Residual risks

- **The QR code is the root of trust for the host.** The digits prove that the person at the host
  saw the device that scanned. They do not prove the screen showed the host the person meant. A
  QR code swapped on a screen, or one received over chat or a screen share from a stranger, pairs
  the scanner with the stranger's machine, which accepts its own digits. The app that scans should
  show the host's name and short id after pairing, and the owner can compare it with
  `hq bridge peers` on the real host.
- **A leaked ticket can be spent or burned** by its holder (#2, #6). It cannot pair a device.
- **The long-term service gate has no per-source budget.** A stranger who knows the id costs a
  handshake and a database read per attempt before being refused. The id is not in any ticket, so
  this needs the id from somewhere else. A Router incoming filter or a short negative cache would
  cap it; tracked as NATIVE-BRIDGE-010.
- **The name the joiner chooses reaches the person before the digits.** It is made printable and
  capped, and is never a reason to trust the device.
- **Relays share a bucket.** Everything that arrives through one relay shares 20 strikes and one
  concurrency budget, so a ticket holder using the same relay as the real joiner can lock the
  joiner out of that relay's bucket. The cost to them is the same as burning the ticket.
- **A relay sees metadata.** Who connects to which endpoint, when, and how much. It cannot read
  the traffic. n0's public relays are for development and testing; replacing them is a release
  gate in the sprint plan.
- **Addresses in a ticket.** By default the ticket lists the throwaway endpoint's addresses, so it
  shows the host's local and public IP addresses to whoever sees the QR code. `--no-addrs` leaves
  them out; the spike measured that a relay-only ticket still ends on a direct path.
- **A scanned ticket is untrusted input.** It can name up to 8 addresses and relays the joiner
  will try to reach with a QUIC handshake. Nothing else is sent to them before the host is
  authenticated.
- **A revoke from another process is not pushed.** It takes effect at the next request or
  handshake; a connection that sends nothing stays open, and idle, until its owner revokes through
  the bridge's own API or it closes.
- **A device that vanishes just as it is accepted** can still end up on the list: the host
  records it after checking the connection is open and cannot undo a write that races with the
  disconnect. It is a peer that never learned the host, which the owner can revoke.
- **A same-user process can read the key file.** The mode check stops other users, not another
  process of the same account.
- **A custom link scheme can be claimed by another app on the phone.** That app would see the
  ticket. It gains what a photograph gives (#2), nothing more. An https link verified by the
  platform would close this; the choice is the owner's and the scheme is one constant.
- **Source classification is by address.** An attacker who reaches the host both directly and
  through a relay gets both budgets. Both are bounded by the global backstop.

## 11. What is verified and what is not

Verified on loopback with real endpoints and the real migrated schema: the exchange, the gate, the
limits, the single use, the throwaway endpoint closing, pushed and pulled revocation, the failing
database, version and purpose handling, and every guard in the table above. Each guard was removed
by hand and the tests run: every removal made at least one test fail (the list is in the pull
request). One ordering is not tested directly: the connection is put in the live registry before the
list is read; removing that makes the pushed-revoke test fail, but a deterministic test of the
race itself would need hooks into iroh's accept path. The spike measured the transport, the relay path and
reconnects on a real phone (`docs/plans/native-bridge/spike-f/README.md`), on the older exchange.

Not verified: the new exchange on a real phone or through a relay; the stock Android camera handing
`hqpair://` to an app (it opened nothing for plain text; a link should work but is untested); a
carrier network or public wifi; behavior under Doze; the limits against a real flood. Those belong
to W4 and W8.
