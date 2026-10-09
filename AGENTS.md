# Project Instructions

This file provides context for AI assistants working on this project.

## What This Is

Agent-HQ: a local-first AI agent hub written in Rust. Single binary (about 58 MB release). Data lives in `.vault/` (a markdown vault, gitignored). Puts one agent on Discord, Telegram, a web UI (PWA) and the terminal, all backed by a shared markdown knowledge base. Coding agents run as harness sessions in the host.

## Project Type: Rust

### Commands

```bash
cargo check                        # type-check all crates
cargo test -p hq-tools             # test a specific crate (prefer this for speed)
cargo test                         # run ALL tests (slow, 30+ test binaries; run before a PR)
cargo build --release -p hq-cli    # release build (about 58 MB)
cargo clippy --workspace --all-targets -- -D warnings   # lint (CI gate)
cargo fmt                          # format
```

### Build Artifact Hygiene

The debug profile accumulates stale binaries Cargo never cleans. With 17 crates and heavy deps, `target/debug/` can reach 100 GB+ in weeks.

- On macOS, an optional weekly launchd job (`com.agent-hq.cargo-gc`) runs `scripts/cargo-gc.sh` at 4am Sunday.
- Prefer `cargo test -p <crate>` over bare `cargo test` (avoids compiling all test binaries).
- Run `./scripts/cargo-gc.sh` manually if builds feel slow or disk is low.
- **Agent rule**: any time you run `./scripts/install-hq.sh` (build + install a release binary) — don't wait for the weekly cron. Run `./scripts/cargo-gc.sh` right after confirming the install (`--check`) succeeded. `target/debug` isn't needed once the release binary is installed and verified; leaving it around between agent sessions is exactly the bloat this script exists to prevent.

## Architecture

- **Language**: Rust (edition 2024, requires Rust 1.83+)
- **Binary**: `hq` at `~/bin/hq`
- **Config**: `~/.hq/config.yaml`
- **Vault**: `.vault/` (markdown + YAML frontmatter, single source of truth)
- **DB**: `.vault/_data/vault.db` (single SQLite, WAL mode)
- **Background turns**: `background_turns` table in vault.db — relay turns that outlive the ack window detach here; reconciled at startup and every 6h. Rows with `kind='watch'` power `/watch` recurring turns; the relay watch scheduler owns their lifecycle (the reconciler leaves them alone unless abandoned).
- **LLM**: OpenRouter via `async-openai` with base URL swap; Ollama also supported
- **All source in `crates/`** — standard Rust conventions (snake_case)
- **Frontmatter**: `gray_matter` crate (Rust port of JS gray-matter)

## Rules

- `.vault/` is the center — shared memory, context engine, knowledge base. Every agent reads from it and writes back to it.
- **Execution**: Sub-agents via the unified `spawn_subagents` tool over `AgentService` (single/parallel/race/graph modes, in-process). It is the only sub-agent substrate (`crates/hq-agent/src/agents/`); `SpawnSubagentTool` and `CoordinatorSession` are gone.
- **No job queue**: The filesystem job queue (`_jobs/`) has been retired. All work dispatches through sub-agents or relay notifications.
- Vault is the primary output target for knowledge, notes, and agent output.
- Follow existing code style and patterns.
- Write tests for new functionality.
- Keep changes focused and atomic.
- Document public APIs.

## Entry Points

| What | Path |
|------|------|
| CLI binary | `crates/hq-cli/src/main.rs` |
| Agent session | `crates/hq-agent/src/session/mod.rs` |
| Sub-agent dispatch | `crates/hq-agent/src/agents/service.rs`, `crates/hq-agent/src/agents/tool.rs` |
| MCP server | `crates/hq-mcp/src/server.rs` |
| Daemon scheduler | `crates/hq-cli/src/commands/start/daemon/mod.rs` |
| Context engine | `crates/hq-agent/src/context/engine.rs` |
| Vault client | `crates/hq-vault/src/client.rs` |
| Search (FTS5 + semantic) | `crates/hq-db/src/search/` |
| Discord bridge | `crates/hq-relay/src/discord/` |
| Telegram bridge | `crates/hq-relay/src/telegram/` |
| Coding-agent sessions (built-in host) | `crates/hq-tools/src/harness_session/`, `docs/AGENT_HOST.md` |
| Windows users (WSL2), adding a machine | `docs/WINDOWS.md`, `docs/JOIN_A_MACHINE.md` |
| Memory system | `crates/hq-memory/src/lib.rs` |

## Crates

| Crate | Purpose |
|-------|---------|
| `hq-core` | Types, config, errors, token counters and microcompact |
| `hq-vault` | Vault I/O, notes, query builder |
| `hq-db` | SQLite pool, FTS5, embeddings, graph links |
| `hq-llm` | LLM provider trait + OpenRouter + Ollama |
| `hq-agent` | Session loop, sub-agents (`AgentService`), coding tools, governance, context engine (`context/`) |
| `hq-tools` | HQ tools (vault, tasks, skills, harness sessions, web) |
| `hq-mcp` | MCP stdio server (2-tool gateway) |
| `hq-daemon` | Agent worker, value bus, memory cycle, email triage |
| `hq-memory` | Consolidator, ingester, querier, forgetter, entity graph |
| `hq-relay` | Platform bridge, unified bot, Discord/Telegram (feature-gated) |
| `hq-web` | WebSocket server, REST API, embedded web UI |
| `hq-convert` | Document conversion both ways (PDF, DOCX, XLSX and more to markdown; markdown to docx/pptx), OCR, brand kits |
| `hq-export` | Native note export (PDF, PNG, SVG through embedded Typst; DOCX, HTML, XLSX, CSV, JSON, XML, LaTeX, notebooks, Jira markup), no external tools |
| `hq-update` | Signed pull-based updater (library behind `hq update`) |
| `hq-cli` | The `hq` binary and its CLI commands |

## Context Engine

The context engine (`hq-agent/src/context/`) assembles token-budgeted context frames with 5 layers:

1. **System** — SOUL + harness instructions
2. **UserMessage** — the current user turn
3. **Memory** — long-term facts (private tags stripped)
4. **Thread** — recent messages, older ones compacted
5. **Injections** — pinned notes + search results

Surplus tokens cascade between layers (thread 50%, injections 35%, memory 15%).

## Writing Style

For all generated documents, profiles, reports, and emails produced by agents:

- **Avoid em-dash (—) overuse.** Em-dashes are a strong signal of AI-generated text. Use commas, periods, colons, or parentheses instead. Max 2–3 em-dashes per page, and only where no simpler punctuation works.
- **Banned openers**: "In today's [adjective] landscape", "Let's dive in", "Here's the thing", "It's important to note that".
- **Banned closers**: "Final thoughts" sections, "In conclusion" paragraphs.
- **Banned words/metaphors**: "delve", "landscape", "tapestry", "leverage", "robust", "seamless", "cutting-edge", "innovative".
- **Structural**: Do not start every bullet with a verb. Avoid the "It's not just X, it's Y" construction. Do not start with sweeping statements about the "rapidly evolving world."
- **Write like a human.** If a paragraph has more than one em-dash, rewrite it. If every bullet follows the same `Verb + object — modifier — result` pattern, break the pattern.

This applies to vault notes, client deliverables, profiles, proposals, and any text output an agent produces.

## Documentation

See README.md for the project overview and quick start, CONTRIBUTING.md for the PR workflow, and CLAUDE.md for more detailed agent guidance.

## Version Control

This project uses Git. See .gitignore for excluded files.
