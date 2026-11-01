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
command group is in every Linux and macOS build (the default `bridge` feature); Windows builds
leave it out, so a Windows machine pairs from WSL2.

## Try it between two terminals

```bash
cargo build -p hq-cli
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
hq bridge run                    # the phone app's helper: JSON lines on stdin and stdout, see docs/HQ_MOBILE.md
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

The app handles it (`PairLinkActivity`): the link opens a confirmation screen and nothing is dialed
until the owner presses Pair. A VIEW intent for it was tried on an emulator and a phone; the stock
Camera app itself has not been tried yet (the spike's camera opened nothing for plain text).

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

### Registering this device with an `hq edge`

The device side of the registration above. `hq bridge register <link>` (or `Bridge::register`)
joins the edge's invitation, shows six digits for the person at the edge to type, and on success
records the edge: its id, its domain, the name this device is reached at and the relay it offered
(`hq bridge edge` shows it, `hq bridge forget-edge` clears it, `hq_db::bridge::edge_get` and
`edge_set` are the table). The name must be `<label>.<domain>` where the label is the first 8
bytes of this device's own id in hex and the domain a plain DNS name the edge serves devices under, so
an edge cannot hand over another device's name; otherwise nothing is recorded. The relay the edge
offers is kept only when it is a plain https URL on a name (not an address, `localhost` or a local
name, no port or credentials), because `--relay auto` makes it this device's home relay. A device
that is already registered is left alone unless `--replace` is given.

A serving bridge (`hq bridge serve`, `run`; `Bridge::start_with_edge_door`) accepts the edge
protocol `hq/edge/1` from the registered edge and from no one else, and pipes each stream it opens
to the connector door (`--edge-door`: a socket path or a loopback `ip:port`; default
`mcp_connector.tls.door_listen`, else a private socket under the vault's
`_data/connector_door/`; `off` turns the protocol away entirely). Paired peers speak `hq/bridge/1` and
cannot open the edge protocol; the edge cannot open the service protocol. Forgetting the edge cuts
its open connections (at once through `Bridge::forget_edge`; from another process at its next
stream). `--relay auto` uses the registered edge's relay, so the edge can find this device.

## What pairing sets up

Pairing a hub with a machine wires both with no configuration. The **hub** holds HQ: the phone app
(its helper `hq bridge run`), or a computer that runs `hq bridge` with `--as hub`. The **machine**
runs coding agents for it: any other computer, which is the default for `pair`, `join` and `serve`.
Right after the six digits match, the machine sends one setup message on its own protocol
(`hq/setup/1`) and the hub answers. Each side then has:

| Side | What it gets |
|---|---|
| Machine | its host gate offered to the hub as the service `gate`; the hub's gate key in `gate_authorized_keys`, from `127.0.0.1` only, labelled `bridge:<first 8 hex of the hub's id>`; a forward on a loopback port to the hub's `web`; a key minted for this machine alone that opens the hub's `/mcp`, used by `hq task` when no `task_peer` is configured |
| Hub | its paired-devices door (`bridge.web_listen`) offered as `web`; a forward on a loopback port to the machine's `gate`; a host named after the machine (`tcp` transport at that forward, with a gate key made for it under `~/.hq/gate/`); the machine's own forward as the `/mcp` address for agents launched there |

Ports are picked on each side from 47400 to 47999 and kept. What was set up lives in the database
(`bridge_setup`), never in `config.yaml`; the hub keeps only a hash of the machine's key. A restart,
or running setup again, reuses the ports and the key; a new pairing starts setup over with a new
key and revokes the old one.

Revoking the peer from either side cleans both sides of the pairing. The revoking side marks the peer
revoked, removes its setup row and device keys, prunes gate lines, closes forward listeners, and closes
open connections with code 403 and reason `revoked`. The other side learns of the revocation from this
connection close (or on its next dial if it was offline or disconnected) and runs the same local cleanup.
Only an explicit revoke says `revoked`: a peer this side has no row for hears `not paired`, and a
database error `unavailable`, and neither drops the other side's pairing. A hub deletes the gate key
it made for a revoked machine; a revoke learned from the peer also removes the Claude entries for it.

```bash
# On the laptop, with the phone app open on its pairing screen:
hq bridge join            # paste or scan the phone's link, then type the digits on the phone
hq bridge serve           # keeps the forward and the gate service up
```

Two things stay with the owner. The machine offers its gate only when `agent_host.gate_listen` is a
loopback address that `agent_host.gate_allowed_sources` admits (`hq bridge serve` prints a one-line
hint otherwise; setup never moves the gate). The hub offers `web` only when `bridge.web_listen` is
set and the web token is on. A computer pairing as a hub keeps `hq bridge serve --as hub` running
so the machine can reach it; a machine that could not reach the hub tries again at its next
`hq bridge serve`. Two hubs, or two machines, pair as peers and set nothing up.

Anything configured by hand wins: a service, forward, host or remote MCP entry with the same name
or port as one setup would make keeps the configured one, so a setup written by hand keeps working.

### Using HQ in Claude

The hub's `/mcp`, reached through the forward and opened with the machine's own key, is also what
Claude Code and Claude Desktop on that computer can use. Nothing is written to them unless you say
so:

```bash
hq claude add             # Claude Code (user scope) and Claude Desktop; name the hub if there are two
hq claude status
hq claude remove          # only what `hq claude add` wrote
```

After `hq bridge pair` or `hq bridge join` has set the machine up, the command asks once on a
terminal (`[Y/n]`) and otherwise prints `hq claude add <hub>`. The desktop app has the same thing
under Claude, "Use HQ in Claude", with an Add or Remove button per hub.

- **Claude Code** gets `claude mcp add --scope user --transport http hq http://127.0.0.1:<port>/mcp
  --header "x-api-key: <key>"`, so the key sits in Claude Code's own user config. The name is `hq`,
  or `hq-<hub>` when `hq` is taken by something else. An entry counts as ours only when its URL
  is the forward's address. Without `claude` on `PATH` this part is skipped.
- **Claude Desktop** gets `mcpServers.hq` in `claude_desktop_config.json`, set to this computer's
  absolute `hq` path with `mcp-serve --remote <hub>`. That entry holds no key: `hq` reads it from
  its own database. The file gets a dated backup (`.bak-YYYY-MM-DD`, the first change of the day),
  is replaced whole, and is left alone if it is not valid JSON. Without Claude Desktop's folder this
  part is skipped.

Revoking the hub (`hq bridge revoke`, or Revoke in the desktop app) removes these entries first. A
revoke from the library alone, or from the phone's helper, does not know where Claude is and leaves
them; `hq claude remove` takes them out while the setup row exists.

The same hub rows also make the hub a remote MCP server for HQ itself (the tools `<hub>_discover`
and `<hub>_call`, only on a live user turn). One resolver builds that list for every caller, and a
`remote_mcp` entry in `config.yaml` with the same name wins.

## Services and forwards by hand

Once two machines are paired, each can offer the other named services and reach the other's through
local ports. Pairing does this for a hub and a machine (above); the same pieces can also be set by
hand, for other pairs or other services. Configure them in `~/.hq/config.yaml`; `hq bridge serve`
(and the phone app's `hq bridge run`) reads it at start.

```yaml
# On the phone: offer the web app to paired devices, and reach the laptop's host gate.
bridge:
  web_listen: 127.0.0.1:5690        # the paired-devices door of the web server (below)
  services:
    web: 127.0.0.1:5690
  forwards:
    - peer: <laptop id from `hq bridge peers`>
      service: gate
      listen: 127.0.0.1:47490       # the phone's host entry for the laptop points here

# On the laptop: offer the host gate, and reach the phone's web app.
bridge:
  services:
    gate: 127.0.0.1:7747            # agent_host.gate_listen
  forwards:
    - peer: <phone id>
      service: web
      listen: 127.0.0.1:47480       # open http://127.0.0.1:47480 here
```

A service name is lowercase letters, digits and `-`. A service leads to a loopback port with a fixed
number or an absolute socket path, chosen by the machine that offers it: the other side only names
it and never chooses an address. A service may not lead to the web server's own port; give paired
devices the web app through `bridge.web_listen`, a second listener the web server opens only when
the web token is set, which serves the web app, `/api`, `/ws` and `/mcp` with an API key or a launched agent's session token, and never
the claude.ai connector's OAuth paths. A forward listens on loopback only. The services use their
own protocol name (`hq/svc/1`), offered only by a machine that has services, and admitted only from
paired peers; revoking a peer closes its open streams.

The web app keeps its sign-in per address, so open it at the forward's address and sign in there
once: on the phone, Settings, Devices, **Sign in another device**, then type the code into the
**Use code** box on the laptop. The desktop app's **Open HQ** window is one more address and signs in
the same way ([DESKTOP.md](DESKTOP.md#opening-your-phones-hq-for-the-first-time)).
List that origin in the phone's `web_allowed_origins` if the browser shows it as refused.

## What listens, and when

| Listener | When |
|---|---|
| Long-term endpoint (one UDP socket) | Between `Bridge::start` with a handler and `shutdown`. `hq bridge serve` runs it until interrupted |
| Throwaway endpoint (one UDP socket per ticket) | From `open_ticket` until the ticket is used, expired or burned. `hq bridge pair` runs it until then |
| The edge protocol on the long-term endpoint | Whenever a serving bridge has an edge door (the default); the gate admits only the registered edge. `--edge-door off` removes it |
| Connection to the connector door | One per stream the edge opens, made by the bridge, to the door's socket (or opted-in loopback port) only |
| The service protocol on the long-term endpoint | Whenever a serving bridge has `bridge.services`, or offers `gate` or `web` for setup; the gate admits only paired peers |
| The setup protocol on the long-term endpoint | On a hub, whenever its bridge runs (`hq bridge run`, or `--as hub`); the gate admits only paired peers. A machine never accepts it |
| One TCP port per forward, on loopback | While `hq bridge serve` or `hq bridge run` runs with `bridge.forwards`, and one per setup row |
| The web server's paired-devices door, on loopback | While the web server runs with `bridge.web_listen` and the web token set |

`hq bridge run` is both: the long-term endpoint for as long as it runs, and one throwaway endpoint while
a ticket is open. The Android app starts it only after the owner turned paired devices on.

Without a handler, `Bridge::start` binds a socket to dial out but accepts no protocol. The commands
`join`, `ping`, `peers` and `revoke` never accept a connection.

## Tests

```bash
cargo test -p hq-db bridge
cargo test -p hq-bridge                       # unit tests and the loopback end-to-end tests
cargo test -p hq-bridge --test setup          # setup after pairing, two machines on loopback
cargo clippy -p hq-cli --all-targets -- -D warnings
cargo test -p hq-cli --bin hq -- bridge_commands bridge_control
```

`bridge_control` drives two machines through the JSON-line control channel the phone app uses (pairing with
typed digits, wrong digits, ping, revoke, probe and redial). The Android side has JVM unit tests
(`cd apps/android && ./gradlew :app:testDebugUnitTest`).

The end-to-end tests use real iroh endpoints on loopback with no relay, the real migrated schema,
and `::1` as a second source address. They need no network. The spike's tests carry over: its
file-mode and symlink tests of the paired text file became the database tests (a failing database
closes the gate) and the key-file mode test, and its single global attempt budget became the
per-source and backstop tests.
