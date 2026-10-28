# Connecting HQ to Gemini Apps (read-only pilot)

Gemini Apps can add a custom MCP server by URL. HQ offers Gemini a separate, read-only profile
beside the claude.ai connector (`docs/CLAUDE_CONNECTOR.md`): its own versioned endpoint, its own
OAuth issuer, clients you create, and four tools that only read notes. Nothing about the Claude
connection changes when it is on. Design, audit and security review:
`docs/plans/gemini-connector.md`.

This has been tested in process only. The first run against a real Gemini account is the
runbook at the end of this page, and it needs your network changes first.

## What Gemini can do

| Tool | Does |
|---|---|
| `vault_search` | Search notes (at most 10 results a call) |
| `vault_list` | List the notes directly inside one folder |
| `vault_outline` | A note's headings |
| `vault_read` | Read a note, or a range of its lines |

Every tool is marked `readOnlyHint: true`, `destructiveHint: false`. HQ's own folders (any path
segment starting with `_`, such as `_system/`, `_threads/`, `_mailboxes/`, `_data/`), hidden
folders and anything that is not a plain relative path are refused, and search and list results
from them are dropped. No `hq_call`, no task, session, mail or config tools. A Gemini grant is
read-only whatever is picked on the consent page.

What these tools return enters Google's systems under Google's terms. Do not connect a vault
whose notes must not leave it.

## Set it up

1. Have the claude.ai connector configured (`mcp_connector.enabled`, `public_url`,
   `authorize_url`), then turn the profile on and restart HQ:

   ```yaml
   mcp_connector:
     enabled: true
     public_url: https://hq.example.com
     authorize_url: https://hq.example.ts.net
     gemini:
       enabled: true
   ```

2. Create a client. The secret is printed once and stored only as a hash:

   ```bash
   hq connector client add "Gemini on my phone"
   ```

   Keep the id and secret out of chats, tickets and config files. `hq connector client list`
   shows clients (never the secret); `hq connector client revoke <id>` ends the client and every
   connection it holds. To rotate, create a new client and revoke the old one.

3. `hq connector status` prints the MCP server URL for Gemini:
   `https://hq.example.com/mcp/gemini/v1`.

## Endpoints the profile adds

| Path | Purpose |
|---|---|
| `POST /mcp/gemini/v1` | MCP Streamable HTTP (JSON responses; `GET` is `405`) |
| `/.well-known/oauth-protected-resource/mcp/gemini/v1` | Resource metadata (RFC 9728), named in the `401` challenge |
| `/.well-known/oauth-authorization-server/gemini` | Authorization server metadata for issuer `https://hq.example.com/gemini` |
| `POST /oauth/register` | Advertised because Google's discovery asks for it; always refuses |

Authorization, token and revocation use the shared `/oauth/authorize`, `/oauth/token` and
`/oauth/revoke`. A Gemini client authenticates at the token endpoint with `client_secret_basic`
or `client_secret_post`; PKCE S256 is required; the code goes only to
`https://oauth-redirect{,-sandbox,-test}.googleusercontent.com/{r,a}/<id>`. Gemini keeps one
refresh token (Google's rule), so HQ extends it on use instead of rotating it; the grant still
ends 90 days after approval.

## Manage connections

Gemini grants appear with Claude's in Settings, `hq connector grants` and `hq connector log`,
named `Gemini (<client label>)`. `hq connector revoke <grant>` ends one, `hq connector off` ends
all of them (clients stay; revoke a client to stop its secret). Disconnecting in Gemini, or at
your Google Account's connections page, should call `/oauth/revoke`.

## Runbook: first manual run in Gemini Apps Developer Mode

Owner-approved steps first. None of them is done by HQ or by this change.

1. **Ingress.** Publish the four paths above on the public origin, next to the Claude ones.
   - Caddy: add them to the `@machine` matcher in your copy of `deploy/Caddyfile.connector`, in
     their own `handle` block whose `remote_ip` list is Google's user-triggered ranges, not
     Anthropic's. Google publishes the ranges at
     `https://developers.google.com/static/crawling/ipranges/user-triggered-fetchers-google.json`
     and `.../user-triggered-agents.json`; they change, so refresh them. Keep Claude's paths on
     Anthropic's range only.
   - HQ ending TLS itself: the door already admits these paths (`tls/gate.rs`) and passes a
     client's `Authorization: Basic` through on `/oauth/token` and `/oauth/revoke` only; the
     forwarder needs no change, and the fence, if any, is on the forwarder's host.
   - Rollback: remove the block; `mcp_connector.gemini.enabled: false` removes the routes.
2. **Check from outside** (any machine off your network):
   `curl -i -X POST https://hq.example.com/mcp/gemini/v1 -H 'content-type: application/json' -d '{}'`
   answers `401` with `resource_metadata=".../oauth-protected-resource/mcp/gemini/v1"`, and both
   well-known documents return JSON.
3. **Arm** in Settings or with `hq connector arm` (ten minutes, one connection).
4. **Add the app.** Open `https://gemini.google.com/devmode`, choose Create new, enter the MCP
   server URL, open Additional settings, Show more, and enter the client id and secret. Connect.
5. **Consent.** Your browser lands on HQ's consent page on the private origin. It should show
   `Gemini (<label>)` and a Google redirect host. Approve.
6. **Inspect** in Developer Mode: Tool Schemas lists the four tools with read-only annotations;
   run "search my notes for <topic>" and "read <a note path>"; Event Log shows each call and HQ's
   `hq connector log` shows the same calls.
7. **Refresh.** Leave it over an hour and run a tool again; it should keep working (a refresh).
8. **Revoke.** Disconnect the app in Gemini, then check `hq connector grants`. Also try
   `hq connector client revoke <id>` and confirm Gemini's next call fails.
9. **Claude unchanged.** Run a tool from your existing Claude connection before and after.

Record what Developer Mode actually did; these are the open questions:

- The redirect URI it sent. If it is not under Google's documented hosts, the consent step fails
  with "the redirect URI is not Google's" and nothing is issued.
- Which discovery document it fetched first (watch HQ's access log). If it reads only the root
  `/.well-known/oauth-authorization-server`, it gets Claude's document; the token endpoint there
  still serves a Gemini client, but if Gemini refuses that document a dedicated subdomain is the
  fallback.
- Whether it sent `resource`, PKCE, and which client authentication method.
- The source addresses its calls came from.

## Publishing (not needed for Developer Mode)

A personal connection in Developer Mode needs none of this. Listing HQ as a Google Connected App
for other people is a separate decision, all of it owner action in Google Connected Console:

1. Get Connected Console access for your Google account.
2. Verify the domain: Google gives a JSON challenge to serve at
   `https://<domain>/.well-known/google-connected-apps` with `200` and no redirects. HQ does not
   serve this path; Caddy (a `respond` for that exact path) or the door's path list would need
   it added first.
3. Public metadata: name, description, a square SVG logo of at least 500 px, and privacy policy
   and terms URLs.
4. Authentication setup in the console: the same client id and secret, and the endpoints above.
5. Testers, then review and publication. Tool schemas are snapshotted at registration, so any
   change to the tools goes on a new version path (`/mcp/gemini/v2`), never into v1.

## Troubleshooting

- **401 or 403 in the Event Log**: the token is not a live Gemini grant. A Claude token, an API
  key or a revoked grant is refused here. Check `hq connector grants`.
- **`invalid_client`** at the token endpoint: wrong or revoked secret, or the profile is off.
- **"This client cannot connect"** on authorize: the profile is off or the client is revoked.
- **A tool says the folder or note is not available**: it is in one of HQ's own folders.
- **"The result was too large to return"**: narrow the search or read a line range.
