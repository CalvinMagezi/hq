# HQ as a Claude connector

Status: plan; slice 0 (prototype) done, findings below. Lets a person add their own HQ to claude.ai (web, Desktop, mobile)
as a custom connector, so a Claude conversation can reach the same `hq_discover` and `hq_call`
gateway that Claude Code already uses with a key. Off by default; nothing changes for an
instance that does not opt in.

## Feasibility

It works on Pro, Max, Team and Enterprise today, with two conditions HQ does not meet yet.

1. **HQ must be reachable from Anthropic's servers over public HTTPS.** Connector traffic comes
   from Anthropic's network (`160.79.104.0/21` at the time of writing), not from the user's
   browser. A tailnet-only instance cannot be added. Anthropic's private-network option (MCP
   tunnels) is Enterprise only and in research preview, so it is not a general answer.
2. **HQ must speak OAuth.** Static request headers (an API key) are a beta limited to some
   accounts: the dialog shows a "Request headers" section only where it is enabled, so most
   owners cannot paste an `AGENTHQ_*` key there. Claude supports OAuth 2.0 with a Client ID
   Metadata Document (CIMD) or Dynamic Client Registration (RFC 7591), PKCE S256, and refresh
   tokens. After probing the server the dialog shows what it detected ("Sign in now", "Use
   Claude's published identity") and lets the owner override it.

What Claude's client requires, and where HQ stands:

| Requirement | HQ today |
|---|---|
| Streamable HTTP, `POST` with JSON responses | `POST /mcp` (`crates/hq-web/src/mcp_http.rs`). `GET` may answer 405. |
| `401` with `WWW-Authenticate: Bearer resource_metadata="..."` on an unauthenticated call. A header on a `200` is ignored. | A refusal is a `200` with a JSON-RPC error (`mcp_http.rs` `handle_mcp`). |
| Protected resource metadata (RFC 9728) whose `resource` equals the URL the user typed | Missing. |
| Authorization server metadata (RFC 8414) reachable from Anthropic's range | Missing. |
| CIMD (or DCR), PKCE S256, form-urlencoded `/token`, refresh rotation, `invalid_grant` for a dead refresh token | Missing. |
| Callback `https://claude.ai/api/mcp/auth_callback` (and the announced `claude.com` variant); loopback on any port for Claude Code | Missing. |
| Discovery and token answers in under 10 s, refresh under 30 s | Local SQLite, no issue. |

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
  endpoints are public. Confirmed in slice 0.
- **CIMD only in the first cut.** claude.ai (web, Desktop, mobile) and Claude Code both identify
  themselves with a Client ID Metadata Document hosted on `claude.ai`, and claude.ai picks CIMD
  whenever the metadata offers it (slice 0). Dropping Dynamic Client Registration removes the
  registration endpoint, the stored-client table and its cap. DCR can be added later if an
  owner needs a client that only does DCR.

## Architecture

```
claude.ai (Anthropic servers, 160.79.104.0/21)
   │  POST /mcp, /oauth/token, GET /.well-known/*
   ▼
public ingress (Caddy, Cloudflare Tunnel or Tailscale Funnel), only those paths
   │
   ▼
hq-web on 127.0.0.1:5678 ─── vault.db (grants, hashed tokens, audit)
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
| `GET /.well-known/oauth-protected-resource` and `.../oauth-protected-resource/mcp` | Claude | `{resource: "<public_url>/mcp", authorization_servers: ["<issuer>"], scopes_supported: ["hq"], bearer_methods_supported: ["header"]}`. One scope only: Claude requests every advertised scope, so the grant's real scope is picked on the consent page. |
| `GET /.well-known/oauth-authorization-server` | Claude | Issuer, endpoints, `response_types_supported: ["code"]`, `grant_types_supported: ["authorization_code", "refresh_token"]`, `code_challenge_methods_supported: ["S256"]`, `token_endpoint_auth_methods_supported: ["none"]`, `client_id_metadata_document_supported: true`, `scopes_supported` including `offline_access` |
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
  `http://127.0.0.1:<any>/callback` for Claude Code. The list is fixed in code. Anything else is refused at authorization.
- **CIMD.** The `client_id` must be an https URL whose host is in `cimd_hosts` (default:
  `claude.ai`; claude.ai uses `https://claude.ai/oauth/mcp-oauth-client-metadata`, Claude Code
  `https://claude.ai/oauth/claude-code-client-metadata`). No redirects followed, 64 KiB cap, 5 s
  timeout, private and loopback addresses refused, so the client id is not an SSRF lever. The
  fetch sends an explicit `User-Agent`: claude.ai's edge answers `403` to a library default
  (Python's was refused in slice 0; reqwest sends none unless told to). The document's
  `client_id` must equal the URL, and the `redirect_uri` must appear in its `redirect_uris` and
  pass the rule above. Documents are cached for an hour.
- **PKCE.** `code_challenge` with `S256` is mandatory; `plain` is refused.
- **Resource.** If the request carries `resource` (RFC 8707) it must equal `<public_url>/mcp`.
  Tokens are stored with it and only accepted on `/mcp`.
- **Codes.** Single use, 60 seconds, bound to client, redirect URI, PKCE challenge, resource
  and the approved scope. A second use revokes the grant it produced.
- **Arming.** Authorization works only inside an armed window that the owner opens from Settings or `hq connector arm` (default 10 minutes, stored in the database so the
  CLI and daemon agree). A stranger who learns the URL cannot start a flow, and a phishing link
  to an authorize URL is dead outside the window. Refreshing an existing grant does not need
  arming. Approving closes the window (one approval per arming), closing it drops waiting
  requests, and at most 20 requests can wait at once. A request is approvable only in the
  window it arrived in.
- **Request binding.** `/oauth/authorize` sets an HttpOnly, Secure, `SameSite=Strict` cookie
  scoped to `/api/oauth/requests` holding a random binder, stored hashed on the request. The
  consent API answers only the browser holding it, so a request started in someone else's
  browser cannot be shown to, or approved by, the owner. A link to the authorize URL itself
  still creates a request in whoever opens it; the consent page shows when the request arrived
  and says to approve only right after pressing Connect, and a loopback redirect shows its port.
- **Consent origin only.** `/oauth/authorize` answers only when the request's `Host` is the
  consent origin, so with `authorize_url` set the public name never reveals the window or takes
  requests, whatever the proxy forwards. With a single public origin, someone who knows the
  URL can watch for the window and fill its 20 request slots, which keeps the owner from
  connecting while they keep it up; a private `authorize_url` is the fix, and the docs say so.

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
   `mcp_http.rs` that assert the refusal shape change with it. `deploy/test-vps-connection.sh`
   and `hq mcp doctor` are checked for anything that expects a `200` refusal.
2. Identity resolution gains a third source after the static keys and session tokens: an
   OAuth access token, which yields the grant's scope (`None`, `SPARK_READONLY_ALLOWLIST` or
   `TASKS_ALLOWLIST`) and the grant id for the checks below. `ApiIdentity` in
   `crates/hq-web/src/auth.rs` gets a `Connector { grant_id, scope }` variant, or the grant id
   travels beside it; settled during slice 1.
3. `initialize` echoes the client's `protocolVersion` when HQ supports it instead of the fixed
   `2024-11-05` (`mcp_http.rs` `initialize_result`). claude.ai asks for `2025-11-25`.
4. Unknown methods keep answering JSON-RPC `-32601`. claude.ai first sends `server/discover`
   (header `MCP-Protocol-Version: 2026-07-28`) and falls back to `initialize` on `-32601`, so
   that answer is load-bearing. Params may carry `_meta` (a `traceparent` on `tools/call`),
   which HQ ignores.

## Limiting the blast radius

None of these change the two-tool shape. They run inside `hq_call` for calls that arrive on a
connector grant, through the same private-marker pattern the gateway already uses for the
tasks and handoff scopes (`HANDOFF_SCOPE_ARG`, `TASKS_SCOPE_ARG`, `mark_spawned_session` in
`gateway.rs`), so a caller cannot forge or strip the marker.

1. **Owner confirmation for high-risk tools.** `CONNECTOR_CONFIRM_TOOLS` in
   `crates/hq-mcp/src/gateway.rs` (a registry test checks every name is a served tool): Google
   Workspace, `agent_send_message` (it reaches the owner's chat or an agent that acts on it),
   `config_manage`, `host_add`, `host_send` and `host_read` (any pane, including the owner's),
   self-update install and
   rollback, skill and slash-command edits (they persist instructions), task and note deletes,
   file edits, commits, PRs and clones on the HQ host, and the convert and OCR tools (they read
   and write host paths). Some count only by their arguments: a vault export with `output`
   (it writes anywhere the HQ user can), a note write or move into `skills/`, `_commands/` or
   `Agents/` (they become instructions), and a task call that sets tags or assignees (they put
   text in a mailbox the owner's chat or an agent drains, the reach `agent_send_message` has).
   Every remote MCP server's tools count too (they act on the owner's accounts), and `hq_ask` in
   `full` mode, read by the tool's own parser (otherwise HQ's own agent is a way around the
   list). Harness session
   tools are deliberately not on it: Claude driving coding sessions is much of the point. The
   consequence, stated plainly: a spawned coding agent runs with permissions skipped and a
   shell on its host, so the list stops a single injected instruction from reaching mail or a
   delete directly, but it is not a wall. Items 2 and 5 are the controls there.

   The check runs in `crates/hq-web/src/oauth/guard.rs`, only for tools inside the grant's
   scope. It files an `ActionNeeded` value item and sends it to the owner's chat at once as an
   interrupt (the value bus's learned batching may hold or drop other notices, and its cycle is
   five minutes), with Approve and Dismiss buttons; the web inbox and `hq queue approve` work
   too. Approvals are signed with the install key (`value_items::is_signed_approval`), so a
   tool cannot approve itself. An approval is:
   - bound to the grant, the tool and a SHA-256 of the canonical arguments (keys sorted, gateway
     markers left out), so it cannot be replayed on other arguments;
   - spent by the one call it lets through (`value_items::consume_approval`);
   - shown to the owner with the real arguments. Arguments too long for a chat message (3000
     characters) are refused rather than shown cut.

   The call waits 40 seconds, then answers "pending" and asks Claude to call again with the same
   tool and arguments. A request, and a decline, lasts a day; after that the same call asks
   again. One connection may have five requests waiting at once. There is no setting to change
   the list: an owner who wants to skip a check uses a key instead of the connector.
2. **Spawned work stays bounded.** A session HQ launches gets its own session token, so its calls
   back to HQ are refused further spawns. A session started on a connector grant must run in the HQ folder of
   the host it runs on (`Documents/HQ`, as that host resolves it) or under
   `agent_host.handoff_cwd_allow` (`connector_folders` and `CwdBound::Connector` in
   `harness_session/cwd.rs`, checked before any folder is made). A host with no HQ folder and an
   empty list starts no connector session at all. An owner who wants Claude in other project
   folders lists them there. Those sessions
   are recorded with origin `connector` (only when a session actually starts, never for one a
   handoff found already working), count against `agent_host.max_mcp_started_sessions`, do not
   start out driven, and are the only ones a connector may send to, resume, stop, re-goal, link
   or read logs from: the owner's own sessions stay theirs. A task a connector files from a note
   routes nowhere.
3. **Provenance on every write.** Task writes from a connector are attributed to
   `mcp:connector/<grant label>` whatever name the call passes, a lease taken on one is named
   the same way, and no other caller may write under that prefix. `hq_ask` questions are already
   shown to later turns as an untrusted MCP-client message (`docs/MCP_ASK.md`).
4. **Rate limit per grant.** 120 calls a minute, counted from the call log; a count that cannot be
   read refuses the call.
5. **Visibility.** Immediate interrupt notices to the owner's chat when a grant is created, when a
   replayed refresh token revokes one, and for every coding session a connector starts (with the
   session, host, folder and every field that reaches the agent as instructions; a long text is
   cut with a note saying where the rest is).
   The call log (`mcp_connector_audit`) keeps grant, tool, outcome and a SHA-256 of the
   arguments, never the arguments, for 30 days. Settings shows the newest calls and `hq
   connector log` lists them.
6. **Kill switch.** One button and `hq connector off`: revoke every grant, cancel approved codes
   not yet exchanged and waiting requests, and disarm, in one transaction. With no
   grant and no armed window, nothing can connect until the owner arms again; `enabled: false`
   removes the routes entirely.

What remains, stated plainly: a full-scope grant can read the whole vault, list directories and
read git history on the HQ host, and act on anything not on the confirm list, and whatever a tool returns is sent to Anthropic as part of the
conversation. Owners who keep sensitive notes in the vault should pick the tasks or read-only
scope, or not connect.

## Configuration

```yaml
mcp_connector:
  enabled: false
  public_url: https://hq.example.com          # origin Claude reaches; the resource is public_url + /mcp
  authorize_url: null                          # browser-only origin for the consent page, e.g. https://hq.example.ts.net; defaults to public_url
  default_scope: read_only                     # preselected on the consent page: read_only | tasks | full
```

Coding sessions a connector starts use `agent_host.handoff_cwd_allow`; see item 2 above.

Token lifetimes (1 hour access, 30 day refresh, 90 day cap), the arming window (10 minutes, `hq
connector arm --minutes` overrides it), the redirect list and the CIMD hosts are constants, not
settings: none of them is a choice an owner should have to make. Fields are added in the slice
that first reads them.

The connector stays off, with an error at startup, when `web_auth_token` is unset (the consent
page needs the owner's identity), when `public_url` or `authorize_url` is not a bare https
origin, or under `profile: lite`. The rest of the web server starts normally. The consent page
calls `/api/oauth/*` from its own origin, which the origin guard already accepts as same-origin
(the `Host` matches), so `web_allowed_origins` needs no entry for it.

CLI, a new top-level command beside `hq mcp`: `hq connector status | arm [--minutes N] | disarm | log | grants
| revoke <id> | off`.

## Ingress

Machine-facing paths published to the internet: `/mcp`, `/oauth/token`, `/oauth/revoke` and
`/.well-known/oauth-*`.

With `authorize_url` set (recommended), nothing else is public: the consent flow
(`/oauth/authorize`, `/connect`, `/assets/*`, `/api/oauth/*`) runs on the private origin. Without
it, those four also have to be public, which puts the PWA shell and the web-token login on the
internet. Every other `/api` route and `/ws` stay private in both cases.

- **Caddy on a VPS with a public name.** Path-scoped `handle` blocks and a `remote_ip`
  matcher for `160.79.104.0/21` on the machine-facing paths (slice 0 saw every Anthropic call,
  discovery, token and MCP alike, come from inside it). The IP rule is defence in depth only:
  Anthropic can change the range, and OAuth is the real control. A sample
  `deploy/Caddyfile.connector` ships with the docs.
- **Cloudflare Tunnel.** For a box without a public address: a public hostname with the same
  path rules and a WAF rule on the source range. The client address arrives in
  `Cf-Connecting-Ip`. Slice 0 ran through a Cloudflare quick tunnel without trouble.
- **Tailscale Funnel.** Publishes a tailnet service on the internet. Whether HQ can see the real
  client address behind it is unverified, so the IP rule may not be possible there.

Split origins: with `authorize_url` set to the tailnet name, the metadata advertises
`authorization_endpoint` on the private origin and everything else on the public one. The
owner's browser, on the tailnet, loads the consent page; Anthropic never can.

## Delivery slices

Each slice ends with a check that fails if it breaks.

0. **Prototype the unknowns.** Done; see "Slice 0 findings".
1. **401 and discovery.** `401` plus `WWW-Authenticate`, protected resource metadata, protocol
   version negotiation, config struct and startup validation. Tests in `mcp_http.rs` for the
   status, the header and every key still working.
2. **Storage.** Migration `087_mcp_connector.sql`: pending requests, codes, grants,
   tokens (hashed), audit, armed window. `crates/hq-db/src/mcp_connector.rs` with unit tests for
   expiry, single-use codes, rotation and reuse detection.
3. **Authorization server.** Metadata, authorize, token, revoke; redirect, CIMD and PKCE rules;
   arming; `hq connector` CLI. Tests drive a full flow in process with a hand-written client
   that replays the requests recorded in slice 0, plus each refusal (wrong redirect, `plain`
   PKCE, reused code, unarmed authorize, foreign CIMD host, rotated refresh token).
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

- **Decided: harness sessions are not on the confirm list.** The directory allowlist and a
  notice on every connector-started session bound them. A later option, if needed: force
  connector-started sessions into `governance.bash.sandbox: required`.
- **Defaults differ from a personal instance.** The shipped default scope is `read_only`; an
  owner who wants Claude as a full orchestrator sets `default_scope: full`.
- **Anthropic's egress range** can change; the IP rule is not a control to rely on.
- **Accepted trade-offs from the security review.** A refresh token presented again within 60
  seconds of its use is refused without revoking the grant, so a client retrying a refresh
  does not lose its connection; a thief inside that minute is refused too, and slice 5 tells the
  owner. `/oauth/authorize` answers differently when armed, so someone who knows the URL can
  tell when the window is open; they still need the owner's browser to approve.
- **Confirm-tier waits** depend on how long claude.ai waits for a tool result. Unknown; the
  pending-and-retry path covers a short limit.
- **Static headers** may open up to all accounts later. OAuth stays the better path (per-grant
  revocation, expiring tokens, no long-lived secret pasted into a web form), but a key-based
  option would then be a few lines of docs.
- **Data leaving the box.** Tool results become part of a claude.ai conversation under
  Anthropic's retention terms. The docs say so on the consent page and in the guide.

## Slice 0 findings

Run on 2026-10-12 with a throwaway stub (Python, two listeners on loopback): the public side
behind a Cloudflare quick tunnel served `/mcp`, `/.well-known/*`, `/register` and `/token`; the
consent side behind `tailscale serve` (tailnet only) served `/authorize`. The stub advertised
CIMD and DCR, echoed the protocol version, and issued 2 minute access tokens to force a refresh.
A real claude.ai account added it, connected, listed the tool and called it.

- **Split origins work.** claude.ai read the metadata on the public origin and sent the owner's
  browser to `authorization_endpoint` on the tailnet origin. The code came back through
  `https://claude.ai/api/mcp/auth_callback` and the token exchange succeeded.
- **CIMD is chosen when offered.** `client_id` was `https://claude.ai/oauth/mcp-oauth-client-metadata`.
  Its document lists one redirect URI (`https://claude.ai/api/mcp/auth_callback`), grant types
  `authorization_code`, `refresh_token` and `urn:ietf:params:oauth:grant-type:jwt-bearer`, and
  `token_endpoint_auth_method: none`. `/register` was never called.
- **The CIMD fetch needs a real User-Agent.** claude.ai answered `403` to Python's default agent
  and `200` to an explicit one. The first connect attempt failed on this.
- **Authorize request:** `response_type=code`, PKCE S256, `state`, `scope` = every entry of
  `scopes_supported` plus `offline_access`, `resource` = the exact `/mcp` URL, `prompt=consent`.
- **Token exchange:** form-urlencoded, `client_id`, `code_verifier` (128 characters),
  `redirect_uri` and `resource`. PKCE, client and redirect all matched.
- **Refresh:** proactive, before expiry, with `client_id` and `resource` and no `scope`.
  Rotation was honoured: the next calls used the new tokens.
- **MCP:** `server/discover` first (`MCP-Protocol-Version: 2026-07-28`); on `-32601` it falls
  back to `initialize` with `2025-11-25`, then `notifications/initialized` (`202`) and
  `tools/list`. Two client identities connect (`Anthropic/Toolbox` and `Anthropic/ClaudeAI`).
  No `GET /mcp`, no `Mcp-Session-Id`, no batches. The `readOnlyHint` annotation put the tool in
  claude.ai's "Read-only tools" group.
- **Network:** every Anthropic request came from `160.79.106.x`. Discovery and token calls use
  `python-httpx`; MCP calls use the agent `Claude-User`.
- **The connector also appears in Claude Desktop's Code sessions** on the same account, so a
  desktop coding session can call HQ through it with no key in its config.
- **Not tested:** DCR (not needed in the first cut) and HQ's current fixed `2024-11-05` answer
  (slice 1 replaces it with an echo).

## End-to-end run

On 2026-10-12, a build of slices 1 to 5 ran on a test instance with a throwaway vault: the public
side behind a Cloudflare quick tunnel, the consent page on a tailnet origin. A real claude.ai
account added it as a custom connector.

- The connect flow ran as designed: Claude reached the tailnet consent page through the
  metadata, an approval that arrived after the window closed was refused ("no longer accepting
  connections"), and after re-arming the owner approved Full access and Claude connected.
  Approving closed the window and sent the "Claude is connected to HQ" notice.
- A claude.ai chat then ran `hq_discover`, read a note, and filed a task, which HQ attributed to
  `mcp:connector/Claude` though Claude passed another name. Its `vault_delete` came back
  "waiting for the owner"; the inbox showed the exact path; after the owner approved, the
  identical call ran and moved the note to `_trash/`. The call log held every call and outcome.
- **Disconnect all** revoked the grant, and Claude's next call ended in "Reconnect a connector to
  keep using it".

It found three defects, all fixed in the same PR: the PWA's service worker answered
`/oauth/authorize` with the cached app shell (the redirect looks like a failed response to a
worker), Settings kept showing an expired window as open, and the test server needed a restart
after a web rebuild (the content-security-policy hashes are read at startup, as documented).
`hq_discover` with no filter returns about 126k characters to Claude, which it truncates; that
affects every client and is tracked separately.

## Sources

- Authentication for connectors: https://claude.com/docs/connectors/building/authentication
- MCP tunnels and OAuth: https://claude.com/docs/connectors/mcp-tunnels/oauth
- MCP authorization specification: https://modelcontextprotocol.io/specification/2025-11-25/basic/authorization
