# Web tools and remote MCP servers

Notes for working under `crates/hq-tools-web/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Remote MCP servers (`remote_mcp:` config) | `crates/hq-tools-web/src/remote_mcp.rs` |

## Remote MCP servers

`crates/hq-tools-web/src/remote_mcp.rs` bridges any Streamable HTTP MCP server into
HQ's tool registry. Each entry under `remote_mcp:` gets two tools,
`<name>_discover` (tools/list) and `<name>_call` (tools/call), in the shared
`remote_mcp` category that the Telegram guest profile denies. Pick a name that
differs from the server's own tool prefixes (a server exposing `foo_call` should
not be named `foo`), since the server's tools are nested one level under
`<name>_call`. Entries default to `live_user_turn_only: true`, which keeps them
out of unattended sessions.

```yaml
remote_mcp:
  - name: diagrams
    url: https://example.com/api/mcp
    api_key: "..."
```
