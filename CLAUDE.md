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
- CI runs tests with `cargo nextest` (install once: `cargo install cargo-nextest --locked`), which
  runs every test and does not stop at the first failing binary. `cargo nextest run -p <crate>` is
  the fast local loop. Plain `cargo test --workspace` still works but aborts at the first failing
  test binary, which masks every crate after it, and it is the one that runs doctests (CI runs
  `cargo test --workspace --doc`). A partial run is not a green run: check the totals.
  Nextest runs each test in its own process, so a test that depends on running after another one
  in the same binary, or on shared process state, fails there: fix the test, not the config.
- Work on a branch and open a pull request. Keep changes focused and atomic,
  write tests for new behavior, and follow the style of the surrounding code.
- Do not sit waiting for CI. Once the gates are green locally, open the PR, turn on auto-merge with
  `gh pr merge --auto --squash` (the repository's auto-merge setting must be on), and start the next
  task in a new worktree branched from current `main`. A task that depends on a PR that has not merged
  branches from that PR's branch instead, and is brought up to date once the PR lands (merge `main`
  in; rebase only a branch nobody else has pulled). Each worktree has its own `target/`, so keep at
  most one extra unless there are 50 GB free.
- A change a user or operator would notice adds one `changelog.d/<slug>.<kind>.md` file (see
  `changelog.d/README.md`) instead of editing `CHANGELOG.md`, so two parallel pull requests never
  collide on it. `scripts/changelog-compile.sh` folds them in when a release is cut.
- A shortcut you knowingly leave is one new file `docs/techdebt/<slug>.md` (see its README), never a
  row in `TECHDEBT.md`, which is frozen. A temporary re-export left by a split is `facade-<what>.md`
  and says when it can be removed.
- On a conflict in a generated file (`Cargo.lock`, or the strings outputs `en.gen.ts` and
  `strings_catalogue.xml`), never merge it by hand: run `scripts/resolve-generated.sh`, then `git add`
  what it lists. Rebase onto current `main` before the final push.
- This root file is loaded into every session, so it stays under 20 KB (`scripts/claude-md-check.py`). Notes about one area go in the `CLAUDE.md` beside that code and are listed in "Where things live" below.
- Do not hand-deploy to a server. Servers update themselves from signed releases
  (`docs/UPDATE_SYSTEM.md`, `deploy/README.md`).

## Architecture

- **Language**: Rust (edition 2024, Cargo workspace of 24 crates, plus the HQ Suite crates under `crates/suite/`)
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
- **No job queue**: the filesystem job queue (`_jobs/`) is retired. All work
  dispatches through sub-agents.
- The vault is the primary output target for knowledge and agent output.
- **Adoption data stays on the device.** What the person uses HQ for (capabilities used or never
  tried, corrections, return visits) is never sent to a model provider, a push service, a relay or
  any third party, and is never sold or shared. It orders the owner's own chips and fills their
  weekly digest, nothing else. The model's "Things you can offer" block takes no usage
  (`hq_tools::capabilities::prompt_block`), web push skips `hq_core::types::DEVICE_ONLY_SOURCES`,
  and `crates/hq-tools/tests/adoption_stays_local.rs` fails if adoption code imports a way out.
  "You haven't tried this" nudges are chosen on the device, never by the model. Policy:
  `docs/security/ADOPTION_DATA.md`.
- **Identity is never model-writable.** Tools learn the caller only from reserved `_hq_*` arguments the runtime sets (`_hq_connector`, `_hq_caller_session`, `_hq_native_*`). A native session strips every `_hq_*` key the model sent before it sets its own, and the MCP gateway strips `_hq_native_*` from callers. Provenance is recorded only where a write path knows the caller; otherwise it is `unknown`. Review state changes only through `note_review`.

## Where things live

Every area keeps its entry points, paths and area rules in a `CLAUDE.md` beside its code, which an
assistant loads when it works there. Start from the file nearest the code you are changing.

| File | Covers |
|------|--------|
| `crates/hq-agent/CLAUDE.md` | Agent sessions, sub-agents and governance |
| `crates/hq-bridge/CLAUDE.md` | Pairing machines |
| `crates/hq-cli/CLAUDE.md` | The `hq` binary and its commands |
| `crates/hq-convert/CLAUDE.md` | Document conversion and import |
| `crates/hq-core/CLAUDE.md` | Types, configuration and editions |
| `crates/hq-db/CLAUDE.md` | The database, search and vault events |
| `crates/hq-desktop/CLAUDE.md` | The macOS desktop app |
| `crates/hq-edge/CLAUDE.md` | The public forwarder |
| `crates/hq-export/CLAUDE.md` | Note export |
| `crates/hq-mcp/CLAUDE.md` | The MCP server |
| `crates/hq-memory/CLAUDE.md` | The memory system |
| `crates/hq-net/CLAUDE.md` | The private network |
| `crates/hq-relay/CLAUDE.md` | Discord and Telegram |
| `crates/hq-tools/CLAUDE.md` | HQ tools, harness sessions and the features built on them |
| `crates/hq-tools-web/CLAUDE.md` | Web tools and remote MCP servers |
| `crates/hq-vault/CLAUDE.md` | The vault client |
| `crates/hq-web/CLAUDE.md` | The web server and its auth |
| `crates/suite/CLAUDE.md` | The HQ Suite |

Docs that are not tied to one crate:

| What | Path |
|------|------|
| Windows users (WSL2), adding a machine | `docs/WINDOWS.md`, `docs/JOIN_A_MACHINE.md` |
| Security policy, release gate | `SECURITY.md`, `docs/security/RELEASE_CHECKLIST.md` |
| Retired components (historical records only) | `docs/HERMES_HARNESS.md` (peer agents and the `hq-bus` fabric), `docs/N8N_HARNESS.md` (n8n and `hq-workflow`), `docs/FLEET_HARNESS.md` (multi-harness dispatch) |

## Development

```bash
cargo check                       # type check all crates
cargo nextest run -p hq-tools     # test one crate (prefer this for speed)
cargo nextest run --workspace     # all tests (slow; run before a PR), then
cargo test --workspace --doc      # doctests, which nextest does not run
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
unprivileged. It is for a developer's own machine only: the root updater and
`deploy/install.sh` refuse a linked or user-owned `/usr/local/bin/hq`, since root
would then run a file the user can change.

Do not compare `--version` to check for staleness: it does not bump between
builds. `--check` compares content hashes, and the installer verifies that the
running daemon's binary matches what was just built. A green `/health` proves
something is listening, not that it is running your code.

### Build artifact hygiene

The debug profile accumulates stale binaries Cargo never cleans, and `target/debug/`
can reach 100 GB or more in weeks.

- On macOS, a weekly launchd job (`com.agent-hq.cargo-gc`, optional) runs
  `scripts/cargo-gc.sh` at 4am Sunday.
- Prefer `cargo nextest run -p <crate>` over a workspace-wide run.
- Run `./scripts/cargo-gc.sh` after `./scripts/install-hq.sh` has built, installed
  and verified a release binary, rather than leaving `target/debug` around.

## Security defaults

- `/mcp` refuses every call unless `AGENTHQ_API_KEY` (or the read-only
  `AGENTHQ_SPARK_API_KEY`) is set. `HQ_MCP_DEV_NO_AUTH=1` opens it only on a
  loopback bind for requests no proxy forwarded. On a server the key lives in an
  root-only env file, `/etc/hq/mcp.env`, loaded by a systemd drop-in (never in
  `/opt/hq`, which the service user owns).
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
- ClickUp and Trello task mirroring. Tasks are native (see `crates/hq-tools/CLAUDE.md`).

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
| `hq-tools` | HQ tools (vault, tasks, skills, agents, harness sessions, web, git), registered in `crates/hq-mcp/src/registry.rs::create_default_registry()`. Its modules are layered: `crates/hq-tools/layers.toml` gives each top-level module a layer and `tests/layering.rs` fails when one reaches up to a higher layer outside `allowed_back_edges`, a list that only shrinks (a new module needs a layer; measure with `python3 -I scripts/ci/module-graph.py`) |
| `hq-tool-api` | The `HqTool` trait and `ToolRegistry` (`registry`), shared helpers (`util`) and the family gates (`family_guest`, `family_confirm`). Depends only on `hq-core`; `hq-tools` re-exports every module, so `hq_tools::registry::...` still works |
| `hq-tools-office` | The `office_file` tool (make a document in the vault smaller), over `hq-suite-adapter`. Split out of `hq-tools` so a change in the HQ Suite no longer rebuilds every tool; `hq-mcp` builds it under its `suite` feature. The tool's name and the actions that move an original are in `hq-tool-api::office` (re-exported as `hq_tools::office`) |
| `hq-tools-web` | The web tools (`web`: search, fetch, the engine pool), the remote MCP bridge (`remote_mcp`) and `imagegen`, split out of `hq-tools`; `hq-tools` re-exports all three so `hq_tools::web::...` still works |
| `hq-mcp` | MCP stdio server (2-tool gateway) |
| `hq-daemon` | Agent worker, value bus, memory cycle, email triage (recurring prompts are `/watch`, not a daemon task) |
| `hq-memory` | Consolidator, ingester, querier, forgetter, entity graph |
| `hq-relay` | Platform bridge, unified bot, Discord and Telegram (feature-gated) |
| `hq-web` | WebSocket server, REST API, embedded web UI |
| `hq-convert` | Document conversion both ways, OCR, brand kits |
| `hq-export` | Native note export (PDF, PNG, SVG through embedded Typst; DOCX; HTML, spreadsheets, JSON, XML, LaTeX, notebooks, Jira markup) with no external tools |
| `hq-update` | Signed pull-based updater (library behind `hq update`) |
| `hq-backup` | Encrypted backup and restore of a vault and its database: ChaCha20-Poly1305 in authenticated chunks, Argon2id or a key file, a hash manifest, hostile-archive checks. Pure Rust, no network |
| `hq-host` | Built-in coding-agent host: pty panes, emulated screen, control socket, state detection |
| `hq-gate` | Signed TCP transport for the host gate over a private tunnel (Ed25519 challenge, unix only); used by `hq host serve` and the `tcp` host transport |
| `hq-net` | `hq net`: the WireGuard private plane (key and subnet setup, peer store, device configs, QR) and `hq-net-apply`, the Linux-only strict-grammar root applier. Root-only code is `cfg(target_os = "linux")`, never `cfg(unix)` |
| `hq-bridge` | Native Bridge pairing over iroh: a one-time ticket shown as an `hqpair://` link and QR code, a throwaway endpoint per ticket, six digits typed on the host, the paired list in `hq-db`, reconnect by key, revocation pushed to open connections. Library only: nothing listens until a caller starts a `Bridge` or opens a ticket |
| `hq-desktop` | The desktop app's logic with no GUI dependency: the host's lifecycle (stops only the child it started), pairing on `hq-bridge` (digits compared in Rust, revoke through the one running bridge), Connect Claude over HQ's own connector routes, and the loopback proxy (loopback only, Host and Origin checks, adds no credential). The Tauri shell in `apps/desktop` is a thin layer over it and is not a member of this workspace |
| `hq-tunnel` | A userspace WireGuard peer for a phone (boringtun and smoltcp, no `VpnService`): serve and reach maps between the tunnel and loopback, join-text parser, status file. Library only so far |
| `hq-edge` | `hq edge`: the blind public forwarder (SNI routing to a registered device over iroh, per-device caps, the `Registry` seam, a co-hosted `iroh-relay` that admits registered devices only). Unix only; not linked into the Android or Windows builds |
| `hq-sandbox` | Process sandbox policy for one coding agent (`sandbox-exec` or `bwrap`) |
| `hq-suite-*` | The HQ Suite, in `crates/suite/`: `md` (hqmd), `ingest`, `shrink`, `adapter`. Free of HQ except the adapter; see `crates/suite/CLAUDE.md` |
| `hq-cli` | The `hq` binary and its CLI commands |

## Writing style

For generated documents, vault notes and UI strings, avoid the usual tells of
machine-written text: no em-dash overuse (commas, periods, colons or parentheses
instead), no "In today's landscape" openers, no "In conclusion" closers, and
none of the stock words "delve", "leverage", "robust", "seamless", "cutting-edge".
Do not start every bullet with a verb. Write like a person.
