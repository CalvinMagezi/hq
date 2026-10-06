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
| `host.status` | none | `protocol_version`, `host_version`, `pid`, `agents` |
| `host.stop` | none | `{}` |
| `agent.spawn` | `name`, `argv`, `cwd`, optional `agent` (kind, for state detection), `env` (object), `rows`, `cols`, `scrollback_rows` | agent info |
| `agent.list` | none | `{"agents": [...]}` |
| `agent.get` | `name` | agent info |
| `agent.read` | `name`, optional `source` (`visible`, `recent`, `recent_unwrapped`; default `recent_unwrapped`), optional `lines` (last N, 0 or absent for all) | `{"text", "truncated"}`; text is cut to its last 400 KiB, at a line start, when longer |
| `agent.send_text` | `name`, `text` | `{}` (raw bytes, as given) |
| `agent.paste` | `name`, `text` | `{}` (bracketed paste when the program enabled it) |
| `agent.prompt` | `name`, `text` | `{}` (paste, short pause, Enter) |
| `agent.send_keys` | `name`, `keys` (`enter`, `esc`, `tab`, arrows, `pageup`, `ctrl+c`, ...) | `{}` |
| `agent.resize` | `name`, `rows`, `cols` | `{}` |
| `agent.wait` | `name`, `until` (`exit`, `quiet` or `state`), optional `timeout_ms`; for `quiet` `quiet_ms`; for `state` `states` (list, required) and `stable_ms` (default 300) | `exit_code`, or agent info |
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
`argv`, `agent`, `title`, `state`, `rule`, `cwd`, `pid`, `status` (`running` or `exited`), `exit_code`, `rows`,
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

## Not built yet

Rule files for agents other than Claude Code and Codex, the "done" state (finished a turn you have not looked at yet), agent-reported state through hooks, persistence across a host
restart, remote hosts, the backend that makes HQ sessions use this host, and
agent-to-agent messaging. Provenance of anything adapted from herdr is recorded
in `docs/provenance/herdr.md`.
