# What a coding agent can see of your environment

A coding agent HQ starts is a separate program with its own shell. It can print its
environment, read `/proc/self/environ` or pass its variables to anything it runs, so
anything in its environment is exposed to a prompt-injected session. This is a different
surface from the `bash` tool: the env allowlist in `BASH_SANDBOX.md` covers shell commands
HQ's own agents run, not harness sessions.

## What the host hands over

The host starts each agent from an empty environment and copies only an allowlist from
its own: `PATH`, `HOME`, `USER`, `LOGNAME`, `SHELL`, `LANG`, `LANGUAGE`, `LC_*`, `TMPDIR`,
`TZ`, `COLORTERM`, the `XDG_*` directory variables, and `TERM`. Provider credentials in the
service's environment (`openrouter.env`, `gh.env` and the like) are therefore not inherited,
and neither are `SSH_AUTH_SOCK` (which would let an agent sign with your keys) and
`XDG_RUNTIME_DIR` (which reaches your dbus, gpg and systemd sockets).

On top of that the agent gets what HQ passes in the spawn request:

- `HQ_SESSION_ID`, and its own `HQ_HOST_TOKEN` and `HQ_HOST_DIR` so its hooks can report to
  the host. The token works only on the agent socket and only for that agent.
- The `env:` of the harness profile it was started with (`agent_host.harness_profiles`).
  These values are written into `config.yaml` as plain text, so use them for non-secret
  settings such as `CLAUDE_CONFIG_DIR`, not for keys.
- In process-sandbox mode, the proxy variables that point its HTTP clients at its egress
  proxy, and `DISABLE_AUTOUPDATER`.

## What to do about it

1. Keep secrets out of profile `env:`. A harness that needs a key should be started through a
   profile whose `command` is a small wrapper that exports it and runs the CLI; keep the
   wrapper and any env file it reads outside the repository, owned by the service user, mode
   `0600`. Anything the wrapper exports is visible to that agent.
2. Leave the sandbox on (`agent_host.sandbox.mode: process`, the default). It also hides your
   credentials directories (`~/.ssh`, `~/.aws`, `~/.config/gh` and others), other Claude
   profiles, and the host's own token files, which the environment allowlist cannot do.
3. If agents of different trust levels share a machine, run them as separate OS users with
   separate hosts and point the untrusted harnesses at that host (`agent_host.default_host`
   and `agent_host.hosts`). File modes do not separate two agents running as the same user.
4. Revoke and rotate any key that was in an agent's environment if the session read
   untrusted content with shell access, as after any suspected exposure.

## Checking what an agent can see

From an agent's shell: `env | cut -d= -f1 | sort` lists the variable names present without
printing values. Anything beyond the allowlist above, `HQ_*` and the proxy variables is
something a profile or wrapper handed over.
