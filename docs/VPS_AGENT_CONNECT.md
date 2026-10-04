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
   `AGENTHQ_HANDOFF_API_KEY` is optional and adds task writes and session
   spawn/handoff (for a client that hands work to coding agents). It is
   equivalent to code execution on the Herdr hosts: set `herdr.handoff_cwd_allow`.
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
