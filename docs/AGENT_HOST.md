# Agent host

`hq host` is HQ's built-in host for long-lived agents: coding agents, and the
general-purpose agents (Hermes, OpenClaw) that HQ uses for monitoring,
research and review but never for code (see "General-purpose agents" in
[AGENT_SESSIONS.md](AGENT_SESSIONS.md)). It runs each agent
in a pseudo-terminal, keeps an emulated screen and scrollback you can read back,
and accepts typed input. It is where every agent session HQ starts runs,
on this machine or on a machine you paired (see [JOIN_A_MACHINE.md](JOIN_A_MACHINE.md); on Windows the host runs inside WSL2, see [WINDOWS.md](WINDOWS.md); HQ Lite on native Windows has no host).

## macOS: a stable signature

macOS ties its folder-access approvals (Documents, Desktop, Downloads) to a program's code
signature, and an ad-hoc signature changes with every build, so every update would ask again,
and an agent started with no one at the screen hangs waiting. `hq host install` therefore
makes a certificate for this machine (`HQ Local Code Signing`, in
`~/.hq/hq-signing.keychain-db`, an empty-password keychain holding nothing else) and signs
`hq` with it. Approve the prompt macOS shows the first time; it stays approved.

After that, `hq update` signs each new build with the same certificate before it replaces the
old one, and so do a source rebuild through `scripts/install-hq.sh` and a self-update. They do
this only on a machine that has already run `hq host install`, so an update never creates a
keychain for someone who did not ask for one. `hq host sign <path>` does the same for a binary
you put in place yourself. If signing fails, the update and the host still go ahead with a
warning, and macOS asks again. `hq doctor` shows whether the running `hq` has the stable
signature.

The update that first brings this in is applied by the older `hq`, which does not sign, so it
lands ad hoc. Run `hq host sign "$(command -v hq)"` once afterwards (it only replaces the file,
nothing running is touched); macOS asks once more at the next host restart, because this is a
new identity to it. Later updates sign by themselves.

The certificate's key sits in that keychain and `codesign` uses it without a prompt. A program
running as you can therefore sign its own binary with it, and macOS would treat that binary as
HQ for folder approvals. HQ's host-run agents cannot reach it (the host sandbox hides `~/.hq`),
and the keychain files are masked from agent file tools and from the chat shell's sandbox (unless
`governance.bash.sandbox` is `off`). A
program that may run `codesign` can still ask macOS to use the key without reading the file, so
treat it like the other keys in your home folder. A Developer ID signature held in CI would
close this and replace the local certificate; the project has no Apple developer account yet.

## Run it

```
hq host serve            # run the host in this terminal (Ctrl-C stops it)
hq host status           # protocol and host version, pid, number of agents, lockdown
hq host stop             # ask a running host to exit
hq host lockdown "why"   # emergency stop: stop every agent, start none (see Lockdown)
hq host unlock           # lift a lockdown; only works on this machine
hq host sign <path>     # macOS: give an hq binary the stable local signature (see above)
```

All of them take `--dir <path>` (default `~/.hq/run/host`). The directory is
created with mode 0700 and holds:

- `host.sock`: the control socket, mode 0600.
- `operator.token`: a random secret, mode 0600, created on first start. A token
  file that is not a regular file owned by you with mode 0600 is refused, and so
  is a `--dir` that is a symlink or belongs to someone else.
- `host.lock`: held while a host runs. A second `serve` on the same directory
  fails; a socket left behind by a host that died is replaced.

## Protocol

One JSON object per line over the socket, at most 1 MiB per line. Requests are
`{"id", "method", "params"}`, replies are `{"id", "result"}` or
`{"id", "error": {"code", "message"}}`.

The first request on a connection must be `hello` with
`{"protocol_version": 1, "token": "<operator token>"}`. Anything else before it
gets `unauthenticated`. A wrong token gets `unauthorized`, a different protocol
version gets `protocol_mismatch`.

| Method | Params | Result |
|---|---|---|
| `host.status` | none | `protocol_version`, `host_version`, `pid`, `agents`, `agents_working`, `agents_blocked`, `binary_stale`, `lockdown` (`reason`, `source`, `at`, or null) |
| `host.lockdown` | optional `reason` | `{"lockdown"}`; stops every agent and refuses every start until `hq host unlock` (see Lockdown). There is no unlock method. |
| `host.workspace` | none | `root` (the HQ folder, `~/Documents/HQ`, created if missing), `os`, `wsl`, `explorer_path` (the `\\wsl$\...` path on WSL2, else null) |
| `host.dirs` | optional `path` (the HQ folder when absent) | `path`, `parent`, `dirs` (`name`, `path`), `truncated`; only folders inside the HQ folder, symlinks out of it refused |
| `host.mkdir` | `name`, optional `parent` | `{"path"}`; one folder name (no slashes, colons, `~`, `$` or backticks), idempotent |
| `host.stop` | none | `{}` |
| `agent.spawn` | `name`, `argv`, `cwd`, optional `agent` (kind, for state detection), `resume_argv` (see Restarts), `env` (object), `rows`, `cols`, `scrollback_rows` | agent info |
| `agent.list` | none | `{"agents": [...]}` |
| `agent.get` | `name` | agent info |
| `agent.read` | `name`, optional `source` (`visible`, `recent`, `recent_unwrapped`, `styled`; default `recent_unwrapped`; `styled` is `recent` with color and style as ANSI escape sequences), optional `lines` (last N, 0 or absent for all) | `{"text", "truncated"}`; text is cut to its last 400 KiB, at a line start, when longer |
| `agent.send_text` | `name`, `text` | `{}` (raw bytes, as given) |
| `agent.paste` | `name`, `text` | `{}` (bracketed paste when the program enabled it) |
| `agent.prompt` | `name`, `text` | `{}` (paste, short pause, Enter) |
| `agent.send_keys` | `name`, `keys` (`enter`, `esc`, `tab`, arrows, `pageup`, `ctrl+c`, ...) | `{}` |
| `agent.resize` | `name`, `rows`, `cols` | `{}` |
| `agent.wait` | `name`, `until` (`exit`, `quiet` or `state`), optional `timeout_ms`; for `quiet` `quiet_ms`; for `state` `states` (list, required) and `stable_ms` (default 300) | `exit_code`, or agent info |
| `events.poll` | optional `after` (a `last_seq`), `timeout_ms` (default 25000, at most 60000) | `{events, last_seq, lost}`: waits for events after `after`; without `after` it returns the current `last_seq` at once. An event is `{seq, name, kind (spawned, state, exited, removed), state, rule}`. `lost` means events were dropped, so re-read `agent.list`. Numbers start from the clock, so they keep rising across host restarts. |
| `agent.report` | `event` (a hook event name), optional `session_id`, `notification_type`; `name` for the operator (a pane token implies its own) | `{}` |
| `agent.set_resume` | `name`, `argv` | `{}`; replaces the command a restart runs (the agent must have been spawned with `resume_argv`) |
| `agent.awaiting` | none | `{"agents": [{name, agent, cwd, env_keys}]}`: restored agents waiting for their environment |
| `agent.resume` | `name`, `env` (object, must cover every name in `env_keys`) | agent info; `missing_env` leaves it waiting |
| `agent.kill` | `name` | `{}` |
| `agent.remove` | `name` | `{}` |

Unknown fields in params are an `invalid_params` error. Rows and columns must be
1 to 1000, `scrollback_rows` is capped at 100000, and `cwd` must be an existing
absolute directory (otherwise `spawn_failed`; the host never falls back to the
home directory).

At most 64 connections are served at once (one more gets `too_many_connections`),
a connection that sends no `hello` within 5 seconds is closed, and a reply that
cannot be written within 10 seconds drops the connection. `host.stop` answers
first and then stops the host.

`agent.kill` and `agent.remove` stop the process and everything it started: the
process group gets SIGHUP, then SIGKILL after 2 seconds if the leader is still
running. `agent.wait` with `until: exit` returns only after the output the
process printed last has been read, so a read right after it sees the tail.

Agent names match `[a-z][a-z0-9_-]{0,31}` and are unique. Agent info has `name`,
`argv`, `agent`, `resumable`, `title`, `state`, `rule`, `cwd`, `pid`, `status` (`running` or `exited`), `exit_code`, `rows`,
`cols`, `bytes_seen`, `quiet_ms`, `age_ms`. Error codes include `agent_not_found`,
`name_taken`, `invalid_name`, `invalid_keys`, `spawn_failed`, `agent_exited`,
`timeout`, `invalid_params`, `unknown_method`, `bad_request`.

## Agent state

When a pane is spawned with an `agent` kind that has a rule file (`claude`,
`claude-code`, `codex` today), the host works out its state from the screen and
the terminal title: `idle`, `working`, `blocked` or `unknown`. Agent info carries
`state` and `rule` (the rule that decided; absent when nothing matched, which
means idle). A kind with no rule file has no `state`. `agent.wait` with
`until: "state"` returns once the state is one of `states` and has held for
`stable_ms`, so a one-frame flicker is not taken for a change.

Rule files are TOML (the format and its provenance are in `docs/provenance/herdr.md`): each rule names a screen
region and a gate of phrases or patterns, and the matching rule with the highest
priority decides. They are built into the binary (`crates/hq-host/src/detect/manifests/`);
there is no fetching from anywhere at run time. Screens captured from real
Claude Code and Codex sessions, each labelled with the state the agent was really
in, are in `crates/hq-host/tests/fixtures/` and are checked on every test run.

### Done, the change counter and events

Agent info also carries `done` (a turn finished and the agent has not worked
since) and `state_seq`, a number that goes up each time the
state changes. HQ alerts once per `state_seq` value, which is how a finished
turn or a blocked dialog is reported once and not once a minute. The host sends
an `events.poll` event for every change, whether an agent reported it or its
screen showed it, and HQ's daemon long-polls it so the supervisor runs within
seconds of a change; its once-a-minute sweep stays as the safety net.

### Agent-reported state

Claude Code can tell the host what it is doing. HQ launches it with
`--settings <run dir>/hooks/<name>.json`, a per-launch file (mode 0600), so
nothing is written to your own Claude configuration. The file runs
`hq host report` on `SessionStart`, `UserPromptSubmit`, `Stop` and
`Notification`; the command prints nothing and always succeeds, so a missing
host never disturbs the agent. The host then knows the agent's state as the agent
reports it, and the agent's own conversation id (`agent_session_id`, the id
`claude --resume` takes). Agent info shows `rule: "hook:<event>"` when a report
decided the state.

The screen still has the last word where it is more reliable: a dialog on screen
is `blocked`, a reported `blocked` ends when the dialog is gone (approving one
fires no hook), and a reported `working` that has printed nothing for 15 seconds
while the screen reads idle gives way to idle (an interrupted turn fires no
`Stop`).

Each pane gets its own token (`HQ_HOST_TOKEN`) and the socket directory
(`HQ_HOST_DIR`). That token can call `agent.report` for its own agent and
nothing else: any other method, or another agent's name, is `forbidden`. It
lives in memory only and stops working when the agent is removed. Anything the
agent runs can read it, so it can also misreport its own state, which the
agent could do by printing to its screen anyway.

### What the host does not protect against

Without a sandbox an agent runs as the same operating-system user as the host.
It can read `operator.token` in the host directory (`HQ_HOST_DIR` is in its
environment) and do everything the operator can, and it can read the MCP config
files under `mcp/`, which hold the HQ tokens of the other sessions. The pane
token and the scope check only keep an honest agent in its lane.

### The process sandbox

`agent.spawn` takes a `sandbox`; HQ sends one for every launch on a built-in
host (`agent_host.sandbox`, default mode `process`). In process mode the host wraps
the agent in `sandbox-exec` (macOS) or `bwrap` (Linux) and refuses to start it
when that is not possible. The agent:

- cannot read the run directory except `host.sock` and its own hook and MCP
  files, nor `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.kube` or `~/.hq`;
- can write only its project, `~/.claude`, caches and the temporary directories
  (plus `agent_host.sandbox.writable`);
- cannot see other processes' environments;
- can read under your home directory only what the policy lists (its own state,
  tool installs, shell and git configuration, caches, and `agent_host.sandbox.readable`),
  and cannot write `~/.claude.json`: the host records trust for the project
  itself, so the agent cannot plant an MCP server command there;
- can connect only to its own egress proxy, the agent socket (`agent.sock`,
  pane tokens only, with its own connection limits, so an agent cannot crowd the
  operator off `host.sock`) and any `agent_host.sandbox.loopback_ports` (below). The proxy allows
  `api.anthropic.com`, the HQ MCP endpoint and `agent_host.sandbox.allow_domains`,
  resolves names itself, and refuses private, loopback and link-local addresses
  unless the endpoint is the MCP one or a listed loopback port. A tunnel must open with a TLS hello naming
  the host it asked for. Every decision is logged
  (`agent.egress`); a denied host shows up there so you can add it.

Two opt-in settings widen this for one machine's own needs:

```yaml
agent_host:
  sandbox:
    # A local model server and the adb server on the agent's machine.
    loopback_ports: [11434, 5037]
    # Only Claude Code may read gh's configuration, and never write it.
    agent_readable:
      claude: ["/home/alice/.config/gh"]
```

`loopback_ports` lets an agent reach those ports on its own machine. A client
that sends loopback requests to the proxy (curl does) gets there through it as
plain HTTP, logged like any other request, on every platform. On macOS the sandbox also allows a
direct connection of any protocol to exactly those ports, which is what adb and
clients that ignore `HTTPS_PROXY` (Dart's `HttpClient`) need. Under bubblewrap
loopback inside the sandbox is the sandbox's own, so only the proxied HTTP path
works there. Every agent on the host gets these ports: a port that drives
something (adb reaches every attached phone and emulator) is a capability, so
list only what agents should have.

`agent_readable` maps an agent kind (`claude`, `codex`, `agy`, `hermes`, ...;
not a harness or profile name such as `claude-code`) to paths only it may read.
Naming one of the credential directories exactly (such as `~/.config/gh`) lifts
its hiding for that agent alone and keeps it read-only. `~/.hq` is never
revealed.
Paths use the agent machine's absolute paths; one that does not exist there is
skipped.

The host itself decides whether the sandbox is optional. `hq host serve` refuses
to start any agent that is not under the process sandbox, whoever asks over the
socket or through the gate; start it with `--allow-unsandboxed` to lift that, and
`agent_host.sandbox.mode: none` in HQ only matters then.

Agents nobody is watching are stopped: a session silent and not working for
`agent_host.idle_reap_hours` (default 24, 0 turns it off) is stopped by the host, even
if HQ is down, and is not restored at the next start. HQ marks it exited and
tells you, and the conversation can be resumed. `hq host status` lists every
agent with its state, sandbox mode, idle time and age. A busy agent is never
reaped.

Each agent also gets what it needs to be that agent: its own login and history directory
(`~/.gemini` for Antigravity, `~/.codex`, `~/.cursor`, `~/.copilot`, `~/.pi`, `~/.hermes`,
`~/.openclaw` and so on, and never another agent's) and the HTTPS hosts it
signs in and reaches its models through. Those defaults are verified for Claude Code and
Antigravity; for agents that talk to whichever provider you configure (opencode, kimi, qwen,
and the general-purpose agents hermes and openclaw) add the provider's host to
`agent_host.sandbox.allow_domains`. Only Claude Code is handed an HQ session token and MCP
configuration; a general-purpose agent is never wired into HQ that way.

The sandbox policy is kept in `session.json` and applied again on restore.
`agent.list` shows each agent's mode and `host.status` counts agents running
with `none`.

The agent cannot write `~/.claude/settings*.json`, `~/.claude/hooks`, or in its
project `.git/hooks`, `.git/config`, `.mcp.json` and `.claude/settings*.json`,
where it could plant code you run later outside the sandbox, and it cannot start
`open`, `osascript` or `launchctl`. The host refuses to start an agent in a
directory that contains your home, the host directory or a secret directory.

What it does not stop: the agent still reads and writes your project and
`~/.claude` (outside the read-only files above); on macOS the Keychain stays reachable because Claude Code keeps its
login there; your own MCP servers, plugins and connectors are refused until you
allow their hosts; a kernel flaw or an allowed domain is out of scope. On Linux
(including WSL2) `bwrap` has no network of its own, so the agent reaches the proxy
through a unix socket and a relay inside the sandbox; a path that does not exist
(a `.mcp.json` the project lacks) cannot be protected there, and each agent gets a
private empty `/tmp`, `/var/tmp` and `/run`.

The host bounds what a client can make it hold: 128 agents, 64 KiB of command
line each, window titles cut to 256 bytes, conversation ids limited to short
plain tokens, `agent.wait` to 10 minutes, a reply that would not fit one
protocol line replaced by an error, and a pane that stops reading its terminal
fails a write after 5 seconds instead of holding the connection. A client also
refuses a host directory that is not owned by you and private to you.

### Agent compatibility

What each agent needed to start under the sandbox, and whether it has been run signed in end to
end. A launch that fails for a reason on this list is a known quirk, not a new bug. Login state is
the user's: HQ cannot sign an agent in for you, and an agent with no login shows its own sign-in
screen.

| Agent | Run signed in | What HQ does for it |
| --- | --- | --- |
| Claude Code | yes | Writes only its config directory; `DISABLE_AUTOUPDATER=1`; project trust recorded by the host. Hooks named in its settings must live in a readable path (add the directory to `agent_host.sandbox.readable`, and any state directory to `writable`), or the prompt hook fails and every prompt is blocked. |
| Antigravity (`agy`) | yes | Own state in `~/.gemini`; Google sign-in and model hosts allowed; trust dialog answered at launch. |
| Codex | yes | Starts with `--no-daemon` (its daemon check reads other processes, which the sandbox forbids), also on resume. |
| Cursor | yes | No `--trust` (headless only); the workspace trust dialog appears on first use in a folder and the first prompt can be lost to it, so send the prompt again. `*.cursor.sh` allowed. |
| pi | yes, reaches its model | Session directory is `{home}/.pi/hq-sessions/<id>` on the host that runs it. Uses whichever provider you logged into, so add that provider's host to `agent_host.sandbox.allow_domains` (Google's Gemini API is built in). |
| GitHub Copilot, opencode, kimi, qwen | no | Hosts for the first are built in from its documented domains; the others depend on the model provider you configure, so add it to `agent_host.sandbox.allow_domains`. |

Every Node-based agent (Cursor, pi) needs `NODE_USE_ENV_PROXY=1` to send its requests through the
egress proxy; the host sets it for all agents.

## Lockdown

The kill switch for agents nobody asked for. While `lockdown.json` (mode 0600)
sits in the host directory, `agent.spawn` and `agent.resume` answer
`locked_down`, and a host that starts up restores nothing. A lockdown forgets
the agents it stopped and any waiting ones, so unlocking brings none of them
back by itself; the agents' own transcripts stay, and HQ keeps what it needs to
resume one on purpose (`harness_session_resume`).

- **Locking** is open to everyone who can drive the host: `hq host lockdown
  "<reason>"` on the machine, the `host.lockdown` method (also through the ssh
  gate), HQ's `host_lockdown` tool, which locks every host when none is named,
  and the **Stop all** button on the web app's Workbench page (beside New agent,
  one tap and a confirmation on a phone; `POST /api/workbench/lockdown`). The
  Workbench names any locked computer above its agent list.
  It kills every agent the host runs with its whole process group at once (no
  grace period, so a child that ignores SIGHUP dies with its running agent;
  children left behind by an agent that already exited are not reached). A start that raced
  the lockdown is killed as soon as it lands. `hq host lockdown` writes the
  file before it asks the host, so new starts are refused even when the host is
  wedged or down. A host that cannot write the file stays locked in memory
  until it restarts.
- **Unlocking** is only `hq host unlock` on that machine, which deletes the file.
  No method unlocks, so a key that can drive the host from elsewhere cannot lift
  a lockdown, and sandboxed agents cannot reach the directory.
- **The breaker.** `hq host serve` locks itself when more than `--max-starts`
  agents (default 50) start within 10 minutes. It refuses the start that went
  over but stops nothing already running. Only starts that succeeded count;
  agents restored or resumed after a restart do not.
- **The cap.** `--max-agents` (default 64) bounds the agents running or waiting
  at once, so a slow leak stops at a known number. Saved agents over the cap at
  startup wait instead of being dropped.
- **Alerts.** A lockdown is an event (`kind: lockdown`, the reason in `rule`).
  HQ's daemon raises an action-needed inbox item and an interrupting relay
  message for it.
- **Tests.** A cargo test binary that resolves a real host through
  `NativeBackend` refuses every call it makes to the machine's own host
  directory or to a remote over the real `ssh`; a test uses a temporary host
  directory or a stand-in program. A test that forgot its folder once started
  real agents this way.

The lockdown covers agents this host runs. Agents a person starts in a terminal,
and other tools that start agents, are outside it.

## Restarts

The host keeps `session.json` in its directory (mode 0600). It lists every
running agent that was spawned with a `resume_argv`: the command that continues
the old session, for example `["claude", "--continue"]`, together with its
working directory, agent kind, size and the names (never the values) of the
extra `env` variables it was started with, so no key or token is ever written
to disk. The file is rewritten when an agent is spawned, removed, told to stop, or
exits.

On `hq host serve` the host starts each listed agent again with its
`resume_argv`, in the same name and directory. These are fresh processes: what
survives is whatever the agent itself resumes (Claude Code's conversation, for
instance), not the old process or its scrollback. An agent that exited, was
removed or was told to stop is not listed. One that cannot be started (its
directory is gone, say) is reported and skipped; the others still come back.
Agents spawned without a `resume_argv` are not brought back.

An agent that was started with extra `env` is not started on its own, because
the host no longer has those values. It is listed by `agent.awaiting` and starts
when whoever launched it calls `agent.resume` with the values again (HQ rebuilds
them from the same profile and session id it used at launch). An agent started
without extra `env` still comes back by itself.

A host that stops cleanly (`hq host stop`, SIGTERM, Ctrl-C) stops its agents but
leaves them listed, so the next start brings them back. A host that is killed or
crashes leaves the file as it was, with the same result.

`session.json` holds commands the host will run, so it is used only when it is a
regular file owned by you with mode 0600 inside a directory you own; otherwise it
is ignored and reported. A file that does not parse is moved to
`session.json.unreadable`.

`host.status` has `binary_stale`: true when the executable the host started from
has been replaced on disk (a new build was installed). A running host keeps
working after its binary is replaced; restart it when `agents_working` and
`agents_blocked` are 0, or when you choose to. Restarting means stopping the host
and starting it again, normally through the service manager, for example
`systemctl restart hq-host`.

Example service files (not installed by HQ yet). On macOS use a LaunchAgent (a login-session job), not a LaunchDaemon: Claude Code's login lives in your Keychain, which a system daemon cannot reach. Keep the `hq` binary outside `~/Documents`, `~/Desktop` and `~/Downloads`, which launchd jobs may not read:

```
# /etc/systemd/system/hq-host.service
[Service]
User=hq
ExecStart=/usr/local/bin/hq host serve --dir /var/lib/hq/host
Restart=on-failure
KillMode=process
```

```
<!-- ~/Library/LaunchAgents/com.example.hq-host.plist -->
<dict>
  <key>Label</key><string>com.example.hq-host</string>
  <key>ProgramArguments</key><array>
    <string>/usr/local/bin/hq</string><string>host</string><string>serve</string>
  </array>
  <key>KeepAlive</key><true/>
</dict>
```

## What an agent process inherits

A pane starts from an empty environment plus: `PATH`, `HOME`, `USER`, `LOGNAME`,
`SHELL`, `LANG`, `LANGUAGE`, `LC_*`, `TMPDIR`, `TZ`, `COLORTERM`,
`XDG_CONFIG_HOME`, `XDG_DATA_HOME`, `XDG_CACHE_HOME`, `XDG_STATE_HOME`,
`TERM=xterm-256color`, and whatever `env` the spawn request passes. Credential
handles are deliberately not inherited: `SSH_AUTH_SOCK` would let an agent sign
with your keys and `XDG_RUNTIME_DIR` reaches your dbus, gpg and systemd
sockets. Pass one in `env` when an agent really needs it (for example git over
ssh). Markers and secrets of whatever
started the host (for example another agent session's `CLAUDE_CODE_*`
variables, which switch transcript saving off) never reach an agent.

## Using it from HQ

`HQ_HOST_DIR` overrides where HQ looks for the host (default `~/.hq/run/host`),
which is also how a second, isolated host is run for testing. A launch is
reported ready only after the agent has drawn its first screen and gone quiet;
an agent that is still starting, or sits at a dialog such as Claude's folder
trust prompt, is reported as `blocked` with its screen and no prompt is typed.

A Claude Code session resumes into its own conversation: once its hooks report
the conversation id, HQ stores it as the session's resume token and gives the host
a restart command that uses `--resume <id>`. Until the id is known, resume falls
back to `-c` (the most recent conversation in the directory).

New coding-agent sessions start on `agent_host.default_host` in `~/.hq/config.yaml`
(`native`, the host on the machine HQ runs on, unless you name a paired machine).
`native` and `local` both mean that host. A session keeps the host it was started
on, so changing the default never moves a running session. After a host restart the
supervisor starts any held agent again with the env HQ built for it at launch
(profile variables and the session id), once per sweep.

## A host on another machine

HQ on a server reaches a host on your laptop (or any machine) over ssh. Pair it with
`hq host join` on the machine and `hq host add` (or the `host_add` tool) on the HQ,
which creates the key and the config entry and prints the one `hq host authorize`
command the machine runs; the steps, including WSL2, are in
[JOIN_A_MACHINE.md](JOIN_A_MACHINE.md). The result is a pinned line in the machine's
`authorized_keys`:

```
restrict,from="<hq-server-address>",command="/home/you/.local/bin/hq host gate" ssh-ed25519 AAAA... hq-gate-laptop
```

and a config entry:

```yaml
agent_host:
  hosts:
    laptop:
      ssh: "you@laptop.example.ts.net"
      identity_file: "/opt/hq/.ssh/hq_gate_laptop"
      # port: 2222   # when ssh does not listen on 22
```

Sessions started with `host: laptop` run there. `hq host gate` reads one request
from stdin (a JSON array of a method name and its params as JSON text, so no
shell parses either), refuses any method outside an allowlist (`host.stop` and
`agent.report` are left out), forwards it to the local socket and prints the
reply. The key cannot run anything else: a command passed to ssh is ignored.
`agent.spawn` still starts whatever command it is given, so treat the key as
shell access as you. A missing agent binary on the remote machine comes back as
the host's own `spawn_failed` error naming the command. The hook settings file is
written on the remote machine by the host (`agent.hook_flags`), so hooks,
conversation ids and resume work the same as locally.

### Over a private tunnel instead of ssh

A host can also be reached over TCP on a private tunnel address (for example a
WireGuard network between your server and your laptop), with no ssh on either side.
The caller proves itself with an Ed25519 key, so there is no shared secret and no
password to leak. Nothing listens unless `agent_host.gate_listen` is set. `hq net` sets up the
WireGuard tunnel itself without hand-written `wg` commands: see [PRIVATE_NETWORK.md](PRIVATE_NETWORK.md).

On the machine that runs the host:

```yaml
agent_host:
  gate_listen: "10.77.0.2:7747"          # the machine's tunnel address, never 0.0.0.0
  gate_allowed_sources: ["10.77.0.1"]    # the server's tunnel address; required
```

On the HQ server, make a key for the host and read its public half:

```
hq host gate-key laptop
```

Then, on the machine, authorize it from the server's tunnel address only:

```
hq host authorize --gate-key <public key> --from 10.77.0.1 laptop
```

and point the server's config at the machine:

```yaml
agent_host:
  hosts:
    laptop:
      transport: tcp
      addr: "10.77.0.2:7747"
```

How it holds up:

- The listener refuses to start on an unspecified address, on an address that is not loopback
  or private (a public one), or with an empty `gate_allowed_sources`. A gate that is configured
  but invalid stops `hq host serve` before it starts anything, instead of leaving you unsure
  whether it is open. If the address is one the machine does not have yet (the tunnel comes up
  after login), the host starts and the gate binds as soon as the address exists. A config file
  that cannot be read at all leaves the gate off, with a warning.
- A source outside `gate_allowed_sources` gets a closed connection and nothing else. An
  allowed source gets a one-line challenge with a fresh random nonce. A caller whose key is not in
  `~/.hq/gate_authorized_keys` (or who connects from an address that key is not pinned
  to with `from=`) gets a closed connection, with no reply to tell it why.
- The signature covers the nonce and a digest of the request, so a recorded exchange
  cannot be replayed, and the request is read only after the key is known. The request
  then goes through the same method allowlist as `hq host gate`.
- Limits: 5 s to authenticate, 10 s to read and write, 16 connections at once. There is no
  lockout after failures, because keys cannot be guessed and a lockout would let anyone who
  can reach the gate from the allowed address shut the real caller out. Whoever can connect
  from an allowed address can still hold the 16 slots for a few seconds each, which is why
  `gate_allowed_sources` should be only the server's address on the tunnel.
- The authorized keys file is read on every connection, so revoking a key takes effect at
  once. It and the key files must be plain files (no links) owned by the user running HQ; the
  authorized keys file must not be writable by group or others, and a key file not readable by
  them. Otherwise the gate refuses every key.
- The chat shell cannot read `~/.hq/gate/*` or write `~/.hq/gate_authorized_keys`. If you point
  `gate_key_file` or `gate_authorized_keys` somewhere else, keep that path out of reach of
  agents yourself.
- The key can start any command as you inside the agent sandbox, like an ssh login: keep
  `gate_allowed_sources` to the one address the server uses on the tunnel.

What the gate does not do: it does not encrypt, and the server does not prove who it is to the
caller. Both come from the tunnel (WireGuard encrypts and only the peer holding the other key
can answer on that address). Use it on a machine only you log in to: on a shared machine
another local user could listen on the tunnel address while the host is down and receive the
server's calls, which include the HQ MCP token. Use the ssh transport there. Signed replies and a
pinned server key would close this and are listed in `TECHDEBT.md`.

The wire protocol is written out in `crates/hq-gate/src/lib.rs`.

## Day to day

- A call costs one ssh round trip plus about 40 ms for the gate on the laptop;
  `agent_host.ssh_multiplex` keeps the ssh connection open between calls. State
  changes arrive as events instead of one poll a minute.
- A laptop that is asleep or off the tailnet is skipped, not marked dead; its
  sessions are picked up again when it answers.
- No terminal UI: `hq host status` on the machine and `harness_session_list` in HQ
  show every session. A session silent and not working for
  `agent_host.idle_reap_hours` (24 by default) is stopped by the host itself and can
  be resumed; HQ tells you it ended. Set it to 0 to keep sessions until stopped.
- Your own MCP servers, plugins and connectors are refused inside the sandbox
  until their hosts are allowed; the host's egress log names each denied host.

## Not built yet

Rule files for agents other than Claude Code and Codex, hooks, MCP config and sandbox-aware launching for agents other than Claude Code, Host-header fronting inside TLS, and reads outside your home directory. Provenance of anything adapted from the project this host replaced is recorded in `docs/provenance/herdr.md`.
