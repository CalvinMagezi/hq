# Cutting over from herdr to the built-in host

This is the order of work for moving HQ's coding-agent sessions from herdr to
`hq host`, with HQ on a server and the agents running on a laptop. Nothing here
changes a running session: a session keeps the host it started on, and
`herdr.default_host` stays `local` (herdr) until you change it.

## Before merging

- The probe PR (`probe/host-stack`, CI only) has green Rust, macOS build, supply
  chain and personal-data checks for the whole stack on top of current `main`.
- Review and merge the stack bottom-up, one PR at a time, so each merge is the
  exact diff that was reviewed: #52, #54, #55, #58, #59, #60, #62, #66, #73, #74,
  #75, #77, #81, #85 (sandbox crate), #84 (egress proxy), #87 (host integration),
  #88 (docs), #90 (review fixes and enforcement). #79 (wording guard) is
  independent. Delete each branch after merging so the next PR retargets `main`.
- A stack merge does not change behaviour by itself: the native backend is used
  only for hosts named `native` or configured `kind: native`.

## On the laptop

1. Put the `hq` binary somewhere launchd can read (not `~/Documents`, `~/Desktop`
   or `~/Downloads`), for example `~/.local/bin/hq`.
2. Run the host as a LaunchAgent so it lives in your login session (Keychain,
   needed by Claude Code's login). Use the plist in `AGENT_HOST.md`, adding
   `RunAtLoad`, `KeepAlive` and log paths, and `launchctl bootstrap gui/$(id -u)
   <plist>`. The host refuses agents outside the process sandbox unless started
   with `--allow-unsandboxed`.
3. Pin a key to `hq host gate` in `~/.ssh/authorized_keys` with `restrict` and
   `from="<server tailnet address>"`, as in `AGENT_HOST.md`. The key is shell
   access as you; treat it that way.
4. Run `hq host status`: it lists the agents, their sandbox mode and idle time.

## On the server

1. Config in `~/.hq/config.yaml`: the laptop under `herdr.hosts` with
   `kind: native`, `herdr.agent_mcp_url` (so sessions get their own token), and
   if the defaults are too tight, `herdr.sandbox.allow_domains`,
   `herdr.sandbox.readable`, `herdr.sandbox.writable`.
2. Start one session with `host: laptop`. Check that it reaches Idle, that you can
   read its screen and send it a prompt, that a task comment appears under its
   own session name, and that a denied host shows up in the host's egress log.
3. Delegate and report between two sessions, then stop the host with an agent
   running and start it again: the agent must come back with its conversation.
4. Only then set `herdr.default_host: laptop`.

## Rolling back

Set `herdr.default_host` back to `local`. Sessions already on the laptop keep
running and stay manageable; new ones use herdr again.

## What to expect day to day

- A call costs one ssh round trip plus about 40 ms for the gate on the laptop
  (debug build); `herdr.ssh_multiplex` keeps the ssh connection open between
  calls. State changes arrive as events instead of one poll a minute.
- A laptop that is asleep or off the tailnet is skipped, not marked dead; its
  sessions are picked up again when it answers.
- No terminal UI: `hq host status` on the laptop and `harness_session_list` in HQ
  show every session. A session silent and not working for
  `herdr.idle_reap_hours` (24 by default) is stopped by the host itself and can
  be resumed; HQ tells you it ended. Set it to 0 to keep sessions until stopped.
- Your own MCP servers, plugins and connectors are refused inside the sandbox
  until their hosts are allowed; the host's egress log names each denied host.

## Not covered yet

Linux hosts (the sandbox cannot enforce its network allowlist there yet, so a
Linux host needs `--allow-unsandboxed`), Host-header fronting inside TLS, reads
outside your home directory, and agents other than Claude Code (no hooks, MCP
config or sandbox-aware launch).
