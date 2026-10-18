# Spike B: macOS, Tauri shell and sandbox checks

Date: 2026-10-18. Machine: macOS 26.6 on Apple Silicon, Rust 1.98, Xcode command line tools, bun 1.3. Nothing here touches a running host: every check uses a copy of the `hq` binary, its own run directory and its own web token. Anyone with a Mac can repeat it with the commands below.

## What was checked and what came out

| Check | Result |
|---|---|
| Tauri v2 window loads a local `hq web` | Works. Sign-in from a `#token=` link, chat socket opens, service worker registers, no CSP violations |
| Relaunch | The token, localStorage and the service worker survive an app restart |
| Loopback reverse proxy in front of `hq web` | Works, WebSocket included, with the Host header kept or rewritten. No server change was needed |
| Web Push in the WebView | `PushManager` is absent and `Notification` exists but is not wired to push, so notifications have to come from the app |
| Tauri IPC from the remote page | The `__TAURI_INTERNALS__` object is visible, but a call is refused ("not allowed") and the page CSP also blocks the `ipc:` scheme. Leave it that way: the web app needs no native access |
| Origin guard | A foreign origin gets 403. Any loopback origin, on any port, is accepted (`origin.rs`), so on a laptop another local web page can reach a loopback proxy. The web token is the real guard there |
| Sandboxed pane (`sandbox-exec`) with a pty | Runs. Same result with an ad-hoc signature and with an ad-hoc signature plus the hardened runtime flag, and no entitlements were needed |
| Sandbox policy | `~/.hq` and `~/Documents` are refused by the sandbox profile itself, before macOS privacy rules are involved |
| Egress allowlist | A host that is not listed fails (`curl` prints `000`) and the decision is readable with the `agent.egress` socket method; a listed host works |
| Run directory under a writable temp dir | The host refuses to sandbox an agent (`sandbox_unavailable`), which is a good guard but trips up anyone who puts the run dir in `/tmp` |
| Unix socket path | macOS limits it to 104 characters, so long scratch paths break `host.sock` |

Folder-privacy (macOS TCC) checks, run with a person at the screen to answer the prompts. Each host ran as its own launchd job so the prompt names the right program. The decisions and the program they were credited to were read from the unified log (`tcclog.py`).

| Check | Result |
|---|---|
| Ad-hoc signed `hq`, first read of `~/Documents/HQ` from a sandboxed pane | macOS credits the access to the `hq` binary itself (read from the log). The pane blocks until it is answered (18.7 s here, however long the person takes) |
| Same binary after an update, re-signed ad-hoc | The code hash changes and the access is decided again (34 s blocked, so a prompt waited for an answer). An agent started with nobody at the screen hangs |
| Binary signed with a self-made certificate, then updated and re-signed with the same certificate | The designated requirement is unchanged (`identifier` plus certificate) and the updated binary ran in 0.1 s with no prompt |
| Host started by a `.app` | macOS credits the access to the app (its bundle id), not to the `hq` child |
| Host as its own LaunchAgent, app killed | Host (same pid) and a running pane survived |
| Host as a child of the app, app killed with SIGTERM | The orphaned host and its pane also survived. Survival does not separate the two shapes; attribution and lifecycle control do |

Not covered here, and why: a Developer ID signature and notarization need a paid account; the Gatekeeper cost for a downloaded, un-notarized app was not measured; whether the very first prompt appeared for the self-signed binary and for the app was not confirmed by the person at the screen (the pane returned in about two seconds).

## Repeat it

Build the shell once (about a minute):

```
cd docs/plans/native-bridge/evidence/spike-b/shell
CARGO_TARGET_DIR=$TMPDIR/spike-shell-target cargo build
```

Start a throwaway web server on a throwaway vault. The token is a random test value, nothing else:

```
WORK=$(mktemp -d)
printf 'ws_port: 5791\nweb_bind: "127.0.0.1"\n' > $WORK/config.yaml
TOKEN=$(openssl rand -hex 24)
HQ_CONFIG_PATH=$WORK/config.yaml HQ_WEB_AUTH_TOKEN=$TOKEN \
  HQ_WEB_STATIC_DIR=<repo>/apps/hq-web/dist/client \
  hq web --vault $WORK/vault --no-open --detach
```

Open the shell on it. The result line is printed to the terminal after about six seconds. Run it a second time without `#token=...` to see persistence:

```
SPIKE_URL="http://127.0.0.1:5791/#token=$TOKEN" $TMPDIR/spike-shell-target/debug/spike-shell
```

Put the proxy in between (`keep` forwards the Host header, `rewrite` replaces it):

```
bun proxy.ts 5792 5791 keep
SPIKE_URL="http://127.0.0.1:5792/#token=$TOKEN" $TMPDIR/spike-shell-target/debug/spike-shell
```

Sandbox and egress, on a host that is not your real one. The run directory must be short and must not sit under a writable temp directory:

```
cp "$(command -v hq)" $WORK/hq-copy && codesign --force -s - $WORK/hq-copy
mkdir -m 700 -p ~/.hq-spike-run
$WORK/hq-copy host serve --dir ~/.hq-spike-run &
python3 drive.py ~/.hq-spike-run $WORK
$WORK/hq-copy host stop --dir ~/.hq-spike-run
```

For the hardened-runtime variant, sign the copy with `codesign --force --options runtime -s -` instead.

Folder-privacy checks (needs you at the screen; the prompts appear on your display):

```
export WORK=$(mktemp -d); cp "$(command -v hq)" $WORK/hq-a && codesign --force -s - $WORK/hq-a
printf 'ws_port: 5791\n' > $WORK/config.yaml
. tcc/lib.sh; start_host a $WORK/hq-a
python3 tcc/probe.py ~/.hqs-spike/a ~/Documents/<a-folder-you-can-read>   # answer the prompt
python3 tcc/tcclog.py 120 hq-a                                          # who was credited, allowed or denied
stop_host a
```

Change the binary and re-sign it (`install_name_tool -add_rpath /usr/lib/x hq-a; codesign --force -s - hq-a`) to see the prompt return. Repeat with a binary signed by a certificate you made yourself to see it stay quiet. Use `/usr/bin/log`, not the shell builtin.

Stop the web server with `kill` on the pid it printed, and delete `$WORK` and `~/.hq-spike-run` afterwards.
