# Security policy

## Reporting a vulnerability

Please report security problems privately, not in a public issue or pull request.

1. Preferred: use GitHub's private vulnerability reporting on this repository
   (Security tab, "Report a vulnerability"). This creates a private security
   advisory that only the maintainers can see.
2. If that is unavailable, open a public issue that says only that you have a
   security report and asks for a private channel. Do not include details.

Include what you found, how to reproduce it, the affected version or commit, and
the impact you expect. You should get an acknowledgement within 7 days. Please
allow 90 days, or until a fix ships, before disclosing publicly. We will credit
you in the advisory unless you prefer otherwise.

Do not include real credentials, personal data or other people's vault contents
in a report. A redacted reproduction is enough.

## Supported versions

Only the latest release and the latest commit on `main` are supported. There are
no maintained release branches, so fixes ship as a new release.

## Scope

In scope:

- The `hq` binary and everything under `crates/`, including the web server, MCP
  gateway, chat relays, bash sandbox, governance policy and the updater.
- The web UI under `apps/hq-web`.
- The deployment files under `deploy/` and the install and update scripts under
  `scripts/`.

Out of scope:

- Vulnerabilities in third-party services or tools HQ talks to (LLM providers,
  Discord, Telegram, Herdr, the coding agents it supervises). Report those
  upstream.
- Attacks that need a deployment the docs tell you not to run, such as exposing
  `/ws` or `/api` to the public internet, or running with
  `HQ_MCP_DEV_NO_AUTH=1` on a non-loopback bind.
- Findings that need an attacker who already controls the host user, the vault
  directory or the config file.
- Model misbehavior on its own. A prompt injection that gets past the shell
  sandbox, the environment allowlist or the governance policy is in scope.

## Supported deployments and trust boundaries

HQ runs tool-using agents that can execute shell commands, so the deployment
shape matters more than usual.

- **Local, single user.** `web_bind: 127.0.0.1` (the default). The machine and
  its user are trusted. Set `web_auth_token` on a shared machine.
- **Private network.** HQ binds to loopback behind `tailscale serve` or a
  reverse proxy on a private network. List the served origins in
  `web_allowed_origins`.
- **Public `/mcp`.** Only through a reverse proxy that exposes `/mcp` and
  `/health` and nothing else, with `AGENTHQ_API_KEY` set. `/mcp` refuses every
  request without a key. `/ws` and `/api` must never be public.

Content the agent reads (web pages, email, documents, vault notes written by
others) is untrusted. Treat a model's refusal as a mitigation, not a boundary:
the boundaries are the shell sandbox, the environment allowlist and the
governance policy (see `docs/security/`).

## Details

- `docs/security/WEB_AUTH.md`: MCP keys, web token, cross-origin protection.
- `docs/security/WEB_FETCH.md`: SSRF protection for `web_fetch`.
- `docs/security/BASH_SANDBOX.md`: shell isolation and the environment allowlist.
- `docs/security/PROMPT_INJECTION.md`: policy for untrusted content.
- `docs/security/SELF_UPDATE.md`: the self-update tools, off by default, owner approval for install.
- `docs/security/SQL_AUDIT.md`: every string-built SQL statement and why it is safe.
- `docs/security/HERDR_ENV_EXPOSURE.md`: provider credentials that Herdr panes inherit, and how to narrow them.
- `docs/security/RELEASE_CHECKLIST.md`: what must pass before a public release.
