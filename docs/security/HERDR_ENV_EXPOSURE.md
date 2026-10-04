# Herdr panes inherit the provider environment

`deploy/herdr.service` loads provider credentials into the Herdr server's
environment with `EnvironmentFile=` lines (`openrouter.env`, `copilot.env`,
`deepseek.env`, `github.env`, `gh.env`; each optional, each missing file
ignored). Every pane the server opens, and so every coding agent HQ starts in
one (`claude`, `codex`, `cursor`, `agy`, `pi`, and the rest), starts with that
environment. The Google Workspace credentials (`gws.env`) are deliberately
left out.

This is a different surface from the `bash` tool. The env allowlist in
`BASH_SANDBOX.md` applies to shell commands HQ's own agents run. It does not
apply to a harness session: the coding agent is a separate program with its
own shell access, and it can print its environment, read `/proc/self/environ`
or pass the variables to anything it runs. A prompt-injected harness session can
therefore exfiltrate every key the unit loaded.

## What to do about it

Load only what every agent you start really needs.

1. Delete the `EnvironmentFile=` lines for providers none of your harnesses
   use. A Claude Code, Codex or Cursor session that signs in with its own
   account needs none of the LLM provider files. Keep `gh.env` only if the
   agents must push or open pull requests, and give it a token with the
   narrowest scope that does that (a fine-grained token limited to the
   repositories involved).
2. Do not put a key in the Herdr environment to serve a single harness. Put
   it where only that harness starts: a harness profile whose `command` is a
   small wrapper that exports the key and then runs the CLI (see "Harness
   profiles" in `docs/HERDR_HARNESS.md`). Keep the wrapper and any env file it
   reads out of the repository, owned by the service user, mode `0600`.
   Profile `env:` values are written into `config.yaml` as plain text, so use
   them for non-secret settings such as `CLAUDE_CONFIG_DIR`.
3. If agents of different trust levels share a host, run a second Herdr
   server under a separate OS user with its own, smaller `EnvironmentFile=`
   set, and point the untrusted harnesses at it (`herdr.default_host` and
   `herdr.hosts`). File modes do not separate two agents that run as the same
   user; only a different user does.
4. Revoke and rotate any key that has been in a pane's environment if a
   session read untrusted content with shell access, as you would after any
   suspected exposure.

## Checking what a pane can see

From a pane: `env | cut -d= -f1 | sort` lists the variable names present
without printing values. Anything beyond `HOME`, `PATH`, `SHELL`, the HQ path
variables and `HQ_SESSION_ID` is a credential the unit handed over.

This document describes the shipped unit. The unit file itself is owned by the
deploy tooling and is not changed here.
