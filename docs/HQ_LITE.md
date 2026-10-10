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
| `/mcp` (tools for editors and agents) | every tool | task tools only (no note tools, so no hidden folder can be read) |
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

## Native Windows

HQ Lite also builds for Windows without WSL2, virtualization or administrator rights. The build
is always Lite: a Windows `hq.exe` ignores `profile: full`, because the coding-agent host, the
sandbox and the chat relays are not part of it. For those, use Full HQ in WSL2
(`docs/WINDOWS.md`).

`irm https://agent-hq.online/install.ps1 | iex` installs it for you (see `docs/WINDOWS.md`). By hand: the `Windows Lite` workflow (it runs after every successful main release, so each release gets its zip, and can be run by hand) produces `hq-lite-<version>-windows-x86_64.zip` holding
`hq.exe`, a `web` folder with the app, and a SHA-256 file. Unzip it anywhere you can write, for
example `%LOCALAPPDATA%\hq-lite`, then:

```
.\hq.exe web             # the web app on this computer, opened in your browser
.\hq.exe task list       # the same tasks, from a terminal
.\hq.exe mcp install     # register HQ with VS Code
```

- The C runtime is linked in, so no Visual C++ redistributable (which needs an administrator) is
  needed. Nothing is installed as a service, and HQ's own files stay in your user profile (a project-scope `hq mcp install` writes into that project). On Windows those files rely on the profile's normal folder permissions.
- The download is signed with HQ's release key (minisign, the same key as the Linux releases), and
  the installer checks that before it runs anything. It is not signed with a Windows code-signing
  certificate (Authenticode). A computer that only allows approved
  programs (AppLocker, WDAC, Smart App Control) may refuse to run it; that is the policy working,
  and HQ does not try to get around it. Connecting VS Code to an HQ that runs elsewhere needs no
  program here at all (`docs/VPS_AGENT_CONNECT.md`).
- `hq update` looks for a newer Lite zip on GitHub (`--apply` downloads and runs `https://agent-hq.online/install.ps1` with PowerShell, which checks the signature and the SHA-256 and swaps the program; like the installer itself it does not go through your script-execution policy, so skip `--apply` and run the installer by hand if your policy matters here). It is a command you run, not a background updater. Your notes and tasks live
  in your user profile (`.hq`), not in the folder you unzipped to.
- `hq host`, `hq sessions` and pairing a machine answer that they need Full HQ.
- `hq autostart on|off|status` adds or removes one value under your user's startup key, so the web app
  starts when you sign in. It is off unless you turn it on, and security software sometimes flags
  startup entries.
- `hq uninstall lite` stops the server and removes that value; the program folder and your notes are
  yours to delete (a running program cannot delete itself). `hq lite export <folder>` first if you want
  your notes: the vault is plain markdown, so moving between Lite and Full HQ is a folder copy
  (`hq lite import <folder>` on the other side). Only files move, HQ's own top-level folders (`_…` and `.…`) stay behind, nothing is overwritten, and links are never followed.
- `hq doctor --windows` reads, and changes nothing: notes or database in a synced folder (OneDrive
  can corrupt a live SQLite database), path length, whether `gh`, `code` and `git` are on PATH, proxy
  settings and whether the port is free.
- A small, documented slice of the local REST API is described in `docs/api/lite-local-api.yaml`
  (OpenAPI). This repo is also a Scoop bucket (no administrator rights):
  `scoop bucket add hq https://github.com/CalvinMagezi/hq` then `scoop install hq/hq-lite`. The
  manifest in `bucket/` is pinned to one signed main build and is not updated automatically (Scoop
  checks the SHA-256, not the minisign signature; `install.ps1` checks both and always fetches the
  newest build). The pin only works while that release asset is unchanged (a rebuilt asset gives a
  hash error until the manifest is re-hashed). Update a Scoop install with `scoop update`, not
  `hq update --apply`, which would swap the program inside Scoop's folder behind its back.
  `packaging/` holds the templates for that and for winget, which is not submitted
  yet: it needs a stable release and a manual pull request to Microsoft's package repository.
- Each build's checksum file is signed by the protected release job with HQ's minisign key
  (`<zip>.sha256.minisig`). The installer verifies that signature with a small verifier it carries
  (Windows PowerShell 5.1 has no Ed25519, so it brings its own, tested against real minisign output
  in CI), then checks the zip against the signed SHA-256, then unpacks. A release with no signature
  is never offered, and there is no switch to skip the check. This proves the build came from HQ's
  release pipeline; it does not replace Authenticode, which is what AppLocker publisher rules and
  Smart App Control look for, and still needs a code-signing certificate. A signature does not stop
  someone who controls the release assets from serving you an older signed build (the signature carries no freshness).

## Nothing leaves unless you listed it

Commands that serve, run the daemon or could call another service (`hq web`, `hq start`, `hq
daemon`, `hq mcp-serve`, `hq reindex`, `hq memory`, `hq usage`, a plain `hq doctor` and others)
check the config first. `hq models` is not part of Lite. `hq doctor --egress`, `hq config`, `hq
env`, `hq mcp install`, `hq status`, `hq stop`, `hq search` and `hq vault` always run. Host and
update commands are handled before this check; see Limits below. If anything that can send your data elsewhere is
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

The Telegram and Discord relays are never enabled by listing: Lite has no chat relay, `hq start`
does not start one, and a relay token in the config is a refusal that only removing it clears.

To allow another one on purpose, name its id or its host:

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

## Linking a company Copilot seat

```
hq copilot link            # try the preferred models and print what to add to the config
hq copilot link --write    # add it to the config (the old file is kept as config.yaml.bak)
hq copilot link --model <id>
```

HQ uses GitHub's own CLI (`gh copilot`) as the model route, so it signs in the way the CLI does
and HQ never holds your GitHub token. Which models a seat offers depends on the plan and on what
your organization enabled, and HQ relies on no documented non-interactive model listing in `gh copilot`, so it uses
the model. So `link` sends one short test request ("reply with the single word: ok") per
model, in the order of `github_copilot.model_preference` (default `gpt-6-luna`, then
`claude-haiku-5.5`), and the first that answers wins. If neither does, it says so and stops; HQ
does not choose a bigger model for you, and a timeout or unrecognised error ends the check without
linking anything. Pass `--model` to name one yourself. The test runs in an empty temporary folder, so
no vault text or project file goes with it; only the one-word prompt does.

Lite itself does not run chat turns, so this is not what powers the web app. It sets up the Copilot
backend for a Full HQ on the same seat, and for the day Lite grows a tool-less chat. Whether
`claude-haiku-5.5` is offered in Copilot at all is unconfirmed; the live check is the answer.

## When MCP is blocked: the terminal

If your organization turns MCP off in Copilot but lets agents run terminal commands, the same
tasks and notes are reachable through the `hq` command. Nothing listens on a port, and nothing
leaves the machine.

```
hq task list --status in_progress
hq task create "Draft the Q4 plan" --priority high --due 2030-12-01
hq task comment PERSONAL-INBOX-001 "blocked on the budget numbers"
hq search budget --json
hq vault read Notebooks/plan.md --json
echo "text" | hq vault write Notebooks/new.md -
hq copilot init            # writes .github/copilot-instructions.md so agents know all this
hq copilot init --agents   # AGENTS.md instead
```

`hq task` goes through the same gateway as the tasks-only MCP key, so it has that key's limits: task
and space tools only, writes attributed to `mcp:tasks` (a lease from `hq task claim` or `hq task next`
is named `mcp:tasks/<name>`), no routing tags, assignees or notifications, no sessions. `hq task time`
and `hq task install-skill` run locally instead. Under Lite, `hq vault` and `hq search` hide `_system`, `_data` and the other folders the
web app hides, list no note that lives behind a symlink into them, and `hq vault context` is refused. Other commands (`hq memory`, `hq config`, `hq logs`) are local administration and are not filtered; the CLI is not a boundary against someone at your keyboard, who could point it at a Full config. Output is JSON on
standard output; logs go to standard error. A terminal agent is still an agent running commands on
your machine: Copilot's own approval prompts are the guardrail, and if your organization blocks MCP,
extensions and terminals alike there is no supported route, and HQ does not suggest working around it.

## Semantic search

Semantic search embeds each note's title and first 512 characters. Lite does no semantic indexing:
the scheduler task that does it is not kept, so nothing is sent to OpenRouter or a remote Ollama
for it, and keyword search (`hq search`, the search box) does not use vectors and works as before; the
keyword index is still kept up to date.

Under Lite the editor-facing tools (`/mcp` and `hq mcp-serve`) are the task and space tools only.
The note tools are not registered, because they would hand `_system` (your agent's memory and
settings) and the other hidden folders to whatever model the editor uses. Notes stay reachable
through the web app and `hq vault`, both of which hide those folders. A plain `hq mcp install`
under Lite writes the tasks scope.

## Connecting an editor

The tasks key gives an editor agent the task tools and nothing else; see
`docs/security/WEB_AUTH.md` and `docs/VPS_AGENT_CONNECT.md`. In Lite the full key is limited to the
same tools, because the others are not registered at all. For a local stdio entry the scope is a
flag in a file you can edit, so it limits what an editor agent is handed, not what a person on the
machine can do; only the server-side key is enforced, and under Lite even an edited entry reaches
task tools only. `hq logs` and `hq health` use Unix tools and are not available on Windows.

## Limits

Lite is not a sandbox. It narrows what the server offers and what it will send, and it is meant
for a machine where you decide what runs. Anyone who can run `hq` as you with a different config
can run the full profile. A misspelled key (`profil: lite`) is only warned about and leaves the
instance on the full profile, so check `hq doctor --egress` prints `profile: lite`.
