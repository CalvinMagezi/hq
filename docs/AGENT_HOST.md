# Agent host

`hq host` is HQ's built-in host for long-lived coding agents. It runs each agent
in a pseudo-terminal, keeps an emulated screen and scrollback you can read back,
and accepts typed input. It is the replacement for the external herdr binary and
is being introduced in steps: today it runs and can be driven over a socket, but
HQ's sessions still use herdr until the native backend is switched on.

## Run it

```
hq host serve            # run the host in this terminal (Ctrl-C stops it)
hq host status           # protocol and host version, pid, number of agents
hq host stop             # ask a running host to exit
```

All three take `--dir <path>` (default `~/.hq/run/host`). The directory is
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
| `host.status` | none | `protocol_version`, `host_version`, `pid`, `agents`, `agents_working`, `agents_blocked`, `binary_stale` |
| `host.stop` | none | `{}` |
| `agent.spawn` | `name`, `argv`, `cwd`, optional `agent` (kind, for state detection), `resume_argv` (see Restarts), `env` (object), `rows`, `cols`, `scrollback_rows` | agent info |
| `agent.list` | none | `{"agents": [...]}` |
| `agent.get` | `name` | agent info |
| `agent.read` | `name`, optional `source` (`visible`, `recent`, `recent_unwrapped`; default `recent_unwrapped`), optional `lines` (last N, 0 or absent for all) | `{"text", "truncated"}`; text is cut to its last 400 KiB, at a line start, when longer |
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

Rule files are TOML in the format herdr documents: each rule names a screen
region and a gate of phrases or patterns, and the matching rule with the highest
priority decides. They are built into the binary (`crates/hq-host/src/detect/manifests/`);
there is no fetching from anywhere at run time. Screens captured from real
Claude Code and Codex sessions, each labelled with the state the agent was really
in, are in `crates/hq-host/tests/fixtures/` and are checked on every test run.

### Done, the change counter and events

Agent info also carries `done` (a turn finished and the agent has not worked
since, herdr's `done`) and `state_seq`, a number that goes up each time the
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
host (`herdr.sandbox`, default mode `process`). In process mode the host wraps
the agent in `sandbox-exec` (macOS) or `bwrap` (Linux) and refuses to start it
when that is not possible. The agent:

- cannot read the run directory except `host.sock` and its own hook and MCP
  files, nor `~/.ssh`, `~/.gnupg`, `~/.aws`, `~/.kube` or `~/.hq`;
- can write only its project, `~/.claude`, caches and the temporary directories
  (plus `herdr.sandbox.writable`);
- cannot see other processes' environments;
- can connect only to its own egress proxy and the host socket. The proxy allows
  `api.anthropic.com`, the HQ MCP endpoint and `herdr.sandbox.allow_domains`,
  resolves names itself, and refuses private, loopback and link-local addresses
  unless the endpoint is the MCP one. Every decision is logged
  (`agent.egress`); a denied host shows up there so you can add it.

The sandbox policy is kept in `session.json` and applied again on restore.
`agent.list` shows each agent's mode and `host.status` counts agents running
with `none`.

What it does not stop: the agent still reads and writes your project and
`~/.claude`; on macOS the Keychain stays reachable because Claude Code keeps its
login there; your own MCP servers, plugins and connectors are refused until you
allow their hosts; a kernel flaw or an allowed domain is out of scope. On Linux
`bwrap` cannot yet enforce the proxy-only network, so process mode fails closed
there; set `herdr.sandbox.mode: none` knowingly or wait for the bridge.

The host bounds what a client can make it hold: 128 agents, 64 KiB of command
line each, window titles cut to 256 bytes, conversation ids limited to short
plain tokens, `agent.wait` to 10 minutes, a reply that would not fit one
protocol line replaced by an error, and a pane that stops reading its terminal
fails a write after 5 seconds instead of holding the connection. A client also
refuses a host directory that is not owned by you and private to you.

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

Example service files (not installed by HQ yet):

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

Set `herdr.default_host: native` in `~/.hq/config.yaml` and new coding-agent
sessions start on the built-in host (run `hq host serve` first). `native` is a
reserved host name next to `local`. A session keeps the host it was started on,
so flipping the default never moves a running session, and sessions on `local`
or a remote keep using herdr. After a host restart the supervisor starts any
held agent again with the env HQ built for it at launch (profile variables and
the session id), once per sweep.

## A host on another machine

HQ on a server reaches a host on your laptop (or any machine) over ssh. On that
machine run `hq host serve` (see the service files above), then pin a key to the
gate in `~/.ssh/authorized_keys` on one line:

```
restrict,from="<hq-server-address>",command="/usr/local/bin/hq host gate" ssh-ed25519 AAAA... hq-gate
```

and add the machine to HQ's config:

```yaml
herdr:
  hosts:
    laptop:
      kind: native
      ssh: "you@laptop.example.ts.net"
      identity_file: "~/.ssh/hq_gate"
      # port: 2222   # when ssh does not listen on 22
```

Sessions started with `host: laptop` run there. `hq host gate` reads one request
from stdin (a JSON array of a method name and its params as JSON text, so no
shell parses either), refuses any method outside an allowlist (`host.stop` and
`agent.report` are left out), forwards it to the local socket and prints the
reply. The key cannot run anything else: a command passed to ssh is ignored.
`agent.spawn` still starts whatever command it is given, so treat the key as
shell access as you, the same as the herdr gate. A missing agent binary on the
remote machine comes back as the host's own `spawn_failed` error naming the
command. The hook settings file is written on the remote machine by the host
(`agent.hook_flags`), so hooks, conversation ids and resume work the same as
locally.

## Not built yet

Rule files for agents other than Claude Code and Codex, hooks for agents other than Claude Code, and agent-to-agent messaging. Provenance of anything adapted from herdr is recorded
in `docs/provenance/herdr.md`.
