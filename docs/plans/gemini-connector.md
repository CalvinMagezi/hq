# Gemini Apps custom MCP profile

Status: implemented behind `mcp_connector.gemini.enabled` (off by default), not yet tested
against a real Gemini account. User-facing setup and the manual run: `docs/GEMINI_CONNECTOR.md`.

Gemini Apps can add a custom MCP server by URL (Developer Mode, `gemini.google.com/devmode`),
optionally with a pre-registered OAuth client id and secret. This plan adds a second, isolated
consumer profile to the existing claude.ai connector so the same HQ can be added to Gemini
without changing anything Claude sees.

## What exists (the Claude connector)

| Piece | Where | Tests |
|---|---|---|
| Config and checks | `crates/hq-core/src/config/mcp_connector.rs` (`McpConnectorConfig::settings`) | same file |
| Routes, AS metadata (RFC 8414), PRM (RFC 9728), `WWW-Authenticate` | `crates/hq-web/src/oauth/mod.rs` | `oauth/mod.rs` tests |
| Client identity: CIMD, fetched only from `claude.ai` | `crates/hq-web/src/oauth/cimd.rs` | same file |
| `/oauth/authorize`: consent origin only, armed window, PKCE S256, Claude callbacks | `crates/hq-web/src/oauth/authorize.rs` | `oauth/authorize/tests.rs` |
| Consent API: browser binder cookie, owner picks scope | `crates/hq-web/src/oauth/consent.rs` | `oauth/flow_tests.rs` |
| `/oauth/token`, `/oauth/revoke`: public client (`none`), one-time refresh tokens, reuse revokes | `crates/hq-web/src/oauth/token.rs` | `oauth/flow_tests.rs` |
| Store: requests, codes, grants, hashed tokens, audit | `crates/hq-db/src/mcp_connector.rs`, `sql/087`, `sql/089` | `mcp_connector/tests.rs` |
| `/mcp` Streamable HTTP (POST JSON, GET 405), grant to allowlist | `crates/hq-web/src/mcp_http.rs` | same file |
| Rate limit, owner approval for high-risk calls, call log | `crates/hq-web/src/oauth/guard.rs` | `oauth/guard/tests.rs` |
| Gateway tools and scope allowlists | `crates/hq-mcp/src/gateway.rs` | same file |
| TLS door path gate | `crates/hq-web/src/tls/gate.rs` (`CONNECTOR_PATHS`) | `tls/tests/` |
| Ingress fence (Anthropic CIDR) | `deploy/Caddyfile.connector`, the edge forwarder | none (deployment) |

Security boundary: the consent page is served only on a private origin; a request is bound to
the browser that started it; an arming window admits one approval; tokens are random, stored
as SHA-256 and prefixed for leak scanning; every call is rate limited, logged by argument hash,
and high-risk tools wait for the owner.

## Gaps against Google's documented requirements

| Google requirement (Connected Apps docs, checked 2026-10-28) | Claude path today | Gemini profile |
|---|---|---|
| Streamable HTTP, single POST endpoint, GET may be 405, MCP up to 2025-11-25 | Met | Same transport, own path `/mcp/gemini/v1` |
| Path-based versioning, schemas snapshotted at registration | `hq_call` catalog changes with the registry | Pinned v1 tool list with fixed schemas; a golden test fails on any change |
| Manual client id and secret, or DCR | CIMD only, `token_endpoint_auth_methods_supported: ["none"]` | Owner-created confidential clients (`hq connector client add`), `client_secret_basic` and `client_secret_post` |
| Discovery doc with `authorization_endpoint`, `token_endpoint`, `response_types_supported: ["code"]`, `registration_endpoint` | No `registration_endpoint` | Own issuer `<public>/gemini` with its own document; `registration_endpoint` refuses every registration |
| PKCE S256 for OAuth 2.1 | Required | Required |
| Redirects under `oauth-redirect{,-sandbox,-test}.googleusercontent.com/{r,a}/*` | Claude callbacks only | Exactly those hosts and prefixes |
| Do not rotate refresh tokens | Rotates; reuse after 60s revokes the grant | Same refresh token returned, expiry extended (capped by the grant's 90 days), client secret required |
| `readOnlyHint` on read tools; absent means destructive | Gateway tools carry no annotations | Every v1 tool `readOnlyHint: true`, `destructiveHint: false`, `openWorldHint: false`, `idempotentHint: true` |
| Avoid generic tools like `execute` | `hq_call` is one | Four named tools, no `hq_call` |
| Source IP allowlist (Google user-triggered fetcher and agent ranges) | Anthropic range in Caddy | Not changed; owner action (see runbook) |
| Domain verification `/.well-known/google-connected-apps` | n/a | Only for Connected Console publication; out of scope |

## Design

1. **Off by default, nested under the Claude connector.** `mcp_connector.gemini.enabled: true`
   adds the Gemini routes; without it none of them exist. Claude's metadata documents, redirect
   rules, CIMD lookup, token rotation and `/mcp` are unchanged, and golden tests pin them.
2. **Own issuer and resource.** Issuer `<public>/gemini`, metadata at
   `/.well-known/oauth-authorization-server/gemini` (RFC 8414 path form, the first URL Google
   tries for a path issuer). Resource `<public>/mcp/gemini/v1`, its PRM at
   `/.well-known/oauth-protected-resource/mcp/gemini/v1`, and its `401` points there. The
   authorization, token and revocation endpoints are the shared `/oauth/*` ones; they tell the
   profiles apart by client id.
3. **Registered clients.** Table `mcp_oauth_clients`: id (`hqcl_` + random), label, profile,
   SHA-256 of the secret, created and revoked times. The secret is printed once by the CLI and
   never written to config or logs. Revoking a client revokes every grant it holds.
4. **Profile on every row.** Requests, codes and grants get `profile TEXT NOT NULL DEFAULT
   'claude'` (migration 102), so existing rows stay Claude. `/mcp` refuses Gemini grants and
   `/mcp/gemini/v1` refuses everything but Gemini grants (no API keys, session tokens or dev
   switch).
5. **Scope.** A Gemini approval is stored as `read_only` whatever the consent page sends.
   Tools are a fixed list of four (`vault_search`, `vault_read`, `vault_list`, `vault_outline`),
   arguments are reduced to the pinned properties, and paths into system folders (any segment
   starting with `_`) are refused before the tool runs. Calls go through `guard::connector_call`,
   so the rate limit, call log and approval checks are the Claude ones.

## Security review points

- Non-rotating refresh tokens lose stolen-token detection. Mitigation: confidential client, so a
  refresh token alone is useless without the secret; grant still ends at 90 days; revocation by
  client, grant or `off`.
- The client secret is a long-lived credential held by Google. It is hashed at rest, shown once,
  and rotated by revoking the client and creating a new one.
- Search results can still name or quote notes outside the pinned paths if the index covers
  them. `vault_search` results are returned as the tool gives them; review before widening.
- Everything the tools return enters Google's systems under Google's terms.
- The redirect allowlist is the documented Google pattern. If Developer Mode uses another
  callback, the flow fails closed with "the redirect URI is not allowed"; widen only after
  seeing the real value.

## Externally gated (needs a live Gemini account, owner approval, or both)

- The exact redirect URI Developer Mode sends, and whether it sends `resource` and PKCE.
- Which discovery URL it fetches first (PRM from the `401`, the path-issuer document, or the
  root document). The root document is Claude's and is not changed; the shared token endpoint
  still serves a Gemini client that reads it.
- Whether `registration_endpoint` must exist when manual credentials are given.
- The token endpoint auth method it uses (both secret methods are accepted).
- Its source addresses, and the ingress change needed: Caddy or the edge must publish the new
  paths and admit Google's user-triggered ranges for them only.
