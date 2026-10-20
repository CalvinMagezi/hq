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

## Not part of this edition

The app that runs this binary as a foreground service and shows the web app in a WebView, backup
and restore, a way for other devices to reach a phone, and an updater for Android. They are
separate work; the profile only makes the binary behave correctly once something runs it.
