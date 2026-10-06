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
- `operator.token`: a random secret, mode 0600, created on first start.

A second `serve` on the same directory fails while the first answers. A socket
left behind by a host that died is replaced.

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
| `agent.spawn` | `name`, `argv`, `cwd`, optional `env` (object), `rows`, `cols`, `scrollback_rows` | agent info |
| `agent.list` | none | `{"agents": [...]}` |
| `agent.get` | `name` | agent info |
| `agent.read` | `name`, optional `source` (`visible`, `recent`, `recent_unwrapped`; default `recent_unwrapped`), optional `lines` (last N, 0 or absent for all) | `{"text"}` |
| `agent.send_text` | `name`, `text` | `{}` (raw bytes, as given) |
| `agent.paste` | `name`, `text` | `{}` (bracketed paste when the program enabled it) |
| `agent.prompt` | `name`, `text` | `{}` (paste, short pause, Enter) |
| `agent.send_keys` | `name`, `keys` (`enter`, `esc`, `tab`, arrows, `pageup`, `ctrl+c`, ...) | `{}` |
| `agent.resize` | `name`, `rows`, `cols` | `{}` |
| `agent.wait` | `name`, `until` (`exit` or `quiet`), optional `timeout_ms`, `quiet_ms` | `exit_code`, or agent info |
| `agent.kill` | `name` | `{}` |
| `agent.remove` | `name` | `{}` |

Agent names match `[a-z][a-z0-9_-]{0,31}` and are unique. Agent info has `name`,
`argv`, `cwd`, `pid`, `status` (`running` or `exited`), `exit_code`, `rows`,
`cols`, `bytes_seen`, `quiet_ms`, `age_ms`. Error codes include `agent_not_found`,
`name_taken`, `invalid_name`, `invalid_keys`, `spawn_failed`, `agent_exited`,
`timeout`, `invalid_params`, `unknown_method`, `bad_request`.

## What an agent process inherits

A pane starts from an empty environment plus: `PATH`, `HOME`, `USER`, `LOGNAME`,
`SHELL`, `LANG`, `LANGUAGE`, `LC_*`, `XDG_*`, `TMPDIR`, `TZ`, `COLORTERM`,
`SSH_AUTH_SOCK`, `DISPLAY`, `WAYLAND_DISPLAY`, `TERM=xterm-256color`, and
whatever `env` the spawn request passes. Markers and secrets of whatever
started the host (for example another agent session's `CLAUDE_CODE_*`
variables, which switch transcript saving off) never reach an agent.

## Not built yet

Agent state detection (working, blocked, idle), persistence across a host
restart, remote hosts, the backend that makes HQ sessions use this host, and
agent-to-agent messaging. Provenance of anything adapted from herdr is recorded
in `docs/provenance/herdr.md`.
