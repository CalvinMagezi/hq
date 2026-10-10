# Connecting an Agent to HQ's MCP Endpoint

HQ serves its MCP gateway (`hq_discover`, `hq_call`) at `POST /mcp` over
Streamable HTTP. A remote agent reaches it through a TLS proxy such as Caddy.

## Architecture

```
Agent (Claude Code, Cursor, a script)
    │
    ▼ HTTPS, Authorization: Bearer <AGENTHQ_API_KEY>
Caddy on the server (mcp.your-domain.com)
    │
    ▼ 127.0.0.1:5678
hq start all (serves /mcp, /health, the web UI)
```

## 1. Server setup

1. Set the key HQ checks on `/mcp`. `AGENTHQ_API_KEY` grants full access;
   `AGENTHQ_SPARK_API_KEY` is optional and limited to a read-only tool set.
   `AGENTHQ_TASKS_API_KEY` is optional and opens only the task tools (list, get, create,
   update, comment, claim, next, heartbeat, release, bulk create and update, plus folders and
   initiatives): no vault, no session tools, no way to run
   code, and no mailbox notifications. It is the key to give an editor agent that should use
   HQ's tasks and nothing else.
   `AGENTHQ_HANDOFF_API_KEY` is optional and adds task writes and session
   spawn/handoff (for a client that hands work to coding agents). It is
   equivalent to code execution on the hosts: set `agent_host.handoff_cwd_allow`.
   Without either, `/mcp` refuses every request (see
   `docs/security/WEB_AUTH.md`).
   ```bash
   export AGENTHQ_API_KEY="$(openssl rand -hex 32)"
   ```
2. Copy `deploy/Caddyfile.production` to `/etc/caddy/Caddyfile`, replace
   `mcp.your-domain.com` with your domain, and reload Caddy:
   ```bash
   systemctl reload caddy
   ```

## 2. Agent configuration

```json
{
  "mcpServers": {
    "agent-hq": {
      "url": "https://mcp.your-domain.com/mcp",
      "headers": {
        "Authorization": "Bearer <AGENTHQ_API_KEY>",
        "Content-Type": "application/json"
      }
    }
  }
}
```

## 3. Verification

```bash
./deploy/test-vps-connection.sh https://mcp.your-domain.com "$AGENTHQ_API_KEY"
```

It checks `/health`, the MCP `initialize` handshake, `tools/list`, and one
`hq_discover` call.

## VS Code

VS Code reads remote servers from `.vscode/mcp.json` in a workspace (or `mcp.json` in your user
profile). Its top-level key is `servers`, and a root `inputs` entry makes VS Code ask for the key
instead of storing it in the file. Let HQ write it:

```bash
hq mcp install --target project --url https://mcp.your-domain.com/mcp
```

which produces

```json
{
  "inputs": [
    { "id": "agent-hq-key", "type": "promptString", "description": "Agent HQ MCP key", "password": true }
  ],
  "servers": {
    "agent-hq": {
      "type": "http",
      "url": "https://mcp.your-domain.com/mcp",
      "headers": { "Authorization": "Bearer ${input:agent-hq-key}" }
    }
  }
}
```

Give VS Code the tasks key, not the full key. `--url` accepts `https://`
URLs, and `http://` only for `localhost`; it refuses a URL that carries credentials, a query or a
fragment. An organization can switch MCP off for Copilot ("MCP servers in Copilot" policy); if it
has, this does not work and the answer is a conversation with your administrator.

To run HQ on the same machine instead, install a local server limited to the same tools:

```bash
hq mcp install --target vscode --scope tasks
```

This writes `hq mcp-serve --scope tasks` into VS Code's user `mcp.json`. The scopes are `full`
(the default) and `tasks`, the same list the HTTP key uses; `--scope` needs `--target`, so it never
rewrites your own full-access entries in other clients, and a later plain `hq mcp install` keeps an
entry's scope instead of widening it. A scoped server sends no tool catalog in its instructions,
so a client is not told about tools it cannot call. The scope is a flag in a file you can edit, so
it limits what an editor agent is handed, not what a person on that machine can do; only the
remote key is enforced by the server.
