# CLAUDE.md

Instructions for AI coding assistants working on this repository. Humans should
start with `README.md` and `CONTRIBUTING.md`.

## What This Is

Agent-HQ: a Rust-based, local-first AI agent hub. One `hq` binary runs the CLI,
the daemon, the chat relays (Discord, Telegram), a web UI and an MCP server.
Data lives in `.vault/` (a markdown vault, gitignored).

## Working agreements

Treat every change as something a stranger will clone, build and run. Make no
assumptions about a specific machine, a specific person's paths, or private
services being available. Config needs sane generic defaults; anything
instance-specific (personal API endpoints, a particular deploy target) belongs
behind an explicit opt-in, never a hardcoded default. Never put real hostnames,
tokens, chat ids or personal names in code, docs, tests or fixtures; use
placeholders (`example.com`, `hq.example.ts.net`, `<owner>/<repo>`).

**The bar is: it works when run, and the gates below are clean.**

- `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
  and the web build (`cd apps/hq-web && bun install && bun run build`) must pass
  before a PR is opened.
- Tests that need external state go behind `#[ignore]` with a reason and a
  documented opt-in command: a real vault, Ollama, an authenticated harness
  binary, the network. They are diagnostics, not regression gates.
- CI fails on unformatted Rust. Run `./scripts/install-hooks.sh` at the start of
  a session (fresh clones, cloud sessions included, don't have it): the
  pre-commit hook then rustfmts staged `.rs` files on every commit. Without it,
  run `cargo fmt --all` before committing.
- `cargo test --workspace` aborts at the first failing test binary, so one bad
  test masks every crate after it. A partial run is not a green run: check the
  totals, not the exit of the first suite.
- Work on a branch and open a pull request. Keep changes focused and atomic,
  write tests for new behavior, and follow the style of the surrounding code.
- Do not hand-deploy to a server. Servers update themselves from signed releases
  (`docs/UPDATE_SYSTEM.md`, `deploy/README.md`).

## Architecture

- **Language**: Rust (edition 2024, Cargo workspace of 17 crates)
- **Binary**: `hq`, built from `crates/hq-cli`. `scripts/install-hq.sh` installs it
  to `~/bin/hq`; servers use `/usr/local/bin/hq`.
- **Config**: `~/.hq/config.yaml` (override with `HQ_CONFIG_PATH`); any key can
  also come from `HQ_`-prefixed env vars with `__` for nesting.
- **Vault**: `.vault/` (markdown plus YAML frontmatter)
- **DB**: `.vault/_data/vault.db` (single SQLite file, WAL mode)
- **LLM**: OpenRouter via `async-openai` with a base URL swap; Ollama and other
  OpenAI-compatible backends via `backends:` config

## Rules

- All source lives in `crates/`, with standard Rust conventions (snake_case).
- `.vault/` is the center: shared memory, context engine, knowledge base.
- Frontmatter is parsed with the `gray_matter` crate.
- **Execution**: sub-agents run through the unified `spawn_subagents` tool
  (`SpawnSubagentsTool`) over `AgentService` (single, parallel, race and graph
  modes, in process). It is the only sub-agent substrate.
  `agent_send_message` and `agent_read_inbox` are plain vault-mailbox tools for
  handing text to `relay` or another local mailbox id.
- **Coding-agent runtime**: long-lived coding agents (Claude Code, Codex, Cursor,
  Pi, OpenCode, Copilot CLI, Kimi, Qwen, Antigravity) run in HQ's built-in host
  (`hq host`, crate `hq-host`). General-purpose agents (Hermes, OpenClaw) run
  there too as a second harness category
  (`crates/hq-tools/src/harness_session/category.rs`): every session has a
  `purpose` (`code`, `monitor`, `research`, `review`, `general`), a
  general-purpose agent refuses `code`, a monitor or reviewer never moves its
  task, and one given sessions to `observe` hears their finished turns, blocks
  and exits through the task-thread messages the supervisor delivers. They get
  no HQ token. The **Agentic Stack** (`stack:` config,
  `crates/hq-tools/src/harness_session/stack.rs`, daemon task `stack-dispatch`,
  `hq stack`) names such agents as specialists that pull the tasks assigned to
  them and run each as one one-shot job (`hermes chat --oneshot -q "<brief>"`); the
  process ending puts the task into review (`mission::Event::RunEnded`). A pane
  whose process ended counts as gone (`AgentInfo::exited`). "General-purpose
  agents" in `docs/AGENT_SESSIONS.md`, plan in `docs/plans/agentic-stack.md`. `crates/hq-tools/src/agent_host/`
  (to be renamed) holds the client: `NativeBackend` talks to the host on this
  machine, or to one on another machine over SSH through a key pinned to
  `hq host gate`. Agents run in a process sandbox with an egress allowlist.
  `hq host join` / `hq host add` / the `host_add` tool pair a machine.
  `harness_session_*`, the read-only `host_list`/`host_agents`/`host_read`
  tools and `host_send` sit on top. The host sends state changes as events; the
  supervisor treats an unreachable host as "unknown" (never "exited"), and alerts
  when an agent blocks. A coding agent's trust dialog defaults to "No, exit", so
  blocked launches are reported, never auto-answered. Setup, security model and
  pairing a machine: `docs/AGENT_HOST.md`, `docs/JOIN_A_MACHINE.md`.
- **No job queue**: the filesystem job queue (`_jobs/`) is retired. All work
  dispatches through sub-agents or relay notifications.
- **Long relay turns**: turns that outlive the ack window
  (`relay.turn_ack_timeout_secs`, default 270s) detach into the persisted
  `background_turns` registry and deliver results asynchronously. A startup and
  6-hourly reconciler interrupts stranded rows past
  `relay.background_turn_max_days` (default 5), resumable via `resume <id>`.
  Detached turns post progress heartbeats (`relay.background_progress_secs`,
  default 300) and the agent can volunteer updates via `report_progress`.
  `/watch <minutes> [for <N>h] <prompt>` schedules durable recurring turns
  (restart-safe); `/unwatch <id>` stops one. The agent can do the same with
  `watch_create`/`watch_list`/`watch_cancel` (`crates/hq-tools/src/background_turns.rs`).
  A firing whose reply contains `WATCH_DONE` delivers once and stops its watch.
  `background_turn_status` looks any turn up by its 8-character short ref.
- The vault is the primary output target for knowledge and agent output.
- **Search freshness has two sources.** The vault watcher (events, seconds) and the scheduled `embeddings` task's full scan (every 30 minutes, the correctness floor). Index writes go through `hq_db::search::reconcile`, never hand-written SQL on `notes_fts`, so a delete clears every derived row. Tests that touch the index use the real migrated schema (`crate::migrations::run`).
- **Identity is never model-writable.** Tools learn the caller only from reserved `_hq_*` arguments the runtime sets (`_hq_connector`, `_hq_caller_session`, `_hq_native_*`). A native session strips every `_hq_*` key the model sent before it sets its own, and the MCP gateway strips `_hq_native_*` from callers. Provenance is recorded only where a write path knows the caller; otherwise it is `unknown`. Review state changes only through `note_review`.
- **Disk watchdog**: `crates/hq-cli/src/commands/start/daemon/tasks_periodic/disk_watchdog.rs` is a 6-hour daemon task
  that checks overall disk usage, the bun install cache, and `node_modules` and
  `target` bloat under configurable roots, notifying via the configured relay on
  breach. Config: `disk_watchdog:` in `~/.hq/config.yaml`.

## Entry Points

| What | Path |
|------|------|
| CLI binary | `crates/hq-cli/src/main.rs` |
| Agent session | `crates/hq-agent/src/session/mod.rs` |
| Sub-agent dispatch | `crates/hq-agent/src/agents/service.rs`, `crates/hq-agent/src/agents/tool.rs` |
| Coding-agent runtime (built-in host) | `crates/hq-host/`, `crates/hq-tools/src/agent_host/`, `crates/hq-tools/src/harness_session/`, `docs/AGENT_HOST.md` |
| Private network (`hq net`, WireGuard hub and spoke), the root applier and its grammar | `crates/hq-net/` (`grammar.rs` is the only language the applier reads, `applier.rs` the Linux-only opener and `wg` caller, `wgconf.rs`, `store.rs`, `init.rs`, `ops.rs`), `crates/hq-cli/src/commands/net.rs`, `deploy/net/` (systemd units), `docs/PRIVATE_NETWORK.md`, `docs/plans/private-network.md`, `docs/security/NETWORK.md` |
| Editions (`profile: full`, `lite`, `mobile`) and the Android build | `crates/hq-core/src/config/profile.rs` (`Profile`, `MobileConfig`, `embeds_notes`), `effective_profile` in `crates/hq-core/src/config/mod.rs` (Windows is always Lite, Android defaults to Mobile), `Profile::drops_tool_category` (used by `crates/hq-mcp/src/registry.rs` and `crates/hq-agent/src/builder/tools.rs`), `scheduled_tasks` in `crates/hq-cli/src/commands/start/daemon/mod.rs`, `.github/workflows/android.yml`, `scripts/check-android-alignment.sh`, `docs/HQ_MOBILE.md`, `docs/HQ_LITE.md` |
| Windows users (WSL2), adding a machine | `docs/WINDOWS.md`, `docs/JOIN_A_MACHINE.md` |
| Task-linked and chat-driven harness sessions | `crates/hq-tools/src/harness_session/mission.rs`, `crates/hq-web/src/session_driver.rs`, `crates/hq-web/src/sessions_api.rs`, "Sessions that work on a task" and "Watching and driving from a web chat" in `docs/AGENT_SESSIONS.md` |
| Pull-based updates | `crates/hq-update/`, `crates/hq-cli/src/commands/update.rs`, `deploy/install.sh`, `deploy/update/`, `docs/UPDATE_SYSTEM.md` |
| Restart notices | `crates/hq-cli/src/commands/notify_restart.rs` (`hq notify-restart`) |
| MCP server | `crates/hq-mcp/src/server.rs` |
| Asking HQ from an MCP client (`hq_ask`, `hq_ask_result`) | `crates/hq-tools/src/ask.rs` (tools and the `AskRunner` seam), `crates/hq-web/src/ws/ask.rs` (runner, read-only enforcement via the permission preset), `crates/hq-db/src/ask_requests.rs`, `docs/MCP_ASK.md` |
| Daemon scheduler | `crates/hq-cli/src/commands/start/daemon/mod.rs` |
| Context engine | `crates/hq-agent/src/context/engine.rs` |
| Vault client | `crates/hq-vault/src/client.rs` |
| Search (FTS5 + semantic) | `crates/hq-db/src/search/` |
| `hq web` (host the web UI: foreground, `--detach`, `status`, `stop`, `--json`) | `crates/hq-cli/src/commands/web.rs`; shared server setup `build_web_state` and `default_static_dir` in `crates/hq-cli/src/commands/start/mod.rs`. Agents setting HQ up for someone should run `hq web --json` (it detaches) and read `login_url`. |
| Discord bridge | `crates/hq-relay/src/discord/` |
| Telegram bridge | `crates/hq-relay/src/telegram/`; commands and watch firing shared by both bridges in `chat_commands.rs` and `watch_scheduler.rs` |
| Remote MCP servers (`remote_mcp:` config) | `crates/hq-tools/src/remote_mcp.rs` |
| claude.ai connector (`mcp_connector:` config, OAuth for claude.ai and Claude Code, `hq connector`) | `crates/hq-web/src/oauth/` (server, consent API, `guard.rs` for approvals, rate limit and call log), `crates/hq-db/src/mcp_connector.rs`, `crates/hq-mcp/src/gateway.rs` (`CONNECTOR_CONFIRM_TOOLS`), `apps/hq-web/src/routes/connect.tsx`, `docs/CLAUDE_CONNECTOR.md`, `docs/plans/claude-connector.md` |
| Note export (pdf, docx, png, svg, html, md, xlsx, csv, json, jsonl, xml, latex, ipynb, jira, code): `hq vault export --format`, `hq vault export-pdf`, `vault_export`, `vault_export_pdf`, `/api/note/export`, `/api/note/pdf` | `crates/hq-export/` (document model `doc.rs`, writers, Typst layout `typst_markup.rs` and `render.rs`, vault glue `note.rs`), `crates/hq-convert/src/note_pdf.rs` (note preparation, legacy PDF engines behind `HQ_PDF_ENGINE`), `crates/hq-tools/src/convert.rs`, `crates/hq-web/src/vault_api.rs`, `docs/NOTE_EXPORT.md` |
| Importing files as notes (pdf, docx, xlsx, pptx, odt, rtf, html, xml, csv, images, old doc/ppt): `hq vault inventory`, `hq vault import`, `vault_inventory`, `vault_import`, `convert_to_markdown` | `crates/hq-convert/src/ingest/` (entry `mod.rs`, sniffing `sniff.rs`, intermediate form and Markdown writer `ir.rs`, report `report.rs`, limits `package.rs`, one reader per format, PDF tables `pdf_layout.rs`, spreadsheet number formats `numfmt.rs` and `xlsx_formats.rs`, OCR confidence in `crates/hq-convert/src/ocr.rs`), `crates/hq-tools/src/vault/import.rs` (inventory, idempotent import by frontmatter `source`/`source_sha256`, archive, the two tools), `crates/hq-cli/src/commands/vault_import.rs`, `crates/hq-tools/src/convert.rs` (`convert_to_markdown`), connector approval in `crates/hq-mcp/src/gateway.rs` (`vault_import` is in `VAULT_WRITE_TOOLS`), taint in `crates/hq-agent/src/governance/taint.rs`, `crates/hq-convert/tests/corpus.rs`, `docs/NOTE_IMPORT.md` |
| Native task management | `crates/hq-db/src/tasks.rs` (shared write surface: spaces > initiatives > tasks > comments), `crates/hq-tools/src/tasks/` (MCP tools), `crates/hq-web/src/tasks_api.rs` (REST + WS broadcast), `apps/hq-web/src/routes/tasks.tsx` (UI). Tag-based agent routing pushes to `_mailboxes/<tag>/` synchronously via `hq_core::mailbox::notify_tagged_agents`. Sub-tasks, start dates, soft dependencies and List/Board/Timeline views: `docs/plans/native-tasks.md`. |
| Web Push and `notify_owner` (`web_push:` config) | `crates/hq-tools/src/notify.rs` (tool), `crates/hq-web/src/web_push/` (VAPID, RFC 8291 encryption, dispatcher, `/api/notifications/push/*`), `crates/hq-db/src/push_subscriptions.rs`, `apps/hq-web/src/lib/webPush.ts`, `apps/hq-web/src/sw.ts`, `docs/WEB_PUSH.md` |
| Vault change events: watcher, debounce queue, index and embedding freshness (`vault_watch:` config, `hq doctor` Search index, `hq vault repair`) | `crates/hq-daemon/src/vault_watch/` (`watcher.rs`, `queue.rs`, `pipeline.rs`, `status.rs`), `crates/hq-db/src/search/reconcile.rs` (the shared index writes and the full scan), `crates/hq-daemon/src/embeddings.rs` and `embedding_worker.rs`, `crates/hq-core/src/config/vault_watch.rs`, `docs/architecture/vault-events.md` |
| Note provenance, review state and the task activity view | `crates/hq-db/src/provenance.rs` (tables `note_provenance`, `note_review_events`), `crates/hq-tools/src/provenance.rs` (who is calling, the `Recorder`), `crates/hq-tools/src/vault/review.rs` (`note_review`, `note_provenance`), `crates/hq-tools/src/tasks/activity.rs` (`task_activity`), `crates/hq-web/src/tasks_api.rs` (REST), `apps/hq-web/src/components/tasks/TaskActivity.tsx` and `InitiativeBriefing.tsx` |
| Context packets (source-linked vault excerpts, built from a task id, with provenance and gaps) and their evaluation | `crates/hq-memory/src/context_packet.rs` (`build_packet`, `verify_citations`), `crates/hq-memory/src/context_task.rs` (the task record and its sources), `crates/hq-tools/src/vault/context.rs` (`context_packet`), `crates/hq-memory/tests/context_packet_eval.rs` (`cargo test -p hq-memory --test context_packet_eval`), `docs/architecture/vault-context-packets.md` |
| Skill evidence, outcomes and fixtures (`hq skills evidence`) | `crates/hq-db/src/skill_invocations.rs`, `skill_outcomes.rs`, `skill_evidence.rs`, `crates/hq-tools/src/skills/fixtures.rs`, `evidence_proposal.rs`, `crates/hq-cli/src/commands/skills_evidence.rs`, `docs/SKILL_FLYWHEEL.md` |
| Memory system | `crates/hq-memory/src/lib.rs` |
| LLM usage report | `crates/hq-cli/src/commands/usage.rs` (`hq usage`/`cost`/`summary` read hq-db `task_outcomes`) |
| Web and MCP auth, origin guard | `crates/hq-web/src/auth.rs`, `crates/hq-web/src/origin.rs`, `docs/security/WEB_AUTH.md` |
| Bash sandbox, prompt-injection policy | `crates/hq-agent/src/bash_sandbox.rs`, `crates/hq-agent/src/governance/` (`taint.rs`, `secrets.rs`, `egress.rs`), `docs/security/BASH_SANDBOX.md`, `docs/security/PROMPT_INJECTION.md` |
| Orchestrator role (`governance.orchestrator_role`) | `crates/hq-agent/src/builder.rs` (`SessionRole`, `ORCHESTRATOR_REMOVED_TOOLS`), `crates/hq-agent/src/session/role_denial.rs`, `crates/hq-tools/src/github.rs`, `docs/security/ORCHESTRATOR_ROLE.md` |
| Security policy, release gate | `SECURITY.md`, `docs/security/RELEASE_CHECKLIST.md` |
| Retired components (historical records only) | `docs/HERMES_HARNESS.md` (peer agents and the `hq-bus` fabric), `docs/N8N_HARNESS.md` (n8n and `hq-workflow`), `docs/FLEET_HARNESS.md` (multi-harness dispatch) |

## Development

```bash
cargo check                       # type check all crates
cargo test -p hq-tools            # test one crate (prefer this for speed)
cargo test --workspace            # all tests (slow; run before a PR)
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt
hq mcp doctor                     # verify MCP connection health
hq skills validate                # reject skills that can never fire (also in hq doctor)
./scripts/cargo-gc.sh             # clean debug artifacts if over 20 GB
./scripts/setup-searxng.sh        # optional: local SearxNG container, tried before the built-in engine pool (set searxng_url). Web search is documented in docs/WEB_SEARCH.md
```

### Installing a new build

```bash
sudo ./scripts/install-hq.sh --link   # ONCE per machine, then never sudo again
./scripts/install-hq.sh               # build + install + restart daemon + verify
./scripts/install-hq.sh --check       # content hashes: is anything stale?
```

`--link` replaces the root-owned `/usr/local/bin/hq` with a symlink to
`~/bin/hq`. That path is hardcoded in the launchd plist, the MCP configs, and
three Rust sources (`mcp.rs`, `cursor_mcp_config.rs`, `self_update.rs`), so the
symlink keeps every one of them working while making the actual write
unprivileged.

Do not compare `--version` to check for staleness: it does not bump between
builds. `--check` compares content hashes, and the installer verifies that the
running daemon's binary matches what was just built. A green `/health` proves
something is listening, not that it is running your code.

### Build artifact hygiene

The debug profile accumulates stale binaries Cargo never cleans, and `target/debug/`
can reach 100 GB or more in weeks.

- On macOS, a weekly launchd job (`com.agent-hq.cargo-gc`, optional) runs
  `scripts/cargo-gc.sh` at 4am Sunday.
- Prefer `cargo test -p <crate>` over bare `cargo test`.
- Run `./scripts/cargo-gc.sh` after `./scripts/install-hq.sh` has built, installed
  and verified a release binary, rather than leaving `target/debug` around.

## Security defaults

- `/mcp` refuses every call unless `AGENTHQ_API_KEY` (or the read-only
  `AGENTHQ_SPARK_API_KEY`) is set. `HQ_MCP_DEV_NO_AUTH=1` opens it only on a
  loopback bind for requests no proxy forwarded. On a server the key lives in an
  env file such as `/opt/hq/mcp.env`, loaded by a systemd drop-in.
- The web token is accepted only as a Bearer header. The chat socket opens with a
  single-use ticket from `POST /api/ws-ticket`; URL tokens are refused.
- `web_allowed_origins` lists every origin the PWA is served from. Without it, an
  instance with no web token refuses unknown `Host` names, so a new deployment
  behind a proxy must set it.
- Bash runs with an allowlisted environment. Secrets reach it only through
  `governance.bash.env_passthrough` (for example `GH_TOKEN` and `GITHUB_TOKEN`
  for `gh`). `governance.bash.sandbox` is `off | best_effort | required`
  (bubblewrap on Linux, sandbox-exec on macOS), and `governance.bash.network: false`
  cuts bash off from the network.
- Once a session reads untrusted content (web, email, documents, other agents'
  mailboxes), reads of secret files and outbound network from bash are denied
  regardless of what the model says.
- A security workflow runs gitleaks and `cargo audit` separately from the main
  CI so it never blocks routine merges.

Details: `SECURITY.md` and `docs/security/`.

## Remote MCP servers

`crates/hq-tools/src/remote_mcp.rs` bridges any Streamable HTTP MCP server into
HQ's tool registry. Each entry under `remote_mcp:` gets two tools,
`<name>_discover` (tools/list) and `<name>_call` (tools/call), in the shared
`remote_mcp` category that the Telegram guest profile denies. Pick a name that
differs from the server's own tool prefixes (a server exposing `foo_call` should
not be named `foo`), since the server's tools are nested one level under
`<name>_call`. Entries default to `live_user_turn_only: true`, which keeps them
out of unattended sessions.

```yaml
remote_mcp:
  - name: diagrams
    url: https://example.com/api/mcp
    api_key: "..."
```

## Skills and document generation (optional)

HQ has its own markdown skills under `.vault/skills/`, validated by
`hq skills validate` and loaded into HQ sessions by hint match (see
`crates/hq-tools/src/skills/`). Skill self-improvement is one mechanism,
`hq_agent::skill_review`: after a session with enough tool calls, one LLM call
proposes up to three edits to skills HQ minted or the owner adopted
(`hq skills adopt|unadopt <name>` sets `managed: true`). Edits go through
`hq_tools::skill_edit`, are scanned for instruction overrides and opaque
payloads, and are audited in `skills/_audit.jsonl`. `hq skills revert <name>`
restores the newest archive and `hq skills history <name>` prints the trail.
`governance.skills_write_approval` (default off) stages every change in
`skills/_proposed/` for `hq skills approve|reject <name>`.

Skill evidence and tests: `skill_invocations` records the skill hash, task, purpose and
outcome of every load (ids and fixed values only, never prompt or document text),
`hq_db::skill_outcomes::settle` scores outcomes from observable acceptance rather than the
session exit, and `skills/<name>/fixtures.yaml` holds regression fixtures that gate edits. See
`docs/SKILL_FLYWHEEL.md`.

Document generation (docx, pptx, xlsx, pdf) is optional and depends on external
skills you install yourself. Brands are vault content, not a compiled-in list:
`hq_convert::brand::discover_brands` scans `Notebooks/Projects/*/Branding/` at
call time, so `brand_assets_list`, `brand_assets_add` and the `brand` parameter
of `convert_from_markdown` offer only the brands registered in that vault. Add
one with `Notebooks/Projects/<Name>/Branding/brand.yaml`.
`hq_convert::brand::BrandKit` (`crates/hq-convert/src/brand.rs`) resolves a brand
and `outbound.rs` uses it to pick a per-brand pandoc reference doc. The
`prose_lint` tool (`crates/hq-tools/src/prose_lint.rs`) is a mechanical check for
AI-sounding prose (`hq_core::prose_quality::SlopDetector`).

## Retired components

HQ is a reactive single harness: relays, web, CLI, harness sessions over the host,
memory, native tasks, and a small maintenance tier. Several subsystems that no
longer fit that shape were removed. Do not reintroduce them, and treat any
reference to them in old notes or skills as stale:

- Peer-agent messaging (`hq-bus`, NATS, `agent_ask` and friends) and the
  multi-harness fleet dispatch (`hq dispatch`, `/v1/fleet`, `harness_dispatch`).
  See `docs/HERMES_HARNESS.md` and `docs/FLEET_HARNESS.md`.
- The `hq-workflow` crate and the n8n integration (`docs/N8N_HARNESS.md`).
- The `hq-aidc` optimize loop, the ModelCard and `card_picker` system, and
  `LessonExtractor`.
- The cognition layer (dream cycle, curiosity, self-model and similar), the
  touchpoint generators (morning brief, digest composer and similar), and the
  CodeProposal approve-and-run path.
- The Cursor and Claude Code singleton tools (replaced by `harness_session`),
  browser and computer-use automation, the Chrome extension bridge, and the
  desktop, mobile, WhatsApp and Obsidian clients.
- `hq-audio` (voice transcription), `hq-company` (company registry and secret
  store), `hq-crypto`, `hq-calendar`, `hq-sync`, `hq-tui` and `hq-codegraph`.
  Email goes through the `gws` CLI; agents orient with `grep` and line-range
  reads instead of a code graph.
- ClickUp and Trello task mirroring. Tasks are native (see Entry Points).

The MCP registry is a 2-tool gateway (`hq_discover`, `hq_call`); call
`hq_discover` at runtime for the live tool catalog instead of trusting a count in
a document.

## Crates

| Crate | Purpose |
|-------|---------|
| `hq-core` | Types, config, errors, token counters and truncation (`tokens`, `text`), microcompact |
| `hq-vault` | Vault I/O, notes, query builder |
| `hq-db` | SQLite pool, FTS5, embeddings, graph links |
| `hq-llm` | LLM provider trait, OpenRouter, Ollama, backend chains |
| `hq-agent` | Session loop, sub-agents, coding tools, governance, 5-layer context engine (`context/`) |
| `hq-tools` | HQ tools (vault, tasks, skills, agents, harness sessions, web, git), registered in `crates/hq-mcp/src/registry.rs::create_default_registry()` |
| `hq-mcp` | MCP stdio server (2-tool gateway) |
| `hq-daemon` | Agent worker, value bus, memory cycle, email triage (recurring prompts are `/watch`, not a daemon task) |
| `hq-memory` | Consolidator, ingester, querier, forgetter, entity graph |
| `hq-relay` | Platform bridge, unified bot, Discord and Telegram (feature-gated) |
| `hq-web` | WebSocket server, REST API, embedded web UI |
| `hq-convert` | Document conversion both ways, OCR, brand kits |
| `hq-export` | Native note export (PDF, PNG, SVG through embedded Typst; DOCX; HTML, spreadsheets, JSON, XML, LaTeX, notebooks, Jira markup) with no external tools |
| `hq-update` | Signed pull-based updater (library behind `hq update`) |
| `hq-host` | Built-in coding-agent host: pty panes, emulated screen, control socket, state detection |
| `hq-gate` | Signed TCP transport for the host gate over a private tunnel (Ed25519 challenge, unix only); used by `hq host serve` and the `tcp` host transport |
| `hq-net` | `hq net`: the WireGuard private plane (key and subnet setup, peer store, device configs, QR) and `hq-net-apply`, the Linux-only strict-grammar root applier. Root-only code is `cfg(target_os = "linux")`, never `cfg(unix)` |
| `hq-sandbox` | Process sandbox policy for one coding agent (`sandbox-exec` or `bwrap`) |
| `hq-cli` | The `hq` binary and its CLI commands |

## Writing style

For generated documents, vault notes and UI strings, avoid the usual tells of
machine-written text: no em-dash overuse (commas, periods, colons or parentheses
instead), no "In today's landscape" openers, no "In conclusion" closers, and
none of the stock words "delve", "leverage", "robust", "seamless", "cutting-edge".
Do not start every bullet with a verb. Write like a person.
