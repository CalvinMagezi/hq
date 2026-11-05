# The desktop app

The first desktop in the Native Bridge sprint ([plan](plans/native-bridge-sprint.md), workstream
W5). A small Tauri 2 app that is the laptop's one app for HQ: it starts and stops the local
host, shows the QR code that pairs a phone or another computer, takes the six digits the other
device shows, and lists and revokes paired devices. While it is on, its bridge also offers this
laptop's coding-agent host to the phone and reaches the phone's web app. The web app
(`apps/hq-web`) is its main window, and when a phone is the hub it shows the phone's copy.

Threat model, listeners and the web view's limits: [security/DESKTOP.md](security/DESKTOP.md).
Pairing itself: [PAIRING.md](PAIRING.md).

## What it does

| Screen | What you can do |
|---|---|
| This computer | What HQ is doing on this computer, in one card: working, ready, slow to start, stopped (with what the log says), locked after too many starts, or waiting on a held port. Start, Open HQ, Stop (a red confirmation when helpers are working, a green one otherwise) and Restart for a slow start. A switch that starts HQ when HQ Hub opens, the `hq` program in use with **Change…** (a file panel, or a pasted path), and the log of the last run with Copy details. A locked host is cleared with **Allow helpers again**, which runs `hq host unlock`. |
| Devices | Switch the app on for paired devices, show a pairing code (QR and the `hqpair://` link), type the six digits the other device shows, list paired devices, revoke one. While the app is on it also lists what it shares with paired devices and the forwards it opened, with "running elsewhere" for a forward whose port another process already holds. |
| Helpers | The coding helpers on this computer, the ones waiting for you first, with a count on the tab. Each helper has its own page (see "Helpers"). A helper that needs you also raises a macOS notice, "AgentHQ · HQ Hub", once per blocked helper. |

The app is named AgentHQ in the Dock and Finder, and its control window is titled HQ Hub. The HQ window is
titled HQ, or "HQ · HQ on your phone" while a phone is the hub. HQ Hub opens `/`, `/chat`,
`/settings/devices`, `/helpers/<id>` and `/inbox/<id>` in that window and nothing else.
On the first start after the rename, Claude Desktop entries that point at the old `HQ.app` path are
rewritten to the new one (a dated `.bak` copy is kept) and the Helpers tab says which.
Windows and Linux are not built yet.

Nothing starts by itself. Opening the app starts no host, opens no network port and writes
nothing but its own folder (`~/.hq/desktop`). The host starts when you press Start (or when you turned on "Start the
host when this app opens"); the pairing endpoint opens when you press Switch on; the app's local
web address opens when you press Open HQ. The app never registers a login item, and it stops the
host it started when it quits (Cmd-Q, closing the window or a plain kill). A host it did not start (one you run in a terminal, or a service)
is shown as "Running elsewhere" and cannot be stopped from the app.

## The phone as the hub

When HQ lives on a paired phone, the laptop's config (`~/.hq/config.yaml`) says so with the
services and forwards from [PAIRING.md](PAIRING.md#services-and-forwards):

```yaml
bridge:
  services:
    gate: 127.0.0.1:7747            # this laptop's host gate, offered to the phone
  forwards:
    - peer: <phone id>
      service: web                  # the phone's web app
      listen: 127.0.0.1:47480
```

The app reads that config and changes three things:

- **The bridge serves and forwards.** Switching on starts the bridge with the configured services
  and opens the configured forwards, using the same code as `hq bridge serve`
  (`hq_bridge::services_from_config` and `start_forwards`). A forward whose port is already taken
  (for example by a `hq bridge serve` on this machine) does not stop the bridge; the Devices
  screen shows it as "running elsewhere". A service that leads to HQ's own web port is refused.
  Switching off closes the forwards the app opened.
- **The HQ window shows the phone.** With a forward to the `web` service, **Open HQ** proxies to
  that forward's loopback address instead of this machine's own web server. The proxy still adds no
  credential and the app puts no token in the window's address: sign in once in the window, which
  keeps the sign-in per address.
- **The Host button starts only the coding agents.** A config that offers a `gate` service and has
  a `web` forward counts as serving a hub, so the button runs `hq host serve` instead of
  `hq start all` (HQ itself is on the phone). The app tells the host is up by connecting to its
  control socket (`host.sock` in the host directory, `HQ_HOST_DIR` or `~/.hq/run/host`) rather
  than asking `/health`, which `hq host serve` does not have. Without a hub the button runs
  `hq start all` as before.

## Opening your phone's HQ for the first time

Once this laptop is paired with the phone, **Open HQ** shows the phone's HQ: your vault, chats, tasks
and inbox, the same as on the phone. The first time, the window asks you to sign in, because the app
never copies a credential into it. Sign in once with a code from the phone:

1. On the phone, open HQ, then **Settings**, **Devices**, and press **Sign in another device**.
2. In the laptop's window, type the code it shows into the **Use code** box (or paste the login link
   into the box under it) and press the button next to it.

The window stays signed in after that. A code works once and expires within minutes; the phone's
Devices list shows the laptop afterwards, and signing it out there takes its access away. Without the
app, the same steps work in a browser at the forward's address, which keeps its own sign-in:
`hq bridge serve` prints it as `Forwarding 127.0.0.1:<port> to web on <phone id>`.

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

## Sharing this computer's model

The Devices screen has a **Share this computer's model** switch, off by default. On, the app's bridge
offers this Mac's Ollama (`127.0.0.1:11434`, or `bridge.share_model.to` in the config) as the `llm`
service to the devices pairing set up as this Mac's hubs, and to no other paired device. Off, the
service is refused as if it did not exist and the streams a hub has open to it end at once, with no
restart of the bridge, so the phone's web app and the gate stay connected.

Next to the switch the app lists Ollama's models (marking those without tool calls, which HQ's tools
need) and warns when the chosen model's context is below 32,768 tokens: HQ's chat prompt is about
26,000 tokens and Ollama cuts a longer prompt without an error. The context is known only once the
model is loaded; until then the app says what to check. The model list is read when the tab opens and
on **Check again**, not on the three-second refresh, because asking Ollama about every model takes a
moment. On a Mac whose bridge is `hq bridge serve` (a launchd service) rather than the app, the same
service comes from `bridge.share_model` in `~/.hq/config.yaml` ([PAIRING.md](PAIRING.md)).
Either way the hub is told: turning the switch on or off, or picking another model, reaches each hub
within a few seconds, and the phone then uses the model with no setting of its own.


## Helpers

The Helpers tab lists the nine coding helpers, the two general helpers and Claude Desktop, with each one's version and
what it needs from you. Nothing runs until you press **Check this computer** once. A check reads the version from the
program's package file when there is one, and otherwise runs the program's version line once, closed off, with no
network and a two-second limit. Two checks run at a time and each answer is remembered by the program's path, size and
modified time; **Check all again** forgets them. Where a platform has no sandbox the version stays unknown and nothing is
run. A program in a folder others can write to is shown as "Program not found" and is never run.

A helper's page has:

- **Connected to HQ**: Connect and Remove the connection write what `hq mcp install` and `hq mcp remove` write for that
  helper. Remove offers **Undo** for ten minutes, restoring the files byte for byte if nothing else changed them. For a
  paired hub, Claude Code and Claude Desktop also list each hub (what `hq claude add` writes).
- **Try it**: starts the helper through HQ's host in one fixed folder (`~/.hq/desktop/try-it`, emptied before and after),
  sends it no prompt, and stops it. A question on its screen is never answered for it, except the folder-trust question
  for Antigravity. A general helper asks you first, and Try it is off while helpers are stopped.
- **Sign in**: keeps the helper open for up to ten minutes. The link is read from its screen inside the app and never
  reaches the page. **Copy sign-in link** copies it; **Open the sign-in page** opens it only when it is `https` and on
  that helper's own sign-in site.
- The install line for helpers that have one. HQ Hub never runs an installer.

Claude Desktop is told to start `hq`, so the app needs that program's absolute path: the one the Start button uses (see
"Finding `hq`" below). Removing a paired device removes its entries in Claude first, and says so if one could not be
taken out.

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
| `hq_path` | unset | The `hq` program to run. Unset: see "Finding `hq`". **Change…** sets it |
| `look_for_helper_updates`, `try_it_dir`, `usual_start_secs`, `recent_start_secs`, `connection_sheet_day`, `bundle_rename_handled` | see below | Written by the app, kept when the window saves the rest |
| `share_model` | `false` | Lend this Mac's model to its hubs ("Share this computer's model" on the Devices screen) |
| `shared_model` | unset | The model hubs are told to use. Unset: the first one Ollama lists |

The app-written keys hold what it learned: the usual start time (the median of the last five starts) and whether the rename was handled. Port, relay, name and `hq_path` changes apply the next time the app opens. The host's own output goes to
`~/.hq/desktop/host.log` (owner-only). The app reads the same `~/.hq/config.yaml` as `hq`.

## Finding `hq`

The app runs HQ's own `hq` program and looks for it when it opens, in this order:

1. The path in Settings (`hq_path`), or the one picked with Change… If that is wrong, the app says so and does not look elsewhere.
2. Next to the app, `~/bin`, `/usr/local/bin`, then each directory on the app's `PATH`.
3. Only when none of those has it: the `PATH` your login shell sets up (`$SHELL -ilc`, given five
   seconds). An app opened from the Dock gets only the system's short `PATH`, so an `hq` that a
   shell profile puts on your `PATH` is otherwise invisible to it.

A candidate counts only when `hq --version` answers `hq <version>` within five seconds: some systems
have an unrelated program called `hq`. When nothing is found, the This computer tab says so as soon as
the app opens and lists where it looked.

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

The result is `target/release/bundle/macos/AgentHQ.app` under the Cargo target directory (set
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

## Signing and the DMG

The **Desktop release** workflow (`.github/workflows/desktop-release.yml`, run by hand or by a
`desktop-v*` tag) makes `AgentHQ.dmg` in two jobs, split the way `release.yml` splits its own:

1. **Build** compiles `AgentHQ.app` unsigned. It runs the repository's build code (and every
   dependency's build script), so it has no environment and no secret.
2. **Sign** downloads that bundle and runs only `scripts/desktop/sign-macos.sh` on it, in the
   `desktop-release` environment that holds the Apple secrets. The certificate and the notary key
   never share a job with build code. For the same reason the app is not signed by `tauri build`,
   which would need the certificate in the build job.

What the script does depends on which secrets the environment has:

| Secrets | Result |
|---|---|
| None | Signed ad hoc. Opens on the machine that has it; Gatekeeper refuses it once downloaded. For testing only |
| `APPLE_CERTIFICATE`, `APPLE_CERTIFICATE_PASSWORD` (and optionally `APPLE_SIGNING_IDENTITY`) | Developer ID signature with the hardened runtime and a secure timestamp, on the app and on the DMG. Not notarized |
| The above plus `APPLE_API_ISSUER`, `APPLE_API_KEY` (key id) and `APPLE_API_KEY_P8` (the key's contents) | The app is notarized and stapled, then the DMG is signed, notarized and stapled, and `spctl` checks both |

`APPLE_CERTIFICATE` is the base64 of a Developer ID Application `.p12`. The script imports it into
a throwaway keychain that it deletes on exit, and restores the keychain search list. It needs no
entitlements: the app is not sandboxed and uses none of the JIT or library-validation exceptions.
If the sandbox is ever turned on, it needs only `com.apple.security.network.client` and `.server`.

To try the script locally on a built app: `bash scripts/desktop/sign-macos.sh
<target>/release/bundle/macos/AgentHQ.app /tmp/AgentHQ.dmg` (ad hoc unless the variables above are set).

The bundle identifier is `online.agenthq.desktop`. WebKit keeps the HQ window's sign-in per
identifier, so a build with the earlier placeholder (`dev.hq-oss.desktop`) asks you to sign in
once more.

## What a release still needs

1. A real icon (`src-tauri/icons` holds a placeholder).
2. The `hq` program inside the app (`bundle.externalBin`, one per target triple), so the app works
   with nothing installed. Today it finds an installed `hq`. The sign script already signs every
   extra program in `Contents/MacOS` before the bundle.
3. The Apple secrets in the `desktop-release` environment. Until they exist, notarization has not
   been run end to end.
4. Where the DMG is published. The workflow keeps it as a run artifact for now.
5. A decision on updates. The host updates itself from signed releases (`UPDATE_SYSTEM.md`); the
   app does not, and a Tauri updater would be a second update path to secure. Until then a new
   version is a new download.
6. Universal binary (`--target universal-apple-darwin`) if Intel Macs matter. The workflow builds
   for Apple silicon only.
7. The relay decision from the sprint plan: n0's public relays are for development and testing,
   so the default relay must be replaced before anyone outside the household installs it.
