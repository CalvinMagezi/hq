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
`hq bridge` is in every Linux and macOS build (the default `bridge` feature); Windows builds leave it out.

| Listener | Exists | Accepts | From |
|---|---|---|---|
| Long-term endpoint, one UDP socket | From `Bridge::start` with a handler until `shutdown`. Without a handler it only dials out and accepts no protocol | `hq/bridge/1`, and `hq/svc/1` when services are configured (section 14) | Peers on the paired list; others are closed at the handshake (403) |
| Throwaway endpoint, one UDP socket per open ticket | From `Bridge::open_ticket` (or `OpenTicket::open`) until the ticket ends | `hq/pair/<v>` or `hq/register/<v>`, one purpose per ticket | Anyone who has the ticket, until it is used, expires, is burned, or the source is shut out |

On Android the phone app runs `hq bridge run` as a child process of the hub service (section 12). It
is the long-term endpoint plus one throwaway endpoint while a ticket is open, and it exists only
after the owner turned "Paired devices" on.

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
   The record includes the address the pairing connection came from, so the peer can be dialed
   even if its details never arrive.
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
bound, the key file creation.

### Review of the fix round

A third fresh agent read the diff of the fixes above. It reported one must-fix and several
should-fix items; each was checked and closed:

| Finding | Fate |
|---|---|
| Must-fix: `hq bridge join` with no argument read to end of input, so the documented paste flow hung after Enter | It prompts and reads one line |
| A peer whose details never arrive was recorded with no way to dial it (`Minimal` preset: no address lookup), and a re-pair wiped good dial info | The address the pairing connection came from is recorded at accept time; an empty list never replaces a known one. Test with a joiner that leaves before the details |
| The joiner's own dial, writes and open had no deadline | `connect_timeout` and the step deadline on every joiner write. Test with a host that never reads |
| Duration arithmetic on caller-supplied timeouts could panic | Saturating |
| `printable` missed other invisible format characters (Arabic mark, tag block, blank fillers) | Wider list, test |
| A machine with many interfaces made a ticket longer than the joiner accepts | The sender caps the list (8 for a ticket, 4 for a card), the person's addresses first |
| Two tests did not guard what they named | One rewritten with a joiner that really leaves; the write-deadline test now fails when only that deadline is removed |
| The protocol changed under version 3 | Version 3 never left this branch (nothing shipped speaks another v3), so it stays 3. The next change that has to interoperate with a released build bumps it |
| The joiner dials with its long-term key, so a rogue QR learns its id and address | Documented in section 10 |
| `endpoint_closed` is a state flag, not a released socket | Documented in the method; the socket goes when the ticket is dropped |

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
- **A rogue QR learns the joiner's long-term id and source address.** The joiner dials the
  throwaway endpoint with its own long-term key so the host can record who it is, so whoever runs
  the endpoint behind a QR code sees both from the handshake. Its dial-back relays and addresses
  are held back until the host proved its identity, but the id and the address it connected from
  are not hidden. A throwaway key for the joiner would hide them at the cost of the host not
  learning the joiner's real id from the connection; a later version could prove the long-term
  key inside the exchange.
- **A two-party window remains at acceptance.** The host records the device, then sends the
  answer. If that send fails, or the joiner cannot record the host, the host shows "paired" and
  the joiner an error. Pairing again heals it (the record is an upsert).
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

## 12. The phone app (W4)

The Android app drives the bridge through `hq bridge run` (`docs/HQ_MOBILE.md`, "The bridge on the
phone"). What that adds, and what it does not.

- **No new listener except the bridge's own.** The app and `hq bridge run` talk over the child's
  standard input and output. There is no control socket, no loopback port and no token for them, so a
  process on the phone that is not the app cannot send it a command, and the child ends when the app's end
  of the pipe closes. The bridge's UDP sockets are the ones in section 3. They are off until the owner
  turns "Paired devices" on or starts a pairing; a pairing that does not succeed leaves them off, and one
  that does turns the switch on for good.
- **The digits stay in `hq`.** The `request` event names the device and carries no digits. The app
  sends what the person typed (`digits`) and `hq` compares it with the answer it computed. The app
  never learns the answer, so no "yes" button can be added to it and a bug in the screen cannot pair a
  device. A wrong answer declines the pairing and burns that ticket, as on the command line.
- **A link is a screen to show, never an order.** `hqpair://ticket/...` can arrive from the stock
  camera, a browser or any app. `PairLinkActivity` checks its shape (a fixed prefix, URL-safe
  characters, 2000 characters at most, the bound `hq` applies) and hands the text to the pairing screen, which asks the owner
  before anything is dialed. Following it would pair the phone with whoever made the link, which is the
  attack in section 7 that the QR code's authenticity is meant to stop; the stock camera cannot show who
  made a code, so the confirmation says so. A link does not replace a pairing already in progress. A code
  scanned inside the app, or a link pasted into it, is the owner's own act and starts at once. The pairing screen is closed to other apps (it needs a
  permission only `adb` holds), so only the link door is open.
- **The link and the digits are not logged.** The control events that hold them are never written to
  the device log (the log names the event type only), `hq bridge run` writes no event to a file, a
  line that does not parse is not echoed in the error, and "Copy the link" marks the clipboard entry as
  sensitive where Android supports it. The link is shown on screen as a QR code while a ticket is open;
  in a build that is not debuggable the code and the digits screens carry `FLAG_SECURE` (no screenshots,
  no recents thumbnail) and ignore touches that pass through another app's window.
- **A line in `files/bridge_relay` cannot become a flag.** Only `none`, `default` or an `https://` address
  without spaces is passed to `--relay`; anything else is the default.
- **The child has no credentials.** It starts with an environment of its own (home, temp, the vault
  path) and none of the hub's secrets, and the paired list and key are in the app's private directory.
- **The camera is used while the scan screen is in front.** Frames go to the QR decoder and nowhere
  else; nothing is stored or sent. No Google Play services are used: CameraX and ZXing, both Apache-2.0.
- **A process abort in the child was a denial of service on the bridge.** With `ndk-context`
  uninitialised the first endpoint bind aborted `hq` (measured: SIGABRT). It is fixed (section 12 of
  `docs/HQ_MOBILE.md`). The fix sets a context with null pointers, which only `netdev` ever reads and
  which it checks. If a future iroh code path reads the context without that check, it aborts again, so
  every endpoint `hq` builds must be given `hq_bridge::dns::resolver()`.

### Residual risks of the app

- **A same-user process on the phone is the app.** Anything running as the app (a child it starts)
  can read the key and the paired list. The phone is debuggable (`TECHDEBT.md`), so `adb run-as` does too.
- **The relay default is n0's public relay**, as in section 10: development and testing only.
- **The confirmation screen is only as good as the person reading it.** A convincing prompt to "pair"
  from a page the owner trusts is a social attack no screen can stop. The owner still has to type the
  digits on the machine that showed the code, so a stranger's link alone pairs nothing.

## 13. The edge protocol on the device (W6)

A device registered with an `hq edge` (`docs/EDGE.md`) lets that edge open streams on a second
protocol name, `hq/edge/1`, and pipes each to the connector door on loopback. The edge sees only
TLS bytes; the device ends TLS itself (`docs/security/CONNECTOR_TLS.md`). The gate treats the two
protocol names as two lists.

| # | Who | Tries | What happens | Test |
|---|---|---|---|---|
| E1 | A stranger, or a paired peer | Opens `hq/edge/1` | Closed at the handshake, no handler ran: only the id in `bridge_edge` is admitted, read from the database on every handshake and failing closed | `nobody_but_the_registered_edge_may_open_the_edge_protocol` |
| E2 | The edge | Opens `hq/bridge/1` | Refused: it is not a paired peer, and registering adds none | `the_edge_cannot_use_the_service_protocol_and_a_peer_cannot_use_the_edge_one` |
| E3 | A device with no edge door | Is dialed on `hq/edge/1` | The protocol is not offered at all | `a_bridge_without_an_edge_door_does_not_speak_the_edge_protocol` |
| E4 | A rogue or compromised edge | Names this device something else at registration (another device's name) | Nothing is recorded: the name must be `<label>.<domain>` with the label this device's own key gives it and a plain DNS domain. The domain itself is whatever the edge serves devices under; that is the edge's to say | `an_edge_that_names_the_device_wrongly_registers_nothing`, `bridge::tests::a_name_that_is_not_this_devices_own_label_under_a_plain_domain_is_refused`, `the_domain_comes_from_the_name_and_is_not_cut_short` |
| E5 | The person | Forgets the edge, here or in another process | Open connections are closed at once from here; from another process the next stream on an open connection is refused and closes it | `forgetting_the_edge_closes_its_open_connection_and_refuses_the_next`, `an_edge_cleared_by_another_process_gets_no_more_streams_on_an_open_connection` |
| E6 | The edge, or whoever holds its key | Opens many streams to exhaust the device | At most 64 piped at once (`max_streams`); extra streams are dropped and counted; each piped stream ends on 150 s of silence | `streams_over_the_cap_are_dropped_and_counted` |
| E7 | The edge, or a caller of the library | Aims the pipe at another address | The target is an absolute socket path or a loopback address with a fixed port, checked by `Bridge::start_with_edge_door` itself (not only by the CLI); the stream carries bytes only | `door::tests::the_edge_can_only_be_pointed_at_a_socket_path_or_a_loopback_port`, `a_door_the_edge_must_not_be_pointed_at_is_refused_at_start` |
| E8 | A pairing ticket | Registers a device with an edge, or the reverse | Refused by `Bridge::register` and `Bridge::join`, and by the protocol name | `a_pairing_ticket_cannot_register_a_device` |

| E9 | Another local user or app | Binds the door's address first so the forwarded bytes (and the CA's validation) reach it | The door is a unix socket in a private directory (owner, no group or other write, never a link) with a `0600` socket; a loopback TCP port is an explicit opt-in because it cannot be defended. A bind that fails is retried and reported | `listener::tests::*`, `the_default_unix_door_gets_its_own_certificate_and_serves_the_connector` |
| E10 | A hostile edge | Offers a relay such as `http://127.0.0.1:6379` or an internal name | Only a plain https URL on a DNS name is kept (not an address, `localhost`, `.local`, `.internal`, no port or credentials, 253 characters at most); anything else fails the registration | `bridge::tests::only_a_plain_https_relay_on_a_name_is_adopted` |
| E11 | A pasted link | Swaps the real edge for another | `hq bridge register` refuses while an edge is registered unless `--replace` is given; replacing closes the old edge's connections | `registering_with_another_edge_closes_the_one_it_replaces` |
| E12 | The CLI in another process | Forgets the edge while a connection is open in this one | Each open connection rechecks the record every second and on every stream; `Bridge::forget_edge` sweeps the edge's connections whether or not a record was left | `forgetting_sweeps_the_edges_connections_even_when_another_process_cleared_the_record` |

Residual risks: the edge learns when a device is online and how much it moves (it sees the TLS server
name and sizes, never content), and a stolen edge key lets its holder open streams to the connector
door, where TLS, the path filter and OAuth still stand in the way. The piped stream is plain bytes
to a loopback port: any bug in the door is reachable by the edge, which is why the door is the
connector's own gate (`docs/security/CONNECTOR_TLS.md`) and nothing else.

## 14. Services and forwards between paired machines

Paired machines reach each other's services over a third protocol name, `hq/svc/1`
(`crates/hq-bridge/src/services.rs`). The first bytes of a stream name a service; the machine that
offers it maps the name to a loopback target from its own configuration and pipes the stream there.
A forward is the client side: a loopback port whose connections each become one stream.

| # | Who | Tries | What happens | Test |
|---|---|---|---|---|
| S1 | A stranger, or the edge | Opens `hq/svc/1` | Closed at the handshake: the gate admits paired peers only, as for `hq/bridge/1` | `a_machine_that_is_not_paired_gets_nothing` |
| S2 | A paired peer | Names a service this machine does not offer, or a malformed name | One status byte says "unknown" and the stream ends; nothing is dialed. A name is 1 to 32 bytes of `a-z0-9-` | `an_unknown_service_is_refused_by_name`, `names_are_short_lowercase_ascii` |
| S3 | A paired peer | Aims a stream at another address | The protocol carries a name, never an address. Targets are a loopback address with a fixed port or an absolute socket path, checked at start | `services_lead_to_loopback_ports_or_socket_paths_only` |
| S4 | A paired peer | Uses the web app as loopback, to skip sign-in | The web server's own port may not be a service (`hq bridge serve` refuses it). The paired-devices door (`bridge.web_listen`) opens only with the configured web token set (a device token alone is not enough: revoking the last one would turn sign-in off) and `HQ_MCP_DEV_NO_AUTH` off. It strips every forwarding and tunnel-identity header a device sends and marks each request as forwarded, so the development switch cannot apply even if turned on later | `a_service_may_not_lead_to_the_main_web_port`, `stays_shut_without_sign_in_or_with_the_dev_switch` |
| S5 | A paired peer | Uses the connector through the bridge, or a connector grant on `/mcp` | The paired-devices door answers 404 for `/oauth/*`, `/.well-known/*` and `/mcp/*`, and passes `/mcp` only with a configured API key or the session token of an agent HQ launched (`agent_host.agent_mcp_url`): a connector grant or the web token is refused before any handler runs, and a revoked session token is refused like any other. The two doors share no path and no credential | `serves_the_owners_api_and_keyed_mcp_and_nothing_of_the_connectors` |
| S6 | A revoked peer | Keeps an open stream | `Bridge::revoke` closes its connections at once. A revoke from another process is seen within a second on connections the peer opened (each rechecks the list), and by a forward before its next local connection, which then closes the connection it kept to that peer | `revoking_closes_open_streams_and_refuses_new_ones`, `a_forward_stops_when_its_peer_is_revoked_from_another_process` |
| S7 | A paired peer, or a local process | Opens many streams or local connections | At most 64 service streams piped at once over all peers, extra ones dropped and counted; a direction silent for an hour is ended. A forward handles 256 local connections at once and gives up dialing after 20 s | shared with the edge door (E6) |
| S8 | Another local user | Connects to a forward's port | A forward listens on loopback, so any local process can use it, with whatever credentials the far service asks for (the web token, an API key, the host gate's signed challenge). Use forwards on single-user machines | residual risk |

Departure from the two-doors plan: `docs/plans/zero-setup-pairing.md` keeps `/mcp` off the
own-devices door entirely. The laptop needs the phone's `/mcp` for `task_peer` and `remote_mcp`, so
this door serves it to API keys, which is the credential that path already used before the
bridge. Agents HQ launches on a paired laptop call back to `/mcp` with their own session token, so
that token is accepted too: it is minted per session, scoped by the session allowlist and revoked
when the session ends. Connector grants still work only on the connector's own door.

Residual risks: a paired device is trusted with whatever the services it can name lead to, so offer
only what the owner's other devices need. The relay (when one is used) sees that two machines talk
and how much, never content.

## 15. Setup after pairing (`hq/setup/1`)

A fourth protocol name, `hq/setup/1` (`crates/hq-bridge/src/setup.rs`), wires a hub and a machine
once they are paired (`docs/PAIRING.md`, "What pairing sets up"; design in
`docs/plans/pairing-autoconfig.md`). The pairing protocol itself is unchanged: a peer that does not
speak setup stays paired with nothing set up.

What each message may set:

- **Machine to hub** `{v, role:"machine", name, gate, mcp_port}`. `name` becomes the hub's host name,
  only as a slug (lowercase letters, digits and `-`, at most 23 characters, the peer id's first 8 hex
  added when the name is taken, built in or configured); it is never used as given. `gate` says
  whether the hub should expect the service `gate`. `mcp_port` is the machine's own forward port,
  used only to write the URL agents on that machine call back to
  (`http://127.0.0.1:<mcp_port>/mcp`); it must lie from 47400 to 47999, and the hub listens on
  nothing because of it.
- **Hub to machine** `{v, role:"hub", name, web, mcp_key, gate_key}`. `name` becomes the machine's
  name for the hub, as a slug. `mcp_key` is a per-device key (`hqd_` and 64 hex), or empty when the
  hub already holds a live one for this machine. `gate_key` is written to the machine's
  `gate_authorized_keys` with `from=127.0.0.1` and the label `bridge:<first 8 hex of the hub's id>`,
  through the gate's own checked file code, unless that line is already there.

Neither message carries an address, and each side picks its own ports. The rules and the test that
holds each one:

| # | Who | Tries | What happens | Test |
|---|---|---|---|---|
| U1 | A stranger, or the edge | Opens `hq/setup/1` | Closed at the handshake: the gate admits paired peers only. Nothing is stored or minted | `a_stranger_gets_nothing_and_two_machines_set_nothing_up` |
| U2 | A paired machine | Pairs with another machine, or a hub sends a machine's message | Only a hub accepts the protocol, and a hub refuses a message that says `role:"hub"`; a machine refuses an answer that is not from a hub. Two machines or two hubs set nothing up | `a_stranger_gets_nothing_and_two_machines_set_nothing_up`, `messages_with_unknown_fields_a_wrong_version_or_too_many_bytes_are_refused` |
| U3 | A paired peer | Sends more than 4 KiB, an unknown field (an address, say), a missing field or another version | The message is refused before anything is stored, and the connection is closed | `a_bad_message_from_a_paired_peer_sets_nothing_up`, `messages_with_unknown_fields_a_wrong_version_or_too_many_bytes_are_refused` |
| U4 | A paired peer | Picks a name that is a path (`../../x`), a built-in host or a configured one | The name is cut to a slug and suffixed when taken, so it can never name a file outside `~/.hq/gate/` or replace a configured host | `a_chosen_name_becomes_a_plain_slug_and_never_a_path`, `bridge_setup_never_takes_a_configured_name_port_or_forward` |
| U5 | A paired machine | Runs setup again, to collect more keys | A machine with a setup row sends nothing. A hub that gets a message for a peer it already set up keeps the host name, the port and the one live key, and answers with an empty key: there is never more than one live key per device. Only a new pairing (which needs the six digits again) starts setup over: `bridge::add_peer` deletes the old row and revokes the old key in its transaction | `running_setup_again_keeps_the_ports_and_mints_no_second_key`, `pairing_again_after_a_one_sided_revoke_gets_a_fresh_key`, `pairing_an_active_peer_again_starts_its_setup_over` |
| U6 | Anyone with the machine's key | Uses it on the hub | It opens `/mcp` with the owner's full scope (owner decision, 2026-10-31) on the paired-devices door and the main port. The hub stores only its sha256. The connector's TLS door refuses it like every credential that is not a connector grant | `a_paired_devices_key_has_full_scope_and_dies_with_the_pairing`, `a_paired_devices_key_opens_mcp_until_the_device_is_revoked`, `the_connector_door_never_accepts_the_credentials_of_the_own_devices_door` |
| U7 | A revoked peer | Keeps using what setup gave it | `bridge::revoke` revokes the device key and deletes the setup row in the same transaction, from every caller (the CLI, the phone's control loop, the desktop app, `Bridge::revoke`). The running bridge closes the setup forward and, on a machine, removes the hub's gate line at once or within two seconds when the revoke came from another process | `revoking_the_peer_revokes_its_key_and_deletes_its_setup`, `revoking_on_the_machine_undoes_its_side`, `revoking_on_the_hub_undoes_its_side`, `a_revoke_from_another_process_is_swept_by_the_running_bridge` |
| U8 | Setup | Moves the owner's gate, or rewrites their config | Never. The machine offers `gate` only when `agent_host.gate_listen` is a loopback address in `gate_allowed_sources`; the hub offers `web` only with `bridge.web_listen`. A configured service, forward, host or remote MCP name wins over setup's. Nothing is written to `config.yaml` | `a_bridge_setup_machine_offers_a_loopback_gate_that_admits_loopback_only`, `a_bridge_setup_hub_offers_its_door_and_the_config_wins`, `a_host_configured_by_hand_wins` |
| U9 | Another local user | Reads the machine's stored key | The key sits in the machine's database (`bridge_setup.mcp_key`), the same trust as `remote_mcp.api_key` in the config today. Single-user machines, as for forwards (S8) | residual risk |

Removing a gate line touches only lines whose label is exactly `bridge:` and eight lowercase hex
characters with no setup row behind them; every other line is kept byte for byte, and the file is
replaced whole after the same ownership and mode checks the gate uses when it reads it
(`removing_drops_only_the_picked_lines_and_keeps_the_rest_as_written`).

Residual risks: the machine's key opens the hub's whole `/mcp`, so a machine that is stolen while
paired has the owner's tools until it is revoked. The hub's gate key for a revoked machine stays in
`~/.hq/gate/`; the machine no longer authorizes it once it revokes too, and the hub never offers it
anything else. A revoke reaches only the side that made it; the other side's forward stops working
because the gate refuses the revoked key, and its row stays until it revokes as well.
