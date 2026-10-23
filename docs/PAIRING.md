# Pairing machines with a QR code

Native Bridge pairing connects two machines of one person with no hub, no account and no VPN. One
machine shows a QR code, the other scans it, the person types six digits, and from then on each
reaches the other by key alone over [iroh](https://github.com/n0-computer/iroh) (QUIC, hole
punching, an optional relay as the fallback).

The protocol, its attack notes and its residual risks are in
[security/PAIRING.md](security/PAIRING.md). The plan it belongs to is
[plans/native-bridge-sprint.md](plans/native-bridge-sprint.md), workstream W3. The code is the
`hq-bridge` crate. This page is for people who build on it and for anyone trying `hq bridge`.

Nothing starts by itself. The `hq` binary does not call the crate at start-up, and the `hq bridge`
command group exists only when built with the `bridge` feature.

## Try it between two terminals

```bash
cargo build -p hq-cli --features bridge
```

On the machine that shows the QR code:

```bash
hq bridge pair
```

It prints a QR code and the same link as text (`hqpair://ticket/...`), then waits. On the other
machine:

```bash
hq bridge join                  # asks for the link: paste it and press Enter
```

(`hq bridge join <link>` also works, but a link on the command line is visible to every user of
the machine while it runs, and it holds the one-time code.)

The second machine prints six digits. Type them at the prompt on the first. The first pairs the
device only if they match, and only if you answer within two minutes. Then:

```bash
hq bridge serve                  # on either machine: stay reachable (an echo service)
hq bridge peers                  # who is paired
hq bridge ping <peer id>         # connect by key alone and time a few round trips
hq bridge revoke <peer id>       # take a peer off the list
```

`--relay none` (the default) uses direct paths only, which is what two machines on one network
need. `--relay default` adds n0's public relays, which are for development and testing, not for
production. `--relay <url>` uses a relay you run. `--no-addrs` keeps this machine's IP addresses
out of the ticket (smaller QR code; the other side then needs a relay to find the direct path).
`--bind <ip:port>` chooses where it listens; the default is every address and any port. A peer
stores the address it was paired at, so with `--relay none` (no relay to find the new one) give
`pair`, `serve` and `ping` the same fixed `--bind` port, or a restart makes the stored address stale.

A ticket works for 10 minutes (`--ttl` 10 to 3600 seconds) and once. Run `hq bridge pair` again for another device.

The key lives in `~/.hq/bridge/bridge.key` (mode 0600; a key that others can read is refused) and
the paired list in the `bridge_peers` table of the vault database.

## The link

```
hqpair://ticket/<base64url of a small JSON object>
```

The scheme is the constant `hq_bridge::LINK_SCHEME`. Whether to move to an https link, which the
platform can verify and which tells someone without the app where to get it, is an open owner
decision, so nothing else spells the prefix out. The parser ignores the case of the scheme and also
reads the older `hqpair1.<same body>` form. A ticket from the spike (layout 1) is refused with a
message saying it is from an older version.

On Android the app registers the scheme in its manifest so the stock camera can hand the link to
it:

```xml
<intent-filter>
    <action android:name="android.intent.action.VIEW" />
    <category android:name="android.intent.category.DEFAULT" />
    <category android:name="android.intent.category.BROWSABLE" />
    <data android:scheme="hqpair" android:host="ticket" />
</intent-filter>
```

The camera's behavior with this link has not been tried yet (the spike's camera opened nothing for
plain text).

## Using the library

All of it is in `hq_bridge`. A caller owns the process, the database and the person's screen.

```rust
let bridge = Bridge::start(BridgeConfig { key, bind, relay, name, with_addrs, advertise },
                           db, Some(echo_handler())).await?;   // None: dial only, accept nothing
```

Showing a QR code:

```rust
let mut open = bridge.open_ticket(Tuning::default()).await?;   // one throwaway endpoint
show(open.ticket.link());                                      // QR code and copy-paste text
while let Some(request) = open.requests.recv().await {         // ends when the ticket is over
    // request.name is untrusted text; request.joiner is the key; request.sas is what to expect.
    let typed = ask_person_to_type_the_digits();               // never a yes/no button
    let _ = request.decide.send(typed == request.sas);         // Err: the answer was too late
}
```

Scanning:

```rust
let ticket = Ticket::parse(&scanned_text)?;
match bridge.join(&ticket, &Tuning::default(), |digits| show_to_person(digits)).await? {
    JoinResult::Paired(joined) => { /* the host is recorded as a peer */ }
    JoinResult::Refused(why) | JoinResult::Declined { .. } => { /* tell the person */ }
}
```

Afterwards: `bridge.connect(&peer_id)` dials a paired peer and returns a connection whose streams
the other side's `StreamHandler` receives (`PeerStream { peer, send, recv }`, already checked
against the list). `bridge.revoke(&id)` writes the revoke and closes the peer's open connections
at once. `bridge.card()` is this machine's id and how to dial it. `hq_db::bridge` has the list
(`list`, `get`, `is_paired`, `revoke`).

`Tuning` holds the ticket's lifetime, the global backstop (1000 wrong codes), the per-source
limits (`LimitConfig`: 5 strikes per address, 20 per relay, 2 exchanges per source, 8 in all, 4096
sources remembered) and the two timeouts (10 s per step, 120 s for the person). The defaults are in
`code.rs`, `limits.rs` and `host.rs`.

### Registering a device with an `hq edge`

The same exchange runs under the protocol name `hq/register/<version>` for the edge (workstream
W1). The edge is the machine that shows the QR code; the device scans it.

```rust
// On the edge: implement Registrar (accept: record the device, say what to return; details:
// optional, how to dial the device back), then
let open = OpenTicket::open(PairingSpec { purpose: Purpose::Register, card, registrar, bind,
                                          relay, with_addrs, advertise, tuning }).await?;
// On the device:
let result = hq_bridge::join::join(bridge.endpoint(), &ticket, &JoinOptions::new(name, relays, addrs),
                                   |digits| show_to_person(digits)).await?;
// JoinResult::Paired(joined): joined.card is the edge's identity, joined.info is what the
// registrar returned (for instance the name it gave the device).
```

A registration adds no row to `bridge_peers`: an edge is not a peer that may open streams to the
device. W1 stores the card and the info itself. A pairing ticket cannot finish a registration and a
registration ticket cannot finish a pairing; both are covered by tests.

The macOS app ([DESKTOP.md](DESKTOP.md)) is the first graphical host: it shows the QR code, takes
the six digits typed by the person and revokes through the one running `Bridge`, using only this
library.

## What listens, and when

| Listener | When |
|---|---|
| Long-term endpoint (one UDP socket) | Between `Bridge::start` with a handler and `shutdown`. `hq bridge serve` runs it until interrupted |
| Throwaway endpoint (one UDP socket per ticket) | From `open_ticket` until the ticket is used, expired or burned. `hq bridge pair` runs it until then |

Without a handler, `Bridge::start` binds a socket to dial out but accepts no protocol. The commands
`join`, `ping`, `peers` and `revoke` never accept a connection.

## Tests

```bash
cargo test -p hq-db bridge
cargo test -p hq-bridge                       # unit tests and the loopback end-to-end tests
cargo clippy -p hq-cli --all-targets --features bridge -- -D warnings
cargo test -p hq-cli --features bridge --bin hq -- bridge_commands
```

The end-to-end tests use real iroh endpoints on loopback with no relay, the real migrated schema,
and `::1` as a second source address. They need no network. The spike's tests carry over: its
file-mode and symlink tests of the paired text file became the database tests (a failing database
closes the gate) and the key-file mode test, and its single global attempt budget became the
per-source and backstop tests.
