# HQ Lite

HQ Lite is a profile of HQ for a machine you do not fully control, such as a work laptop. It
serves the web app, your tasks and your notes, and it refuses to start while anything that sends
data to another service is configured, unless you list it. It is the same `hq` binary: you turn it
on with one setting.

```yaml
# ~/.hq/config.yaml
profile: lite
```

or set `HQ_PROFILE=lite`. The default is `full`, and an existing config stays on `full`.

## What changes under `profile: lite`

| | Full | Lite |
|---|---|---|
| Web app: tasks, notes, search | yes | yes |
| `/mcp` (tools for editors and agents) | every tool | notes and tasks tools only |
| Chat (a reply that can run tools) | yes | refused over the socket |
| Coding-agent sessions, Workbench, hosts | yes | not served; no tools registered |
| Shell, file and git tools, web search and fetch, image generation, email | yes | no tools registered |
| Settings, setup and admin routes, usage and budget panels, Gmail webhook | yes | not served |
| Relays (Telegram, Discord), remote MCP servers, self-update | when configured | refused at startup unless listed |

A route Lite does not serve answers with the same JSON 404 as a path that does not exist, so a
client learns nothing about what is switched off. The gate is on the server, not in the web app.

## Nothing leaves unless you listed it

Before it serves, `hq web`, `hq start`, `hq mcp-serve` and `hq daemon` check the config. If
anything that can send your data elsewhere is set, they stop and say what and how to clear it:

```
$ hq doctor --egress
Outbound destinations (profile: lite)
  OpenRouter  (openrouter)  [REFUSED in lite]
      hosts:   openrouter.ai
      carries: prompts, and note titles and excerpts sent for semantic search
```

`hq doctor --egress` prints every destination the config can reach, whatever the profile, and
exits non-zero under Lite when one is refused. The things it looks at are model API keys (in the
config or the environment), `providers:` and `backends:` entries, the Telegram and Discord relays,
`remote_mcp:` servers, self-update, and the Copilot credit meter. Destinations on this machine
(`localhost`, `127.0.0.1`) are not listed.

To allow one on purpose, name its id or its host:

```yaml
lite:
  allow_egress: [telegram, api.example.com]   # a host also allows its subdomains
```

A host must contain a dot (`api.example.com`); a bare word such as `ai` is read as an id and never
as a whole top-level domain.

The `github-copilot-api` backend and the credit meter that goes with it sign in through GitHub
Copilot's internal token endpoint and present themselves as VS Code. GitHub does not document that
as a supported way for other software to use a seat, so Lite refuses them. `github-copilot-cli`,
which runs GitHub's own CLI, is accepted. `lite.allow_unofficial_copilot: true` lifts the refusal
for the first two.

## Semantic search

Semantic search embeds each note's title and first 512 characters. With an OpenRouter key set that
goes to OpenRouter, which is why a key is refused in Lite. Without one, HQ asks a local Ollama, and
if you have none, semantic search simply has no vectors; keyword search (`hq search`, the search
box) does not use them and works either way.

## Connecting an editor

The tasks key gives an editor agent the task tools and nothing else; see
`docs/security/WEB_AUTH.md` and `docs/VPS_AGENT_CONNECT.md`. In Lite the full key is also limited
to notes and tasks, because the other tools are not registered at all.

## Limits

Lite is not a sandbox. It narrows what the server offers and what it will send, and it is meant
for a machine where you decide what runs. Anyone who can run `hq` as you with a different config
can run the full profile.
