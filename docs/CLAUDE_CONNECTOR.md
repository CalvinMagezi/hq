# Connecting HQ to claude.ai

HQ can be added to claude.ai (web, Desktop and mobile) and to Claude Code as a custom connector.
A Claude conversation then reaches the same two gateway tools every other MCP client gets,
`hq_discover` and `hq_call`, with an access level you choose when you approve it. It is off by
default. Design, threat model and test notes: `docs/plans/claude-connector.md`.

## What you need

- A Claude plan with custom connectors (Pro, Max, Team or Enterprise).
- HQ reachable from Anthropic's servers over public HTTPS. Connector traffic comes from
  Anthropic's network, not your browser, so a tailnet-only HQ cannot be added on its own.
- `web_auth_token` set, and a browser where you are signed in to HQ's web app: you approve each
  connection there.

## Set it up

1. **Choose the origins.**
   - `public_url`: the https origin Claude's servers reach, for example `https://hq.example.com`.
     Claude gets `https://hq.example.com/mcp` as the connector URL.
   - `authorize_url` (recommended): a private https origin for the consent page, such as your
     tailnet name (`https://hq.example.ts.net`). Your browser can load it; the internet cannot.
     HQ answers its sign-in page only on this origin, so nobody outside can see when a
     connection window is open or fill it with requests.
2. **Configure HQ** (`~/.hq/config.yaml` or `/opt/hq/config.yaml`) and restart it:

   ```yaml
   mcp_connector:
     enabled: true
     public_url: https://hq.example.com
     authorize_url: https://hq.example.ts.net
     default_scope: read_only        # preselected on the consent page: read_only | tasks | full
   ```

   If the connector cannot start (no web token, a URL that is not a bare https origin, or
   `profile: lite`), HQ logs why and leaves it off. `hq connector status` shows the reason.
3. **Publish only the machine endpoints**: `/mcp`, `/oauth/token`, `/oauth/revoke` and the three
   `/.well-known/oauth-*` documents. `deploy/Caddyfile.connector` does this with Caddy and also
   limits those paths to Anthropic's address range (defence in depth; the range can change). The
   web app, `/api` and `/ws` stay private. A Cloudflare Tunnel with the same path rules works on
   a machine with no public address; behind Cloudflare, filter on `Cf-Connecting-Ip` in a WAF
   rule rather than Caddy's `remote_ip`.
4. **Coding agents Claude starts** run in the HQ folder of the machine they run on
   (`~/Documents/HQ`, the same place HQ starts a session given no folder), on every host,
   with no setup. To let Claude use other project folders too, list them:

   ```yaml
   agent_host:
     handoff_cwd_allow: [/srv/projects]
   ```

   Anything outside the HQ folder and that list is refused.

## Connect

1. In HQ, open Settings, then Claude connector, and choose **Allow a new connection** (or run
   `hq connector arm`). The window stays open for ten minutes and admits one connection.
2. In Claude, open Settings, then Connectors, add a custom connector with the URL Settings shows,
   and press Connect. Claude detects the sign-in method on its own; leave its defaults.
3. Your browser opens HQ's consent page. Check that the request arrived when you pressed Connect,
   pick what Claude may do, name the connection, and approve.

For Claude Code, add the same URL as an HTTP server (`claude mcp add --transport http hq
https://hq.example.com/mcp`) and authenticate when it asks; its sign-in uses a local callback.

## What Claude can do

The access level you approve maps onto the gateway's existing scopes:

| Choice | Reaches |
|---|---|
| Read only | Search and read notes, tasks and session logs. No changes. |
| Tasks | Read and update tasks only. |
| Full | Every HQ tool, with the checks below. |

On top of the scope:

- **Your approval for high-risk calls.** Mail and other Google Workspace calls, remote MCP
  accounts, messages into your chat or an agent's mailbox, config changes, pairing a machine,
  reading or typing into a terminal pane, self-update, skill, slash-command and agent-definition
  edits, deleting a task, file reads and writes outside the vault (file edits, exports to a path,
  convert and OCR), commits, PRs and clones, and `hq_ask` in full mode each wait for you. HQ sends
  the exact call to your chat (and the web inbox) with Approve and Dismiss. One approval runs that
  call once, with exactly those arguments. Claude is told to call again after you approve. A
  request, and a dismissal, lasts a day.
- **Notes and tasks need no approval.** Claude can create, edit, move and trash notes, and
  create and update tasks (tags, assignees and all), without asking you. A deleted note goes to
  `_trash/` and stays there for 30 days. The exception is the vault's system folders: `skills/`,
  `_commands/` and `agents/` (they become instructions), and `_system/`, `_threads/`, `_data/`,
  `_trash/` and `_mailboxes/` (HQ's own identity, history and mail). A write, move or delete
  that touches one of those waits for you like the calls above. A task Claude files or updates
  keeps its tags and assignees but sends nobody a message: no agent mailbox and not your chat.
  An agent sees the task when it looks at its queue, never as something pushed at it.
- **Coding sessions are not gated**, so Claude can drive agents for you, but only in the host's
  HQ folder and `handoff_cwd_allow` folders, only the sessions it started (yours stay yours), and
  every one it starts sends you a notice with what it was told.
- **Provenance.** Tasks Claude writes are attributed to `mcp:connector/<connection name>`.
- **Limits and records.** 120 calls a minute per connection. Every call is logged (tool, outcome
  and a hash of its arguments, never the arguments) for 30 days: Settings shows the newest, and
  `hq connector log` lists them.

Two things to keep in mind. A Claude conversation reads web pages, mail and files, and text in
them can steer what Claude asks HQ to do; the checks above limit that, they do not make it
impossible. And whatever HQ's tools return becomes part of your Claude conversation, which
Anthropic stores under its own terms. If your vault holds things that must not leave it, choose
Tasks or Read only, or do not connect.

## Manage connections

- Settings, Claude connector: the connector URL, the window, connected clients (disconnect one),
  **Disconnect all**, and recent calls.
- `hq connector status | arm [--minutes N] | disarm | grants [--all] | revoke <id> | off | log`.
- `off` and **Disconnect all** revoke every connection, cancel approvals in flight and close the
  window. `mcp_connector.enabled: false` removes the endpoints entirely.

Tokens last an hour and refresh for up to 30 days, and a connection ends 90 days after you
approved it. A refresh token used twice ends its connection and tells you, since that means a
copy exists somewhere else.

## Troubleshooting

- **"HQ is not accepting new connections right now"**: the window is closed. Allow a new
  connection in Settings and press Connect again.
- **"This request has expired, was already answered, or was started in another browser"**:
  approve in the browser that Claude opened, within ten minutes.
- **Claude says it couldn't reach the server**: check that `https://<public_url>/mcp` answers
  `401` with a `WWW-Authenticate` header, and that `/.well-known/oauth-protected-resource` and
  `/.well-known/oauth-authorization-server` return JSON from outside your network.
- **"too many connection requests are waiting"** with a single public origin: someone is filling
  the window. Set `authorize_url` to a private origin.
