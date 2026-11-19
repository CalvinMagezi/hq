# The web server and its auth

Notes for working under `crates/hq-web/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| claude.ai connector (`mcp_connector:` config, OAuth for claude.ai and Claude Code, `hq connector`) | `crates/hq-web/src/oauth/` (server, consent API, `guard.rs` for approvals, rate limit and call log), `crates/hq-db/src/mcp_connector.rs`, `crates/hq-mcp/src/gateway.rs` (`CONNECTOR_CONFIRM_TOOLS`), `apps/hq-web/src/routes/connect.tsx`, `docs/CLAUDE_CONNECTOR.md`, `docs/plans/claude-connector.md`; the whole path from the forwarder to the door is `crates/hq-web/src/tls/tests/chain.rs`, and the real-account run is `docs/plans/native-bridge/w6-claude-run.md` |
| Web and MCP auth, origin guard | `crates/hq-web/src/auth.rs`, `crates/hq-web/src/origin.rs`, `docs/security/WEB_AUTH.md` |

## Rules

- **Three places.** The web app, the desktop window and the phone show Notes, Chat and Inbox.
  Tasks and Helpers are where HQ and its agents work: they keep their pages and addresses and
  open from an Inbox item, a chat card, search or Settings, never from the bar
  (`BAR_PLACES` in `apps/hq-web/src/lib/routes.ts`, `Place.bar` in the Android shell). Do not
  add a place to the bar for something only agents use. Plan: `docs/plans/three-places.md`.
