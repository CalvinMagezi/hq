# Spike F: pairing with a one-time ticket, then reconnecting by key

Throwaway crate, outside the HQ workspace. Spikes A to E (on the branch
`claude/native-bridge-validation`) showed that the host gate runs over iroh and that iroh runs on a
phone. They paired by editing a file. This one builds the step a stranger needs: scan a QR code,
type six digits, done. The plan around it is `docs/plans/zero-setup-pairing.md`.

| Command | Runs on | What it does |
|---|---|---|
| `show` | the machine showing the QR | Opens a ticket (10 minutes, one use), prints it as a QR and as text, asks the person to type the digits shown on the other device, records the device only if they match |
| `join` | the machine scanning | Dials the host by key, proves the ticket's code, prints the six digits for the person to type on the host, stores the host on acceptance |
| `dial` | the paired machine | Reconnects by key alone; reports path, connect time, round trip, idle cost |
| `stranger` | anywhere | Dials with a key nobody paired, to show what it gets |

## How pairing works

1. `show` makes (or loads) its key and a random 8-byte code, and prints a ticket
   `hqpair1.<base64url json>`: its endpoint id, optional relay and addresses, the code. No secret key.
2. `join` dials the host's id (so the host is authenticated by the key in the QR) on the protocol
   `hq/pair/1` and sends the code inside that encrypted connection.
3. The hello carries the code and a commitment (a hash) to a random nonce of the joiner's. The host
   checks the code in constant time. Wrong guesses are counted across all callers; the fiftieth burns
   the ticket (the code is 64 bits, so fifty guesses succeed with odds near 3e-18). A correct code is
   spent at once, so a photograph of the screen is worth nothing after the first use.
4. The host answers with its own nonce; only then does the joiner reveal its nonce, and the host
   checks it against the commitment. Fixing one side before seeing the other is what stops a machine
   in the middle grinding for matching digits. Both sides then compute six digits from both ids, the
   code and both nonces.
5. The joiner shows the digits. The person at the host types them; the host sends "accept" only if
   they match what it computed, so a habitual "y" cannot pair a device. The pairing is written by the
   handler, only when the answer arrived inside the decision window and the joiner is still
   connected: a late answer leaves the list unchanged and the host is told it was too late. The
   joiner stores the host's id.
6. The gate reads the paired list on every handshake, and an open connection rechecks it on every
   stream and once a second, so removing a line cuts off a device that is already connected. The list
   is trusted only if it is a plain file owned by the host's user and not writable by others.

Two layers sit in front of the protocols: while no ticket is open the pairing protocol is closed at the
handshake (410), and the other protocol is closed to every key that is not paired (403), both before
any handler runs. The tests count handler arrivals to prove that.

## Run it

```
cargo build --release          # about a minute; needs Rust 1.91 or newer
cargo test                     # 11 unit tests and 12 end-to-end tests on loopback, no relay
B=target/release/pairspike
touch paired.txt; chmod 600 paired.txt
$B show --key host.key --paired paired.txt --relay default --serve      # prints the QR and the ticket
$B join --ticket <ticket> --key dev.key --store dev.json --relay default
$B dial --key dev.key --store dev.json --relay default --calls 20
$B dial --key dev.key --store dev.json --relay default --idle 120       # bytes an hour when idle
$B stranger --ticket <ticket> --relay default
```

`--relay none` (the default) measures direct paths only. `--relay default` uses n0's public relays,
which are meant for development and testing; use them only to compare. `--no-addrs` leaves the host's addresses out of the ticket (smaller QR, no IP address in it; needs a relay). `--advertise ip:port` adds an
address to the ticket when the host listens on every interface. `--keepalive S` sets the transport's
keep-alive.

For Android: `cargo build --release --target aarch64-linux-android` with the NDK's clang (as in Spike
D), `adb push`, run from `/data/local/tmp`.

## Results (2026-10-22)

Loopback, macOS, no relay: all 23 tests pass, clippy clean; each guard was mutation-checked (the
gate, the single use, the attempt cap, the handshake-level close, the commitment check, the decision
timeout, the revocation tick, the file mode and symlink checks, the stream timeout): breaking any one
fails a test.

Mac host with the public relay on, Android 16 arm64 emulator (behind the emulator's NAT) as the
joiner:

| Check | Result |
|---|---|
| Pair with the full ticket | Paired; the six digits matched on both screens (protocol v1; see below) |
| Pair with a ticket that has only the host id and relay (no addresses) | Paired; the connection started on the relay |
| Reconnect by key alone, full ticket store | Direct, connected in 579 ms, round trip 2.4 ms, `host.status`-sized echo p50 3.1 ms |
| Reconnect by key alone, relay-only store | Connected in 2.04 s, then moved to a direct path; echo p50 2.6 ms |
| Unpaired key | Closed with 403 at the gate |

Findings:

- **A ticket needs no addresses.** The relay-only ticket paired and then upgraded to a direct path in
  about two seconds. Leaving addresses out makes the QR much smaller (the full ticket is about 540
  characters) and stops it from carrying the host's local and public IP addresses.
- **A live connection costs about 0.28 MB an hour when idle, and that cannot be tuned down.** Over 90
  idle seconds a paired connection moved 2.7 kB out and 4.2 kB in at keep-alive settings of 5, 15 and
  30 seconds alike. iroh's per-path heartbeat is fixed at 5 seconds and it ignores larger values (its
  source says so), so the setting has no effect. That is about 6.7 MB a day, roughly 200 MB a month,
  per connection that stays open: acceptable on wifi, a real cost on metered mobile data. A phone that
  must not hold a connection open all the time needs another wake-up path (connect when the app is
  open, or a push), which is the plan's open question P4.
- **Android prints a panic outside an app process.** A tokio worker panics with `android context was
  not initialized` (`ndk-context`, reached through `n0-dns-resolver` and `netdev`) when the binary is
  run as a plain process. Pairing, the relay path and reconnects all worked despite it. Inside the real
  app, the `hq` child process has no JVM context either, so this needs a decision: initialize the
  context, patch the two crates, or confirm what the resolver falls back to.
- **Not measured yet** (need the owner's hands or a second network; `RUNBOOK.md` has the steps): a carrier hotspot, public wifi,
  UDP blocked with only the relay, sleep and wake, a network change mid-session, and the time of the
  whole pairing on a real phone camera.

## Independent review (2026-10-22)

A reviewer with no part in the design read the code and attacked the protocol. Fixed in this spike,
each with a test: the digits are now a commit-then-reveal over both nonces and the person types them
(before, a nonce-free hash that the host also sent, and a "y" paired); the pairing is written only
for an answer in time and a joiner still connected (before, the host UI wrote it before telling the
handler, so a late "y" paired a device that was told "timeout"); a connection that never opens a
stream, or never finishes a step, is closed after 10 seconds, and at most four pairing exchanges run
at once; removing a device closes its open connection; the paired list must be a plain file owned by
the host's user and not writable by others; text from the other machine is stripped of control
characters before it is printed.

The emulator rows above were taken on protocol v1, before the nonces and typed digits; the transport,
the relay path and the reconnect behaviour they measure did not change, but the new exchange has only
run on loopback.

Left for the product, on purpose:

- **Denial of service.** Four idle pairing connections hold every slot for ten seconds, and fifty
  wrong codes burn the ticket. Both are cheap for a stranger who knows only the host's id. The product
  should pair on a throwaway key made per ticket (so burning one reveals nothing about the long-term
  identity) and limit per source, with the large budget kept as a backstop.
- **Check then open.** The paired-list checks and the open are separate calls; the product keeps the
  list in the database and writes it in a transaction.
- **Revocation is polled.** A once-a-second check and a per-stream check are enough for a spike; the
  product pushes the removal to open connections.
- **A device that vanishes just as it is accepted** can still end up on the list (the host records
  it after checking the connection is open, which narrows the window but cannot close it).
