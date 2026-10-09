# HQ Lite

HQ Lite is a profile of HQ for a machine you do not fully control, such as a work laptop. It
serves the web app, your tasks and your notes, and it refuses to start while anything that sends
data to another service is configured, unless you list it. It is the same `hq` binary: you turn it
on with one setting.

```yaml
# ~/.hq/config.yaml
profile: lite
```

or set `HQ_PROFILE=lite`. The default is `full`, and an existing config stays on `full`. The
edition is fixed for the life of a process: a running daemon ignores a config change that flips it.

## What changes under `profile: lite`

| | Full | Lite |
|---|---|---|
| Web app: tasks, notes, search | yes | yes |
| `/mcp` (tools for editors and agents) | every tool | notes and tasks tools only |
| Chat (a reply that can run tools) | yes | refused over the socket; `hq chat` and bare `hq` refuse |
| Coding-agent sessions, Workbench, hosts (`hq sessions`, `hq host`, `hq agents`) | yes | not served; commands refuse; no tools registered |
| Shell, file and git tools, web search and fetch, image generation, email | yes | no tools registered |
| Settings, setup and admin routes, usage and budget panels, Gmail webhook | yes | not served |
| Notes under `_system`, `_data`, `_threads`, `_mailboxes` and other `_` or `.` folders | yes | not readable or writable through the web app |
| Scheduled daemon work | everything | housekeeping only (below) |
| Semantic indexing of notes | when a key or local Ollama exists | none; keyword search works |
| Relays (Telegram, Discord), remote MCP servers, self-update | when configured | refused at startup unless listed |

A route Lite does not serve answers with the same JSON 404 as a path that does not exist. The gate
is on the server, not in the web app. `/health` is unchanged, so it still reports the version.

The scheduler in Lite keeps six housekeeping tasks (`expire-approvals`, `vault-health`,
`thread-log-rotation`, `vault-cleanup`, `db-vacuum`, `vault-cap-enforcer`) and none of the startup
loops. It does not poll mail, embed notes, supervise agent sessions, ask a provider for usage or post
to a relay.

## Nothing leaves unless you listed it

Commands that serve, run the daemon or could call another service (`hq web`, `hq start`, `hq
daemon`, `hq mcp-serve`, `hq reindex`, `hq memory`, `hq models` and others) check the config first.
Diagnostics and configuration (`hq doctor`, `hq config`, `hq env`, `hq mcp install`, `hq status`,
`hq stop`, `hq search`, `hq vault`) always run. If anything that can send your data elsewhere is
set, a checked command stops and says what and how to clear it. The running daemon applies the same
check when it reloads the config file, and keeps the old config if the new one would be refused.

```
$ hq doctor --egress
Outbound destinations (profile: lite)
  OpenRouter  (openrouter)  [REFUSED in lite]
      hosts:   openrouter.ai
      carries: prompts, and note titles and excerpts sent for semantic search
```

`hq doctor --egress` prints every destination the config can reach, whatever the profile, and
exits non-zero under Lite when one is refused. It looks at model API keys (in the config or the
environment, including SiliconFlow and Novita and the `OPENROUTER_BASE_URL`, `TURBOQUANT_BASE_URL`
and `OLLAMA_HOST` overrides), `providers:` and `backends:` entries, a Telegram or Discord token
(whether or not the relay is enabled, since the disk watchdog and restart notices post with it),
`remote_mcp:` servers, `decisions:` routes, company email listeners and `gws` connectors,
`agent_host.hosts` and `agent_host.agent_mcp_url`, self-update, and the Copilot credit meter.
Destinations on this machine (`localhost`, `127.0.0.1`) are not listed.

To allow one on purpose, name its id or its host:

```yaml
lite:
  allow_egress: [telegram, api.example.com]   # a host also allows its subdomains
```

An item that reaches several hosts is allowed by its id, or when every host is listed. A host must
contain a dot (`api.example.com`); a bare word such as `ai` is read as an id and never as a whole
top-level domain.

`github-copilot-cli`, which runs GitHub's own CLI, is accepted without a listing: it is the route
meant for a company Copilot seat, and what it sends goes to GitHub under your seat's terms. The
`github-copilot-api` backend and the credit meter that goes with it sign in through GitHub
Copilot's internal token endpoint and present themselves as VS Code. GitHub does not document that
as a supported way for other software to use a seat, so Lite refuses them, and only
`lite.allow_unofficial_copilot: true` lifts that; listing `github.com` or the item id does not.

What this does not cover: `hq update` (the signed updater and its timer, which check GitHub and
`agent-hq.online`), `hq web --build` (which runs `bun install`), and a loopback endpoint that is
itself a proxy to a remote service. Switch those off where you need to.

## Semantic search

Semantic search embeds each note's title and first 512 characters. Lite does no semantic indexing:
the scheduler task that does it is not kept, so nothing is sent to OpenRouter or a remote Ollama
for it, and keyword search (`hq search`, the search box) does not use vectors and works as before.

## Connecting an editor

The tasks key gives an editor agent the task tools and nothing else; see
`docs/security/WEB_AUTH.md` and `docs/VPS_AGENT_CONNECT.md`. In Lite the full key is also limited
to notes and tasks, because the other tools are not registered at all.

## Limits

Lite is not a sandbox. It narrows what the server offers and what it will send, and it is meant
for a machine where you decide what runs. Anyone who can run `hq` as you with a different config
can run the full profile. A misspelled key (`profil: lite`) is only warned about and leaves the
instance on the full profile, so check `hq doctor --egress` prints `profile: lite`.
