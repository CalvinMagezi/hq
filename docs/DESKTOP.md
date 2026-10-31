# The desktop app (macOS)

The first desktop in the Native Bridge sprint ([plan](plans/native-bridge-sprint.md), workstream
W5). A small Tauri 2 app that starts and stops the local HQ host, shows the QR code that pairs a
phone or another computer, takes the six digits the other device shows, lists and revokes paired
devices, and has a Connect Claude screen. The web app (`apps/hq-web`) is its main window.

Threat model, listeners and the web view's limits: [security/DESKTOP.md](security/DESKTOP.md).
Pairing itself: [PAIRING.md](PAIRING.md).

## What it does

| Screen | What you can do |
|---|---|
| Host | Start and stop the `hq` host (`hq start all`) and see its state. Open HQ, which shows the web app in a second window. An optional setting starts the host when the app opens. |
| Devices | Switch the app on for paired devices, show a pairing code (QR and the `hqpair://` link), type the six digits the other device shows, list paired devices, revoke one. |
| Connect Claude | Show whether the claude.ai connector is on, the address to add in claude.ai, and open or close the window that lets one new connection through. |

Nothing starts by itself. Opening the app starts no host, opens no network port and writes
nothing but its own folder (`~/.hq/desktop`). The host starts when you press Start (or when you turned on "Start the
host when this app opens"); the pairing endpoint opens when you press Switch on; the app's local
web address opens when you press Open HQ. The app never registers a login item, and it stops the
host it started when it quits (Cmd-Q, closing the window or a plain kill). A host it did not start (one you run in a terminal, or a service)
is shown as "Running elsewhere" and cannot be stopped from the app.

## Pairing a device

1. Devices, **Switch on**, then **Show pairing code**.
2. On the other device, scan the QR code (the camera opens the app, because the code is a link),
   or run `hq bridge join` and paste the link. It shows six digits.
3. Type those digits in the app and press **Pair**. The device is paired only if they match. A
   typo gets two more tries; the third wrong try declines the device. There is no "yes" button
   and the app never shows the expected digits.
4. The device appears under Paired devices. **Revoke** (press twice) takes it off the list and
   closes every connection it has open, at once.

A code works once and expires after ten minutes. Showing a new one replaces the old one. Switching
the app on opens one UDP port (47475 by default) on every interface, and each code opens a second,
temporary one. Only devices you paired get past the handshake. Do not run `hq bridge serve` while
the app is switched on: both would use the same key.

Devices on the same network pair directly. The relay setting defaults to `none`; a phone on mobile
data needs a relay (`relay` in settings: `default` for n0's public relays, which are for development
and testing only, or a relay URL you run).

## Settings

`~/.hq/desktop/settings.json`, owner-only, created when you first save a setting. Unknown keys are
refused.

| Key | Default | Meaning |
|---|---|---|
| `start_host_on_launch` | `false` | Start the host when the app opens |
| `proxy_port` | `47474` | The app's local web address, `http://127.0.0.1:<port>`. Fixed, because HQ's web app keeps its sign-in per address |
| `bridge_port` | `47475` | UDP port of the pairing endpoint. Fixed, because a paired device stores the address it paired at |
| `relay` | `none` | `none`, `default` or an `https://` relay URL |
| `device_name` | the host name | What this Mac is called to the devices it pairs with |
| `hq_path` | unset | The `hq` program to run. Unset: next to the app, `~/bin`, `/usr/local/bin`, then `PATH` |

Port, relay, name and `hq_path` changes apply the next time the app opens. The host's own output goes to
`~/.hq/desktop/host.log` (owner-only). The app reads the same `~/.hq/config.yaml` as `hq`.

## Connect Claude

Until `hq edge` and the device-side TLS (workstreams W1 and W2) are merged, this machine has no
address Claude's servers can reach, and the screen says so instead of inventing one. If you
already run a public address, set `mcp_connector.public_url` (see
[CLAUDE_CONNECTOR.md](CLAUDE_CONNECTOR.md)) and the screen shows the connector address to paste
into claude.ai, and **Allow a new connection** opens the ten-minute window.

The consent page (`/connect`) and its API are served on the app's own address, so no tailnet name
is needed to open them in a browser on this Mac. One piece is still open and belongs to
workstream W6: HQ tells Claude to send your browser to `mcp_connector.authorize_url`. That may now
be the app's own loopback address (`http://127.0.0.1:<port>`, the only plain-http origin HQ
accepts), but whether claude.ai follows such an `authorization_endpoint` is still a test to run
against a real account (`docs/plans/native-bridge/w6-claude-run.md`). The screen says when the
advertised origin is not the app's own. To approve on the app's address, get a sign-in link from
the screen and open it in the browser first.

## Building it

The app is its own Cargo workspace (`apps/desktop/src-tauri`), not a member of the root one:
the web view needs system libraries the Linux CI job does not have, and `cargo test --workspace`
must stay fast. Everything that can be tested without a window is in the `hq-desktop` crate
(`crates/hq-desktop`, in the root workspace), whose tests run with the rest.

```bash
cd apps/desktop
bun install --frozen-lockfile
bun run typecheck
bun run build            # tauri build --bundles app; unsigned
```

The result is `target/release/bundle/macos/HQ.app` under the Cargo target directory (set
`CARGO_TARGET_DIR` to share one with the main workspace). The Tauri CLI is the pinned npm
package, so there is nothing to `cargo install`. Build with `CARGO_PROFILE_DEV_DEBUG=0
CARGO_INCREMENTAL=0` on a tight disk, and delete the target directory when done.

To try it without bundling: `cd apps/desktop && bun run build:ui && cd src-tauri && cargo run`.

Tests:

```bash
cargo test -p hq-desktop                         # logic, proxy, pairing end to end, shell config guards
cd apps/desktop && bun run typecheck             # the control window's script
```

The pairing test pairs a joiner built the way `hq bridge join` is (its own bridge, the link, the
digits shown through a callback) with the app's host, connects by key, then revokes and checks
the open connection closes. One more test drives the real `hq bridge join` program; it needs a
built `hq` binary, so it is opt-in:

```bash
cargo build -p hq-cli
HQ_BRIDGE_BIN=target/debug/hq cargo test -p hq-desktop --test cli_join -- --ignored
```

## What a release needs

This build is unsigned and has no updater. Gatekeeper refuses an unsigned app downloaded from the
internet; on the machine that built it, it opens. A release needs:

1. A bundle identifier on a domain the project owns (it is `dev.hq-oss.desktop` now), and a real
   icon (`src-tauri/icons` holds a placeholder).
2. The `hq` program inside the app (`bundle.externalBin`, one per target triple), so the app works
   with nothing installed. Today it finds an installed `hq`. Sign it with the app.
3. A Developer ID Application certificate, the hardened runtime, and entitlements for what the
   app does: it needs no sandbox exemption beyond the network client/server entitlements
   (`com.apple.security.network.client` and `.server`) if the sandbox is ever turned on, and
   none of the JIT or library-validation exceptions.
4. Notarization with `notarytool` and stapling the ticket, in CI with the credentials as secrets
   (release workflows run on the dedicated runner, not the hosted one).
5. A decision on updates. The host updates itself from signed releases (`UPDATE_SYSTEM.md`); the
   app does not, and a Tauri updater would be a second update path to secure. Until then a new
   version is a new download.
6. Universal binary (`--target universal-apple-darwin`) if Intel Macs matter.
7. The relay decision from the sprint plan: n0's public relays are for development and testing,
   so the default relay must be replaced before anyone outside the household installs it.
