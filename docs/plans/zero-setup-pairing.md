# Zero-setup pairing: the plan for connecting machines

Goal: a stranger downloads HQ, scans a QR code shown by the desktop app, and the two machines have a
secure direct connection. No WireGuard or Tailscale setup, no hub to run, no account. External
connectors such as claude.ai must still connect securely.

The WireGuard path (`hq net`, `hq-tunnel`, `docs/PRIVATE_NETWORK.md`) stays as the power-user and hub
fallback. This plan is the end state for everyone else. It builds on the Native Bridge work (iroh:
dial a machine by its key, QUIC, hole punching, a blind relay as the fallback), whose spikes A to E
live on the branch `claude/native-bridge-validation` and are summarized below.

## What is already known

- The unmodified host gate runs over an iroh connection in both directions. An unpaired key is closed
  with 403 before any handler runs.
- From a home connection to a VPS in another country the connection was direct in 10 of 10 trials,
  319 to 360 ms to connect, 158 to 166 ms transport round trip.
- iroh builds for Android with the NDK unchanged (4.3 MB size-optimized) and for iOS with two small
  patches (which lose network-change detection).
- Spike F (this directory's sibling, `native-bridge/spike-f`) adds the missing step: pairing with a
  one-time ticket instead of editing a file, and reconnecting by key alone.

## Two doors, kept apart

| | Door 1: your own devices | Door 2: external services (claude.ai) |
|---|---|---|
| Transport | iroh: dial by key, hole punching, blind relay fallback | Public HTTPS on a name you own, TLS ending on your machine |
| Setup for the user | Download, scan a QR | An optional step, later: a public address |
| Authentication | Each device's key pair; the QR carries a one-time code, never a secret key | OAuth with a consent you approve per connection |
| Exposure | One UDP port or a relay; unpaired keys closed at the handshake | `/mcp`, the OAuth endpoints and two `.well-known` paths only |

The doors must not depend on each other. Tests to keep: the bridge never serves connector paths; the
connector never accepts bridge credentials; turning one off leaves the other unchanged; revoking one
does not touch the other.

`docs/CLAUDE_CONNECTOR.md` needs a public HTTPS origin Anthropic's servers can reach and a private
HTTPS origin for the consent page (a tailnet name today). Without Tailscale the consent page is served
on the desktop app's own loopback proxy, which a browser treats as a secure context and the internet
cannot reach. The public origin remains a real requirement (a domain and a reachable machine); a QR
scan cannot create a public name, so the app guides it as an optional step.

## The relay decision

Direct connections need no relay. A relay is the fallback when UDP is blocked or both ends sit behind
strict NATs. It carries ciphertext only; it sees who connects, when and how much.

| Option | User setup | Cost and control | Verdict |
|---|---|---|---|
| R1. n0's public relays (the iroh default) | None | Free, but their terms say development and testing, throttled to 5 MB/s unauthenticated; a third party is in the path for the connections that need it | Fine for the spike. A production default only if their terms allow it |
| R2. n0's paid relay | An account | A bill and a vendor | Only if R1 is refused and R3 is too hard |
| R3. Self-hosted `iroh-relay` | A public machine, a name, TLS on 443 | Free and ours | The right answer wherever a public machine exists; `hq` on a public server could run one, off by default |
| R4. A relay the project runs | None | The project operates infrastructure and carries abuse and cost | Rejected for now |
| R5. No relay | None | Direct only, plus same-network discovery | Works on most networks; fails on the rest with a clear message |

Recommendation: ship direct-first with the relay URL as a single config value; decide the production
default from the measured table (how many real networks need a relay at all). The relay must be blind
either way, and a paired device must work with no relay.

## Spike F: the pairing slice

`native-bridge/spike-f`: `show` (the machine showing the QR), `join` (the one scanning), `dial`
(reconnect by key), `stranger` (an unpaired key).

1. `show` prints a ticket, as text and as a QR code: its endpoint id, optional relay and addresses, a
   one-time code (10 minutes, one use, fifty wrong guesses across all callers burns it).
2. `join` dials by key, proves the code inside the encrypted channel and commits to a nonce; the host
   answers with its own, then the joiner reveals. Both sides compute six digits from both ids, the
   code and both nonces. The joiner shows them; the person at `show` types them, and the host pairs
   the device only if they match and the answer came in time.
3. On acceptance each side stores the other's id. After that, only the key is needed: no code.

## What Spike F found

- Pairing works end to end between a Mac and an Android emulator, with the public relay on: the six
  digits match, the ticket is spent on first use, and a ticket with only the host id and relay (no
  addresses) pairs and then upgrades to a direct path in about two seconds. So the QR can be small and
  carry no IP address.
- A live connection costs about 0.28 MB an hour idle (about 200 MB a month) and iroh does not let the
  heartbeat be lengthened. A phone on metered data should connect when it is needed, not stay
  connected.
- On Android, outside an app process, a worker thread panics with `android context was not
  initialized` (`ndk-context`, through `n0-dns-resolver` and `netdev`). Everything still worked. It has
  to be understood before the relay is relied on from the `hq` child process.
- An independent review found a weak digit scheme and a late-answer bug (a late "y" paired a device
  that had been told "timeout"); both are fixed with tests, along with open connections surviving a
  revoke, unbounded pairing connections and an untrusted paired list. What the review left for the
  product (denial of service on the pairing door, per-ticket throwaway keys, push revocation) is in
  the spike's README.
- On a real phone (details in the spike's README): pairing took 1.2 to 2.7 s on the same LAN and on
  mobile data; a relay-only ticket still ended on a direct path from behind carrier address
  translation; with all UDP dropped it pairs and works over the relay (about 0.5 s a call). A wifi to
  mobile-data switch stalls a live connection about 18 s, so the app must report network changes
  and retry. The phone camera read the terminal QR first time, but a plain `hqpair1.` text opens
  nothing: the payload should be a link the camera can hand to the app.
- Not yet measured: a carrier hotspot and public wifi (no second phone, guest wifi pending), and
  the sleep rows inside a real app.

## After the slice, in order

The pairing protocol written up with attack notes and an independent review; the desktop app (Tauri)
around pair, permissions, host lifecycle and the loopback proxy that also serves the connector's
consent page; the Android side (iroh inside `hq` as the child process, QR scanning with CameraX and
ZXing, no proprietary dependency); the connector regression tests and the guided public-address step;
then the decision about the WireGuard path and Tailscale from data.
