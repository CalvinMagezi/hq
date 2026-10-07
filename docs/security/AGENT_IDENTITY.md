# Agent identity

HQ can tell which launched coding-agent session is calling it, so what an agent
does on behalf of itself (comment on a task, and later message a peer or claim
work) is attributed to a session HQ launched, not to a name the agent typed.

## How it works

1. Set `herdr.agent_mcp_url` to the HQ MCP endpoint the agent's machine can
   reach (for example `https://hq.example.ts.net:8444/mcp`). Without it nothing
   below happens.
2. When HQ launches a Claude Code session on the built-in host it mints a secret
   for that session. Only the SHA-256 is stored (`harness_session_tokens`).
3. The host writes `<run dir>/mcp/<session>.json` (mode 0600) holding the
   endpoint and `Authorization: Bearer <secret>`, and starts the agent with
   `--mcp-config <that file>`. The server appears to the agent as `hq-session`,
   beside any MCP servers the user already configured. The secret never appears
   on a command line. The flag is kept in the restart command, so a resumed agent
   stays connected.
4. `/mcp` accepts the secret like a key, but it identifies one session, only
   while that session is running, and only the tools in
   `hq_mcp::gateway::SESSION_ALLOWLIST` (task reads, `task_comment_add`,
   `harness_session_status`). Everything else, including vault reads, settings
   and session spawning, is refused. The gateway sets `_hq_caller_session` on the
   call, and removes any value a caller supplied itself, so a tool that reads it
   can trust it.
5. `task_comment_add` uses that id as the comment's author, whatever `author` the
   agent passes.

## What it does not do

- The secret is readable by the agent and by anything it runs, because the agent
  needs it. A leaked secret can act as that session until the session ends.
- Only Claude Code is wired. Other agents have no MCP config path yet.
- Permissions beyond the fixed allowlist, quotas and parent and child limits are
  not built yet.
