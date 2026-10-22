# HQ Mobile

The `mobile` profile is the edition of HQ for an instance that lives on a phone. It is Full HQ for
a machine with no shell sandbox, no `git` and no way to replace its own binary, plus a data saver
for people who pay for every megabyte. An Android build runs as this profile by default.

## What each edition does

| | Full | Lite | Mobile |
|---|---|---|---|
| Web app, tasks, vault, search, import | yes | yes | yes |
| Chat with a model, sub-agents | yes | no | yes |
| Telegram and Discord relays (they only dial out) | yes | no | yes |
| Drives coding agents on remote hosts (a laptop, a server) | yes | no | yes |
| Agent shell inside an OS sandbox | yes | no | no (a phone has no sandbox backend, so the shell is refused) |
| Git tools, the binary self-updater | yes | no | no |
| Starts only with the egress it lists | no | yes | no |
| Embeds note text for semantic search | yes | no | only with the data saver off |

Lite stays the edition for a machine you do not control. Mobile does not make Lite's promise of
no outbound traffic: a phone instance is allowed to talk to its model provider and its relays.

## Choosing it

On an Android build a config left at the default (`profile: full`) runs as Mobile, and a config
that asks for `profile: lite` keeps Lite. Elsewhere, ask for it:

```yaml
profile: mobile
mobile:
  data_saver: true   # the default
```

Environment: `HQ_PROFILE=mobile`, `HQ_MOBILE__DATA_SAVER=false`. Both are read when HQ starts. The
running daemon does not change edition on a config reload, as with Lite, and it refuses a change of
`mobile.data_saver` the same way (the embedding worker and the task list are fixed at start, so a
half-applied switch would keep sending note text after you turned it off). Restart to change either.
Full cannot be chosen on an Android build: a config left at `profile: full` runs as Mobile.

## The data saver

With `data_saver: true`:

- No note text is sent to an embedding provider. The keyword (full-text) index still follows every
  edit, so search keeps working.
- The scheduler skips `memory-consolidation`, `inbox-triage` and `email-poll`, the timers that
  call a model or a mail API without being asked.

Turn it off (`data_saver: false`) on Wi-Fi or an unmetered plan to get semantic search and those
timers back. A phone app that knows the connection type can set `HQ_MOBILE__DATA_SAVER` before it
starts HQ.

## What Mobile removes from Full

- Tool categories `git` and `self_update`, in the MCP registry and in the chat agent's own tool
  set alike, so no transport and no chat turn offers them (`MOBILE_REMOVED_TOOL_CATEGORIES` and
  `Profile::drops_tool_category` in `crates/hq-core/src/config/profile.rs`, used by
  `crates/hq-mcp/src/registry.rs` and `crates/hq-agent/src/builder/tools.rs`).
- The scheduled task `disk-watchdog`, which watches the disk and package caches of a development
  machine (`crates/hq-cli/src/commands/start/daemon/mod.rs`).
- The notice about a missing shell sandbox at start-up: there is no sandbox backend on a phone, so
  the agent's shell is refused (`governance.bash.sandbox` stays at its default, `required`) and
  nobody needs telling at every start.

The coding-agent tools, the session supervisor and the host event loops stay: they are how a phone
instance runs sessions on a host somewhere else. A built-in host on the phone itself is not
something to run; point `agent_host.default_host` at a configured remote host.

## Building for Android

`.github/workflows/android.yml` builds `hq-cli` for `aarch64-linux-android` with `--profile
release-small` and a pinned NDK, and fails when any LOAD segment is aligned below 16 KB
(`scripts/check-android-alignment.sh`), the page size Google Play requires of native code in apps
that target Android 15 or newer. OpenSSL is built from source for Android as it is for Linux
(`crates/hq-cli/Cargo.toml`).

To build by hand, point the C compiler, archiver and linker for the target at the NDK's
`aarch64-linux-android26-clang` and `llvm-ar`, then run
`cargo build -p hq-cli --profile release-small --target aarch64-linux-android --locked`.

The Android build also turns on the `tunnel` feature (`--features tunnel`), which adds `hq net
tunnel`: the phone as a userspace WireGuard peer with no system VPN slot. It is off in every other
build, nothing starts it on its own, and the app that runs it is separate work. The commands, the
files and the security notes are in `docs/PRIVATE_NETWORK.md` ("A phone without the WireGuard app")
and `docs/security/NETWORK.md`, section 9.

It turns on the `bridge` feature too (`--features tunnel,bridge`), which adds `hq bridge`, the QR
pairing of `docs/PAIRING.md`. The app drives it through `hq bridge run`, described next.

## The bridge on the phone

The phone can pair with another machine of its owner by QR code and then be reached by key, with no
account and no VPN (`docs/PAIRING.md`, `docs/security/PAIRING.md`). The app has three parts for it,
all new files (`apps/android/.../bridge/`, `ui/pairing/`, `ui/connect/`):

- **`hq bridge run`**, a child process of the hub service. It owns the iroh endpoint and the paired
  list. The app talks to it in JSON lines on its standard input and output: no socket, no port,
  so nothing but the app can reach it, and it ends when the app's end of the pipe closes.
- **The pairing screen** (`PairingActivity`): show a code, scan one with the camera, paste a link,
  type the six digits, list the paired devices, remove one. Removing closes that device's open
  connection at once.
- **The `hqpair://` link handler** (`PairLinkActivity`): a QR code read by the stock camera, or a link
  in a browser, opens the app on a confirmation screen. Nothing is dialed until the owner presses Pair,
  because anyone can send a link.

**Off by default.** The child binds a UDP socket and accepts paired devices, so it runs only after the
owner turned "Paired devices" on in the pairing screen (pairing a first device turns it on). The
state is remembered; the hub service starts the bridge again with the app after that. Turning it off
stops the child.

**The relay.** Pairing and reconnecting need a relay to find the direct path from behind a phone's
address translation. The default is n0's public relays (`--relay default`), which are for development and
testing, not production; a release uses a relay the household runs (the `hq edge` co-hosted relay).
Put `none`, `default` or a relay URL on one line in `files/bridge_relay` to change it. With a relay the
ticket carries no IP addresses (`--no-addrs`), which makes the QR code about half the size.

**The control protocol.** One JSON object per line. Commands, from the app: `open` (show a ticket;
`ttl_secs` optional), `cancel`, `digits` (the digits the person typed for a waiting device), `join` (a
link), `peers`, `revoke`, `network_change`, `ping`, `keep`, `keep_stop`, `quit`. Events, from `hq`:
`ready`, `ticket`, `request` (a device waits for digits: its name and id, never the digits),
`decision` (paired, declined or too late), `ticket_closed`, `digits` (what the phone shows while it
joins), `joined`, `join_failed`, `peers`, `revoked`, `ping`, `keep`, `error`, `bye`. The six digits
are compared inside `hq`: the app forwards what was typed and never learns the right answer, so a
"yes" button cannot be built on it. Unknown events are skipped, so a newer `hq` does not break an older
app. The code is `crates/hq-cli/src/commands/bridge_control.rs` (the loop works over any reader and
writer and is tested end to end with two machines) and `apps/android/.../bridge/BridgeWire.kt`.

**Why a child process needed a fix.** iroh reads Android's DNS servers through JNI (`ndk-context`),
and a child process has no JVM. The call panics, the release profiles abort on panic, so the first
endpoint bind ended `hq` with SIGABRT (measured on an Android 16 emulator; the spike's unwinding build only
printed the panic). `hq` now sets a null `ndk-context` on Android (`crates/hq-cli/src/android_runtime.rs`;
`netdev`, which also asks, checks it and skips what needs the JVM) and gives every bridge endpoint a
resolver that asks `getaddrinfo` first and iroh's public resolvers second (`hq_bridge::dns`), so the
JNI read is never reached. A new iroh endpoint anywhere in `hq` must be given that resolver.

**Network changes.** Android does not tell native code when the network changes. The app registers a
default-network callback (`NetworkWatcher`) and sends `network_change`, which calls
`Endpoint::network_change`. See `docs/plans/native-bridge/w4-measurements.md` for what that did and
did not fix, and for `keep`, the diagnostic that holds a connection to a paired device and traces its
round trips into `files/bridge.log`.

**Keeping the connection with the app in the background.** The bridge child lives inside the existing
foreground service, so it runs for as long as `hq` does. What a sleeping phone then does is in
`docs/plans/native-bridge/w4-measurements.md` and in the risk section of
`docs/plans/native-bridge-sprint.md`.

**Connect Claude** (`ConnectClaudeActivity`) shows the address to give claude.ai and an Arm button
(`POST /api/oauth/connector/arm`, the same window the web app opens). The address is what hq reports once
`mcp_connector` is on. Until the edge and the device's own TLS (workstreams W1 and W2) are merged, a
stand-in address in `files/connector_url` shows the screen with one; Arm stays off for a stand-in.

Diagnostics for `adb`, on the pairing screen's intent (it needs a permission only `adb` holds):
`--es keep <device id>` starts the `keep` trace, `--es keep ""` stops it, `--ez watch false` stops the app
from reporting network changes (for a before and after), `--es screen show|scan|paste` opens a screen.
`-Phq.appId=<id>` builds a copy that installs beside the real app for a phone test; it is never a release id.

## Not part of this edition

The app that runs this binary as a foreground service and shows the web app in a WebView, backup
and restore, and an updater for Android. They are separate work; the profile only makes the binary
behave correctly once something runs it. A way for other devices to reach a phone is the bridge above.
