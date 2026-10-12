# HQ as a Claude connector

Status: plan, not started. Lets a person add their own HQ to claude.ai (web, Desktop, mobile)
as a custom connector, so a Claude conversation can reach the same `hq_discover` and `hq_call`
gateway that Claude Code already uses with a key. Off by default; nothing changes for an
instance that does not opt in.

## Feasibility

It works on Pro, Max, Team and Enterprise today, with two conditions HQ does not meet yet.

1. **HQ must be reachable from Anthropic's servers over public HTTPS.** Connector traffic comes
   from Anthropic's network (`160.79.104.0/21` at the time of writing), not from the user's
   browser. A tailnet-only instance cannot be added. Anthropic's private-network option (MCP
   tunnels) is Enterprise only and in research preview, so it is not a general answer.
2. **HQ must speak OAuth.** The "Add custom connector" dialog asks for a name and a URL only.
   Static request headers (an API key) are a beta limited to some organizations, so the
   existing `AGENTHQ_*` keys cannot be entered there. Claude supports OAuth 2.0 with Dynamic
   Client Registration (RFC 7591) or a Client ID Metadata Document (CIMD), PKCE S256, and
   refresh tokens.

What Claude's client requires, and where HQ stands:

| Requirement | HQ today |
|---|---|
| Streamable HTTP, `POST` with JSON responses | `POST /mcp` (`crates/hq-web/src/mcp_http.rs`). `GET` may answer 405. |
| `401` with `WWW-Authenticate: Bearer resource_metadata="..."` on an unauthenticated call. A header on a `200` is ignored. | A refusal is a `200` with a JSON-RPC error (`mcp_http.rs` `handle_mcp`). |
| Protected resource metadata (RFC 9728) whose `resource` equals the URL the user typed | Missing. |
| Authorization server metadata (RFC 8414) reachable from Anthropic's range | Missing. |
| DCR or CIMD, PKCE S256, form-urlencoded `/token`, refresh rotation, `invalid_grant` for a dead refresh token | Missing. |
| Callback `https://claude.ai/api/mcp/auth_callback` (and the announced `claude.com` variant); loopback on any port for Claude Code | Missing. |
| Discovery, registration and token answers in under 10 s, refresh under 30 s | Local SQLite, no issue. |

A side benefit: Claude Code supports the same OAuth flow, so a laptop could connect to a remote
HQ without a key in its config file.

## Decisions

- **HQ is its own authorization server.** HQ is single-owner and already has an owner credential
  (`web_auth_token`). An embedded, minimal OAuth server keeps the feature self-contained for
  anyone who clones the repo. Delegating to an external identity provider (Authelia, Keycloak,
  Pocket ID) is a possible later option for people who already run one, not the default.
- **The two-tool gateway stays.** The connector sees `hq_discover` and `hq_call`, as every other
  client does. It is token-light and works in any session. Claude's per-tool approval therefore
  sees one write-capable tool; HQ adds its own checks behind `hq_call` instead (below).
- **Full scope is allowed.** A grant can be full, read-only (Spark) or tasks, mapped onto the
  allowlists that already exist in `crates/hq-mcp/src/gateway.rs`. The handoff scope is not
  offered: full is a superset, and handoff exists for keys handed to other machines.
- **Prompt injection is not solved here.** A claude.ai conversation reads web pages, mail and
  files, and text in them can steer tool calls. Anthropic's model-side defences are the first
  layer. HQ's job is to shrink what a hijacked conversation can do and make it visible.
- **The consent page never needs to be public.** The browser that approves is the owner's, so the
  authorization page can live on a private origin (a tailnet name) while only the machine-facing
  endpoints are public. This needs a prototype before it is relied on (see Risks).

## Architecture

```
claude.ai (Anthropic servers, 160.79.104.0/21)
   │  POST /mcp, /oauth/token, /oauth/register, GET /.well-known/*
   ▼
public ingress (Caddy, Cloudflare Tunnel or Tailscale Funnel), only those paths
   │
   ▼
hq-web on 127.0.0.1:5678 ─── vault.db (clients, grants, hashed tokens, audit)
   ▲
   │  GET /oauth/authorize, /connect (PWA consent page), /api/oauth/*
owner's browser, on the private origin (tailnet) or the same public origin
```

### Endpoints

All new routes live in a new module `crates/hq-web/src/oauth/` and are registered in
`crates/hq-web/src/lib.rs` beside `/mcp`. When `mcp_connector.enabled` is false they are not
registered at all, so they 404 like any unknown path.

| Route | Caller | What it does |
|---|---|---|
| `GET /.well-known/oauth-protected-resource` and `.../oauth-protected-resource/mcp` | Claude | `{resource: "<public_url>/mcp", authorization_servers: ["<issuer>"], scopes_supported, bearer_methods_supported: ["header"]}` |
| `GET /.well-known/oauth-authorization-server` | Claude | Issuer, endpoints, `response_types_supported: ["code"]`, `grant_types_supported: ["authorization_code", "refresh_token"]`, `code_challenge_methods_supported: ["S256"]`, `token_endpoint_auth_methods_supported: ["none"]`, `client_id_metadata_document_supported: true`, `scopes_supported` including `offline_access` |
| `POST /oauth/register` | Claude | DCR for public clients. Refused unless the connector is armed. Every `redirect_uri` must pass the redirect rules. Caps the number of stored clients. |
| `GET /oauth/authorize` | owner's browser | Validates the request, stores it for 5 minutes under a random id, redirects to the PWA `/connect?request=<id>`. Never issues a code itself. |
| `GET /api/oauth/requests/{id}`, `POST .../approve`, `POST .../deny` | PWA | Behind the web token, the origin guard and `X-HQ-Client`. Approve returns the callback URL with `code` and `state`; the page navigates there with JS. |
| `POST /oauth/token` | Claude | `application/x-www-form-urlencoded`. `authorization_code` and `refresh_token` grants. |
| `POST /oauth/revoke` | Claude, owner | RFC 7009. |

The consent page uses a `fetch` and a JS navigation rather than a form post that redirects:
the CSP's `form-action 'self'` (`crates/hq-web/src/security_headers.rs`) would block a form
redirect to `claude.ai`. The PWA's `/api` calls already carry the web token and `X-HQ-Client`
through `hqFetch`.

`/oauth/*` and `/.well-known/*` are left outside the origin guard (`origin.rs`
`is_protected_path`): Claude calls them server to server with no `Origin`, and none of them
acts on ambient browser credentials. `/api/oauth/*` sits under `/api`, so the web token and the
guard already cover it. HQ Lite's route list (`profile_gate.rs`) gets the new routes only if
Lite should support connectors; the first cut leaves Lite out.

### Validation rules

- **Redirect URIs.** Exactly `https://claude.ai/api/mcp/auth_callback` and
  `https://claude.com/api/mcp/auth_callback`, plus `http://localhost:<any>/callback` and
  `http://127.0.0.1:<any>/callback` for Claude Code when `allow_loopback_redirects` is on.
  Configurable, but the default list is closed. Anything else is refused at registration and
  at authorization.
- **CIMD.** A `client_id` that is an https URL is fetched only if its host is in `cimd_hosts`
  (default: `claude.ai`). No redirects followed, 64 KiB cap, 5 s timeout, private and loopback
  addresses refused. This prevents the client id from being an SSRF lever.
- **PKCE.** `code_challenge` with `S256` is mandatory; `plain` is refused.
- **Resource.** If the request carries `resource` (RFC 8707) it must equal `<public_url>/mcp`.
  Tokens are stored with it and only accepted on `/mcp`.
- **Codes.** Single use, 60 seconds, bound to client, redirect URI, PKCE challenge, resource
  and the approved scope. A second use revokes the grant it produced.
- **Arming.** Registration and authorization work only inside an armed window that the owner
  opens from Settings or `hq connector arm` (default 10 minutes, stored in the database so the
  CLI and daemon agree). A stranger who learns the URL cannot start a flow, and a phishing link
  to an authorize URL is dead outside the window. Refreshing an existing grant does not need
  arming.

### Tokens and grants

- One **grant** per approval: client, scope, created by the owner at a time, label, last used.
- **Access tokens**: `hqo_` prefix, random, 1 hour, stored as SHA-256 like
  `crates/hq-db/src/session_tokens.rs` does for session secrets.
- **Refresh tokens**: `hqr_` prefix, rotated on every use, 30 day sliding window with a 90 day
  absolute cap. Presenting an already-rotated refresh token revokes the whole grant and notifies
  the owner (token theft signal). Errors use `invalid_grant`.
- **Revocation**: per grant from Settings and `hq connector revoke <id>`; `hq connector off`
  revokes everything and disarms.

### Wiring into `/mcp`

1. `handle_mcp` answers a missing or invalid credential with HTTP `401`. When the connector is
   enabled the response carries
   `WWW-Authenticate: Bearer resource_metadata="<public_url>/.well-known/oauth-protected-resource"`.
   The JSON-RPC error body stays, so key-based clients still see the reason. The `401` applies
   whether or not the connector is enabled, because it is the correct status; existing tests in
   `mcp_http.rs` that assert the refusal shape change with it.
2. Identity resolution gains a third source after the static keys and session tokens: an
   OAuth access token, which yields the grant's scope (`None`, `SPARK_READONLY_ALLOWLIST` or
   `TASKS_ALLOWLIST`) and the grant id for the checks below. `ApiIdentity` in
   `crates/hq-web/src/auth.rs` gets a `Connector { grant_id, scope }` variant, or the grant id
   travels beside it; settled during slice 1.
3. `initialize` echoes the client's `protocolVersion` when HQ supports it instead of the fixed
   `2024-11-05` (`mcp_http.rs` `initialize_result`). Claude Code already works with the fixed
   value; claude.ai has not been tested against it.

## Limiting the blast radius

None of these change the two-tool shape. They run inside `hq_call` for calls that arrive on a
connector grant, through the same private-marker pattern the gateway already uses for the
tasks and handoff scopes (`HANDOFF_SCOPE_ARG`, `TASKS_SCOPE_ARG`, `mark_spawned_session` in
`gateway.rs`), so a caller cannot forge or strip the marker.

1. **Owner confirmation for high-risk tools.** A short, named list
   (`CONNECTOR_CONFIRM_TOOLS` beside the allowlists, with a test that every entry is a real
   tool): starting, sending to or handing off harness sessions, `host_send`, sending mail through
   `gws`, `hq_ask` in `full` mode, self-update, skill and slash-command edits (they persist
   instructions into future sessions), and deletes. On a connector grant such a call raises an
   approval through `value_items`, which the owner approves from the chat relay button, the web
   inbox or `hq queue approve`. Those approvals are signed with the install key
   (`value_items::is_signed_approval`), so a tool cannot approve itself. The call waits up to
   about 45 seconds, then returns a pending result telling Claude to retry with the approval id.
   `mcp_connector.confirm_tools` can widen the list, or set it to `[]` to turn this off for an
   owner who accepts the risk.
2. **Spawned work stays bounded.** Sessions started through a connector grant are marked as
   spawned (`mark_spawned_session`), so they cannot start more, and their working directory
   must fall under `mcp_connector.spawn_cwd_allow` (default: `agent_host.handoff_cwd_allow`).
3. **Provenance on every write.** Tasks and comments get the author `mcp:connector/<grant
   label>`. `hq_ask` questions are already shown to later turns as an untrusted MCP-client
   message (`docs/MCP_ASK.md`); connector asks use the same label. Anything a connector writes is
   treated by HQ's own agent as external input, never as the owner speaking.
4. **Rate limits per grant.** Calls per minute and confirm-tier calls per day, with the counters
   in the database like `ask_requests` keeps its limits.
5. **Visibility.** A relay notice when a grant is created, on its first call, on every confirm-tier
   call and on refresh-token reuse. An audit table records grant, tool, time, outcome and a
   truncated argument digest; Settings shows the last calls per grant.
6. **Kill switch.** One button and `hq connector off`: revoke every grant, disarm, and stop
   answering the OAuth routes until re-enabled.

What remains, stated plainly: a full-scope grant can read the whole vault and act on anything
not on the confirm list, and whatever a tool returns is sent to Anthropic as part of the
conversation. Owners who keep sensitive notes in the vault should pick the tasks or read-only
scope, or not connect.

## Configuration

```yaml
mcp_connector:
  enabled: false
  public_url: https://hq.example.com          # origin Claude reaches; the resource is public_url + /mcp
  authorize_url: null                          # browser-only origin for the consent page, e.g. https://hq.example.ts.net; defaults to public_url
  default_scope: read_only                     # preselected on the consent page: read_only | tasks | full
  max_scope: full                              # the widest scope the consent page offers
  arm_minutes: 10
  access_token_ttl_secs: 3600
  refresh_token_days: 30
  allow_loopback_redirects: true               # Claude Code's OAuth flow
  extra_redirect_uris: []
  cimd_hosts: [claude.ai]
  confirm_tools: default                       # default | [] | a list of tool names
  spawn_cwd_allow: null                        # defaults to agent_host.handoff_cwd_allow
```

Startup refuses `enabled: true` when `web_auth_token` is unset (the consent page needs the
owner's identity), when `public_url` is not https, or when `authorize_url` is set and not https.

CLI, a new top-level command beside `hq mcp`: `hq connector status | arm [--minutes N] | grants
| revoke <id> | off`.

## Ingress

The PWA and `/api` stay off the public ingress in every option. Only these paths are published:
`/mcp`, `/oauth/register`, `/oauth/token`, `/oauth/revoke`, `/.well-known/oauth-*`, plus
`/oauth/authorize` and the PWA when `authorize_url` is not set.

- **Caddy on a VPS with a public name.** Path-scoped `handle` blocks and a `remote_ip`
  matcher for `160.79.104.0/21` on the machine-facing paths. The IP rule is defence in depth
  only: Anthropic can change the range, and OAuth is the real control. A sample
  `deploy/Caddyfile.connector` ships with the docs.
- **Cloudflare Tunnel.** For a box without a public address: a public hostname with the same
  path rules and a WAF rule on the source range.
- **Tailscale Funnel.** Publishes a tailnet service on the internet. Whether HQ can see the real
  client address behind it is unverified, so the IP rule may not be possible there.

Split origins: with `authorize_url` set to the tailnet name, the metadata advertises
`authorization_endpoint` on the private origin and everything else on the public one. The
owner's browser, on the tailnet, loads the consent page; Anthropic never can.

## Delivery slices

Each slice ends with a check that fails if it breaks.

0. **Prototype the unknowns.** A throwaway build on a test instance (the Hetzner demo box):
   does claude.ai accept an `authorization_endpoint` on a different origin from the token
   endpoint, which `protocolVersion` it sends, and whether it uses CIMD or DCR against HQ's
   metadata. Outcome recorded in this document before slice 3.
1. **401 and discovery.** `401` plus `WWW-Authenticate`, protected resource metadata, protocol
   version negotiation, config struct and startup validation. Tests in `mcp_http.rs` for the
   status, the header and every key still working.
2. **Storage.** Migration `087_mcp_connector.sql`: clients, pending requests, codes, grants,
   tokens (hashed), audit, armed window. `crates/hq-db/src/mcp_connector.rs` with unit tests for
   expiry, single-use codes, rotation and reuse detection.
3. **Authorization server.** Metadata, register, authorize, token, revoke; redirect, CIMD and
   PKCE rules; arming; `hq connector` CLI. Tests drive a full flow in process with a
   hand-written client, plus each refusal (wrong redirect, `plain` PKCE, reused code, unarmed
   registration, foreign CIMD host, rotated refresh token).
4. **Consent and management UI.** PWA `/connect` route and a "Claude connector" section in
   Settings: arm, grants with last use, revoke, kill switch. Checked in the browser on desktop
   and mobile, including the expired-request and unarmed states.
5. **Blast-radius checks.** Confirm tier through signed approvals, spawn marking and directory
   limit, provenance, rate limits, notifications, audit. Tests that a forged marker is
   stripped and that a confirm-tier call without a signed approval never runs.
6. **Docs and real-world proof.** `docs/CLAUDE_CONNECTOR.md`, updates to
   `docs/security/WEB_AUTH.md` and `CLAUDE.md` entry points, the Caddy sample. Then connect a
   real claude.ai account and Claude Code to the test instance and exercise discover, a read, a
   write, a confirm-tier call, revocation and the kill switch.

Security review (independent reviewers on authz, token handling and the consent flow) before
the PRs for slices 3 to 5, and again after fixes.

## Risks and open questions

- **Split origins** are allowed by RFC 8414 and not forbidden by Claude's connector docs, but
  not confirmed either. If claude.ai refuses them, the consent page must be public; arming and
  the web-token login then carry the protection.
- **Anthropic's egress range** can change; the IP rule is not a control to rely on.
- **Confirm-tier waits** depend on how long claude.ai waits for a tool result. Unknown; the
  pending-and-retry path covers a short limit.
- **Static headers** may open up to all accounts later. OAuth stays the better path (per-grant
  revocation, expiring tokens, no long-lived secret pasted into a web form), but a key-based
  option would then be a few lines of docs.
- **Data leaving the box.** Tool results become part of a claude.ai conversation under
  Anthropic's retention terms. The docs say so on the consent page and in the guide.

## Sources

- Authentication for connectors: https://claude.com/docs/connectors/building/authentication
- MCP tunnels and OAuth: https://claude.com/docs/connectors/mcp-tunnels/oauth
- MCP authorization specification: https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization
