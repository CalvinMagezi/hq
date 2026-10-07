# Changelog

All notable changes to Agent-HQ will be documented in this file.

## [Unreleased]

### Added

- **`hq host`**: a built-in host for long-lived coding agents (pseudo-terminals with a readable screen, a JSON control socket with an operator token). `hq host serve|status|stop`; it also detects whether Claude Code and Codex are idle, working or blocked from the screen. It brings agents spawned with a resume command back after a restart or crash (`session.json`, mode 0600) and reports when its binary has been replaced on disk. See `docs/AGENT_HOST.md`. A built-in host on another machine is reachable over ssh through `hq host gate` (`herdr.hosts.<name>.kind: native`, optional `port`). The host sends state changes as events and tracks `done`, so HQ's supervisor reacts within seconds instead of the next minute sweep. Claude Code launched on it reports its own state and conversation id through per-launch hooks (`hq host report`). New sessions can run on it with `herdr.default_host: native`; the default is still herdr.

### Upgrading from 0.9.0 or earlier

- **`HTTP-Referer` is no longer sent by default.** It used to be a hardcoded URL. To keep OpenRouter app attribution, set `http_referer: <your url>` in `config.yaml` (or `HQ_HTTP_REFERER`) before upgrading.
- **Agent bash now needs a sandbox by default.** `governance.bash.sandbox` defaults to `required`. On a host without bubblewrap (Linux) or sandbox-exec (macOS) the `bash` tool refuses every command. Before upgrading such a host, run `apt install bubblewrap` (then restart HQ), or set `governance.bash.sandbox: best_effort` explicitly to keep running commands unwrapped. HQ logs an error at startup and files an owner notification when it is refusing; `hq doctor` shows the state.
- **Chat relays no longer adopt the first sender as owner.** Set the owner in config, or run `hq pair` and send `/pair <code>` (Telegram, private chat) or `!pair <code>` (Discord DM). Existing owners recorded earlier keep working.
- **Self-update install needs owner approval**, and the tools stay off unless `self_update.enabled` is true.
- **Removed: `companies[].budget` (`monthly_usd`, `alert_at`).** Nothing ever read these settings. A config that still has them keeps loading, the keys are ignored, and they are dropped from `config.yaml` the next time HQ saves its config.

## [0.9.0] - 2026-05-24

### Agent Sessions

- **Extended runtime limits**: sessions now default to 500 turns and a 5-hour wall-clock limit. When either is hit the relay receives a `TimeLimitReached` or `MaxTurnsReached` summary with elapsed time and files touched.
- **Graceful cancel**: send `!cancel` in Discord or Telegram to interrupt the active session at the next tool boundary. The agent replies with a summary before halting.
- **Cancel handle**: `AgentSession::cancel_handle()` returns an `Arc<AtomicBool>` that relay adapters hold to trigger the interrupt without locking the session.
- **File tracking**: `AgentSession` now records every file path written during a session for inclusion in cancel/timeout summaries.

### Relay

- **Richer heartbeat**: Discord and Telegram heartbeat tickers now show turn count and the currently executing tool name.
- **!cancel command**: both Discord (`hq-relay`) and Telegram (`hq-relay`) relay bots now accept `!cancel` mid-session.
- **`ChannelState::active_cancel`**: typed cancel-flag field on the shared relay channel state.

### Daemon

- **NotebookLM morning brief**: feature-flagged daily task (`daemon.notebooklm_morning_brief: true`) that pipes vault context into NotebookLM via the `nlm` CLI and pushes the audio URL to Telegram. Timeout is 600 s.
- **Orphan CC task detection**: on daemon startup, any Claude Code sentinel files left by crashed sessions are detected and a relay notification is fired so you can resume.
- **`notebooklm_morning_brief` MCP tool**: accessible as `hq_call({tool: "notebooklm_morning_brief", ...})`.

### Agent Intelligence

- **Mood engine** (`hq-agent/src/session/mood.rs`): classifies the user's last message into `UserEmotion` (Delighted/Stressed/Focused/Confused/Neutral) and picks a matching `AgentMood` (Celebratory/Supportive/Calm/Clarifying/Default).
- **Persona sharing** (`hq-agent/src/sharing.rs`): `package_agent_persona` zips an agent profile into a `.hqap` archive, stripping API keys, tokens, and `[private]` blocks before packaging.

### Desktop App (Tauri)

- **Active window context** (`active_context.rs`): queries the focused macOS application and window title via `osascript`, exposed as a Tauri command for agent context injection.
- **Enhanced vault watcher** (`vault_watcher.rs`): debounced (300 ms), stability-checked, and wired to the search engine so changed notes are re-indexed immediately.
- **TypeScript bindings** (`bindings.ts`): new `ActiveWindowContext` type added to the generated bindings.

### Chrome Extension

- **Branded icons**: 16/32/48/128 px icon set for the browser toolbar and extension page.
- **Tab groups**: agents can now create, rename, and navigate Chrome tab groups.
- **Activity banner**: overlay shown when an agent session is active.
- **Cursor overlay**: visualises agent pointer position during automated interactions.

### Canvas / DrawIt

- **DrawIt v2 engine**: `DiagramViewer` rewritten to use the DrawIt engine's React bindings directly, replacing the v1 script-tag approach. Full edit mode with undo/redo, tool palette, and save-to-vault.
- **DrawitToolbar**: new event-driven toolbar (undo/redo, zoom, tool selection, SVG/PNG export, dirty indicator). All state driven by `DiagramEngine` events — no polling.
- **Agent diagram push**: `diagram_push` and `diagram_read` MCP tools broadcast diagram documents over a dedicated `/ws/canvas` WebSocket endpoint. Connected `/drawit` tabs load the document instantly. New tabs receive the last-pushed document on connect.
- **New diagram creation**: sidebar "+ New diagram" button creates a `.drawit` file directly in `Notebooks/Diagrams/` and immediately opens it for editing.
- **Canvas nav link**: `/drawit` added to the desktop header and mobile bottom nav.

### Evening Brief

- **Rich evening context**: `gather_evening_context` now collects news headlines, git commits (EAT-aware midnight), recently modified vault notes, active/stalled plans, and tomorrow's calendar events.
- **`format_evening_digest`**: structured Evening Brief message with NEWS, TODAY, and TOMORROW sections; capped at 3 headlines and 5 commits.
- **LLM script generation**: evening audio brief uses the full `EveningContext` as prompt context for a 3-minute two-speaker dialogue.

### Bug Fixes

- **`canvas_ws` race**: WebSocket handler now subscribes to the broadcast channel before reading `canvas_last`, closing a race where a diagram push between the two operations was silently lost.
- **`handleSave` data loss**: save handler now checks `writeNote`'s `{ success }` response; dirty flag is no longer cleared on a rejected write.
- **`handleNewDiagram` phantom file**: diagram creation now checks `writeNote`'s `{ success }` response before adding the file to the sidebar. Also strips path separators from the diagram name to prevent writing outside `Notebooks/Diagrams/`.
- **`get_git_activity_today` working directory**: git log now runs with `current_dir(repo_root)` so the daemon finds the `.git` directory regardless of its launch working directory.
- **`get_git_activity_today` timezone**: `--since=00:00` replaced with `--since=@<unix_timestamp>` computed from EAT midnight, so launchd's inherited TZ does not affect the result.
- **Evening brief unconditional delivery**: brief is now skipped when all context sections (news, git, vault notes, stalled plans) are empty, matching the intent of the old `items.is_empty()` guard.
- **TTS fallback injection**: fallback dialogue no longer embeds raw `context_str`, which could contain `[S1]`/`[S2]` tokens that corrupt speaker-tag parsing.
- **Ollama provider prefix**: the `ollama/` prefix is now stripped before the model name is sent to the Ollama API, fixing model-not-found errors when using the `ollama/<model>` routing alias.

## [0.8.9] - 2026-05-17

### Security Patch — Open-Source Hardening

#### Security

- **Telegram bot authorization** (`hq-relay`): Incoming messages are now gated to the
  authorized owner chat. On the very first message the chat ID is recorded in
  `_system/.telegram-auth-chat`; subsequent messages from any other chat ID are
  silently dropped. The config-level `relay.telegram_authorized_chat_id` field
  provides an explicit override for users who want to hard-code the authorized ID.
  Previously, any Telegram user who discovered the bot token could invoke
  `/screenshot` (capturing the owner's screen) or approve self-agency proposals.
- **Telegram notifications bot authorization** (`hq-relay`): The separate
  notifications approval handler (`run_notifications_approval_handler`) now applies
  the same chat-ID guard before processing any `approve`/`skip` commands.
- **Web server binds to 127.0.0.1, not 0.0.0.0** (`hq-cli`): The WebSocket/REST
  server now listens on loopback only. Previously it accepted connections from any
  device on the local network, exposing unauthenticated write endpoints (note delete,
  relay config) to network neighbors.
- **XSS: `onclick` removed from DOMPurify allowlist** (`hq-desktop`): The Markdown
  renderer no longer allows inline `onclick` attributes through DOMPurify. Code-copy
  buttons now use a `data-copy-text` attribute with a delegated container listener,
  eliminating a stored-XSS vector in vault notes rendered inside the Tauri WebView.
- **XSS: Mermaid SVG sanitized** (`hq-desktop`): Mermaid diagram output is now passed
  through `DOMPurify.sanitize()` before being written to `innerHTML`. Previously,
  Mermaid diagrams (a known XSS vector) could execute arbitrary JavaScript in the
  Tauri context.
- **XSS: KaTeX output sanitized** (`hq-desktop`): Math render output now passes
  through DOMPurify before `innerHTML` injection, consistent with the Mermaid fix.
- **XSS: PDF print popup escapes filename and sanitizes HTML** (`hq-desktop`,
  `hq-web`): The print-to-PDF handler now HTML-escapes the note filename and runs
  `DOMPurify.sanitize()` on the rendered HTML before writing to the popup window.
  Previously a note named `<script>...`.md` could execute JavaScript in the popup.
- **Path traversal via symlink in `note_delete_handler`** (`hq-web`): Added
  `std::fs::canonicalize` before the `starts_with(vault_path)` containment check.
  The previous string-based `..` filter did not catch symlink-based traversals.
- **Path traversal bounds check in desktop notes server** (`hq-desktop`): Added
  `assertInVault()` guard to `getNote`, `getNoteOrDir`, and `updateNote`. Previously,
  a path like `../../.ssh/id_rsa` was passed directly to the Tauri `read_file`
  command with no validation.
- **Personal OAuth client ID removed from plugin defaults** (`plugins/obsidian-vault-sync`):
  The gws CLI OAuth client ID (`759142433452-...`) was shipped as the default in
  `types.ts`. Users now supply their own OAuth credentials. See the plugin README for
  how to create a Desktop app credential in Google Cloud Console.
- **Zombie process prevention in Claude cron runner** (`hq-cli`): Spawned `claude -p`
  processes are now awaited with a 5-minute timeout. Previously each cron tick that
  fired a task leaked a zombie process until daemon restart.

#### Added

- **GitHub Copilot harness** (`hq-core`, `hq-relay`): New `GitHubCopilotConfig` struct
  and `"github-copilot"` harness entry. The `gh copilot` extension is treated as
  available when `gh auth status` succeeds. Telegram `/harness github-copilot` (or
  aliases `copilot`, `ghc`, `gh-copilot`) activates it.
- **`SocialConfig`** (`hq-core`): New `[social]` config section for the multi-agent
  bulletin discussion system — `tick_interval_hours`, `max_rounds_per_day`,
  `max_agents_per_round`, `ollama_model`, `prefer_free_tier`.
- **Telegram slash-command autocomplete** (`hq-relay`): `setMyCommands` is called at
  bot startup to register `/new`, `/reset`, `/focus`, `/harness`, `/model`, `/status`,
  `/benchmark`, `/screenshot`, `/computer`, `/help` for Telegram's autocomplete UI.
- **`/focus [topic]`** command (`hq-relay`): Clears conversation history and per-harness
  session IDs while keeping the current harness and model override. An optional topic
  argument seeds the new thread with a context hint.
- **`/screenshot`** command (`hq-relay`): Captures a macOS screenshot via `screencapture`
  and sends it directly to the authorized Telegram chat. Auth-gated by the new
  chat-ID guard above.
- **`/computer <task>`** command (`hq-relay`): Initiates a supervised computer-use
  session, taking an initial screenshot and guiding the user through instructions.
- **Proposal quick-action keyboard** (`hq-relay`): Code proposals are now sent with
  a `ReplyKeyboardMarkup` showing `approve <id>` / `skip <id>` buttons so the user
  can tap instead of copying the hex ID.
- **`/new` alias** (`hq-relay`): `/new` now works as an alias for `/reset`.
- **Progress heartbeat** (`hq-relay`): During a running proposal implementation,
  heartbeat messages are sent at 90 s, 3 min, and 5 min so the user knows CC is
  still working.
- **Bulletin fast-cycle router** (`hq-cli`): New `tasks_fast.rs` module with
  `run_bulletin_router` — scans new bulletin entries for @mentions and routes them
  to the appropriate agent (Ollama inline or mailbox for external CLIs) every minute.
- **`run_expire_approvals`**, **`run_plan_sync`**, **`run_embedding_on_change`**:
  Additional fast-cycle daemon tasks extracted into `tasks_fast.rs`.
- **Memory consolidation insights feed self-agency** (`hq-daemon`): `gather_insights`
  now reads `Notebooks/Memories/` consolidation-insight notes as a third insight
  source. Promoted insights are marked `promoted: true` to prevent duplicate proposals.
- **`relay_status_handler`** (`hq-web`): New `GET /api/relay/status` endpoint returns
  connection status for Telegram and Discord relays (connected/disconnected/not_configured).
- **`relay_config_get/post_handler`** (`hq-web`): New `GET/POST /api/relay/config`
  endpoints for reading (masked) and updating relay bot tokens from the settings UI.
- **`note_delete_handler`** (`hq-web`): New `DELETE /api/note?path=...` endpoint
  for deleting vault notes from the web UI, with path-traversal protection.
- **`getBulletinFeed`** server function (`hq-web`, `apps/hq-web`): Reads
  `_bulletin/board.jsonl` and returns the last 60 entries for the bulletin UI.
- **Chrome extension bridge** (`hq-web`, `hq-tools`): `WsState` gains an optional
  `ext_bridge` field (local-feature-gated) and a `/ws/extension` WebSocket endpoint
  for the HQ Chrome extension.
- **`VaultGuardConfig`** and **`SocialConfig`** added to `HqConfig` default (see 0.8.8).
- **Ollama model list endpoint** (`hq-web`): `GET /api/ollama` returns available
  local models from `http://localhost:11434/api/tags`.
- **Harness GET endpoint** (`hq-web`): `GET /api/harness` returns the current harness
  name and discovered harness list.
- **Desktop vault directory browsing** (`hq-desktop`): `getNoteOrDir` now returns
  directory entries when the path resolves to a folder, enabling folder navigation
  in the vault viewer.
- **Tauri capabilities** (`hq-desktop`): `notification:default`,
  `global-shortcut:allow-*`, and `autostart:allow-*` capabilities added for
  quick-note overlay and login-item support.
- **Web app graph error handling** (`hq-web`): `GraphViewer` now handles fetch errors
  gracefully instead of leaving the canvas stuck in a permanent loading state.

#### Changed

- **`generate_natural_message` prompt** (`hq-daemon`): Refined to produce exactly
  2 sentences (not 2-3), enforces no filler phrases, rejects LLM output that echoes
  the task description verbatim, and hides the scope line when `affected_files_count`
  is 0.
- **Web router pruned** (`hq-web`): `create_router` now exposes only vault-focused
  endpoints. Novel reader, WhatsApp, model-card, and agent-chat endpoints removed from
  the default router to reduce attack surface.
- API path `/api/daemon-status` → `/api/daemon`, `/api/ollama/models` → `/api/ollama`.
- `.obsidian` no longer blanket-excluded from sync (see 0.8.8; included here for
  the changelog completeness of this release).

#### Fixed

- **PDF print popup null-window crash** (`hq-web`): Added guard against
  `window.open` returning `null` (browser popup blocker) plus `try/catch` around
  `marked.parse`.
- **GraphViewer stuck loading** (`hq-web`): `.catch()` handler added so a failed
  `getVaultGraph` call still calls `setLoading(false)`.
- **Proposal message with no pending list** (`hq-relay`): "No proposal found" reply
  now lists up to 5 pending proposals to help the user identify the correct ID.
- **User-model trait capture from reply** (`hq-relay`): Telegram replies to
  user-model questions are now routed to `apply_user_reply` before falling through
  to the regular message handler.

---

## [0.8.8] - 2026-05-12

### Tailscale Sync, Obsidian Bridge, VaultGuard & Document Conversion

#### Added

- **`hq-convert` crate** — new bidirectional document conversion library:
  - **Inbound** (any → Markdown): pure-Rust `transmutation` crate (PDF, DOCX,
    XLSX, PPTX, HTML, CSV, RTF, ODT, images, audio/video). Zero Python dependency.
  - **Outbound** (Markdown → any): shells to `pandoc` with clear install instructions
    if pandoc is missing. Supports PDF, DOCX, PPTX, HTML, EPUB, RTF.
  - `InboundConverter`, `OutboundConverter`, `DetectedFormat`, `ExportFormat`,
    `ConvertError` types with full format detection and safe path validation.
- **`VaultGuard`** (`hq-sync`): enforces the markdown-first vault invariant.
  Watches the vault for non-`.md` files, auto-converts them to Markdown via
  `hq-convert`, then moves the originals to configured export destinations.
  Non-blocking — spawns Tokio tasks; never stalls the watcher loop.
- **`VaultGuardConfig`** (`hq-core`): new config section `[vault_guard]` with:
  - `enabled` (default: true)
  - `export_destinations` (per-extension paths with `~` expansion)
  - `obsidian_api_port` (default: 27125)
  - `obsidian_api_key` (config override; auto-generated and persisted if absent)
  - `obsidian_live_api_key` (for bridging to real Obsidian Local REST API)
- **HQ Obsidian-compatible REST API** (`hq-web`): full implementation of the
  [Obsidian Local REST API](https://coddingtonbear.github.io/obsidian-local-rest-api/)
  spec so `mcp-obsidian` and other Obsidian automation clients work when Obsidian
  is not running. Endpoints: `GET/PUT/POST/DELETE /vault/{path}`, `POST /search/simple/`,
  `GET /tags/`. Path-traversal validation (`..`, absolute paths, null bytes rejected).
- **Obsidian bridge MCP tools** (`hq-tools`): 6 new MCP tools under category
  `"obsidian"` that auto-detect whether Obsidian is running and proxy accordingly:
  `obsidian_search`, `obsidian_read`, `obsidian_write`, `obsidian_patch`,
  `obsidian_tags`, `obsidian_open`.
- **Conversion MCP tools** (`hq-tools`): 2 new tools under category `"convert"`:
  `convert_to_markdown` and `convert_from_markdown`.
- **`hq sync serve`** (`hq-cli`): new subcommand starts the WebSocket sync server
  so Obsidian mobile can connect over Tailscale. Accepts `--port N` (default 18800).
- **Obsidian plugin — Tailscale real-time sync** (`plugins/obsidian-vault-sync`):
  - Primary transport upgraded to WebSocket over Tailscale (`SyncEngine`).
  - Google Drive demoted to offline fallback transport.
  - Settings tab redesigned with Tailscale section first, Drive as fallback.
  - New commands: `force-full-restore`, `show-mobile-setup`.
  - `isConnecting` guard on `SyncTransport` prevents duplicate connect races.
  - `workspace.json` debounce extended to 5 s to reduce noise from active Obsidian.
  - Mtime-unstable files are now re-queued instead of silently dropped.
  - `base64ToUtf8` helper added to `protocol.ts` for correct non-ASCII decoding.
- **Selective `.obsidian/` sync** (`hq-sync`): scanner and watcher now allow
  syncing `app.json`, `workspace.json`, `community-plugins.json`, `core-plugins.json`,
  `hotkeys.json` and theme/snippet CSS from `.obsidian/` instead of blanket-ignoring
  the directory. Plugin binaries are still excluded.
- **VaultClient write guard** (`hq-vault`): `write_note()` now rejects non-`.md`
  paths with a clear error, enforcing the markdown-first invariant at the API level.
- **Protocol `camelCase` serialization** (`hq-sync`): `#[serde(rename_all = "camelCase")]`
  added to all protocol structs so Rust ↔ TypeScript wire format is consistent.
- **Self-contained plugin protocol** (`plugins/obsidian-vault-sync`): all types,
  crypto, and constants inlined into `protocol.ts` — the `@repo/vault-sync-protocol`
  monorepo package is no longer required.
- **Obsidian API server** (`hq-daemon`): daemon now starts the HQ Obsidian REST API
  on port 27125 when `vault_guard.enabled = true`. API key is auto-generated with
  `uuid::Uuid::new_v4()` and persisted to `_data/obsidian-api-key`.

#### Changed

- `.obsidian` is no longer blanket-excluded from sync; essential config files are
  selectively included via `OBSIDIAN_ALLOWED` and `OBSIDIAN_ALLOWED_DIRS`.
- Default sync excludes list updated: `.obsidian` removed, comment added.
- Obsidian plugin transport architecture: `SyncTransport` no longer mutates
  `settings.deviceToken` on `hello-ack`; that responsibility belongs to `SyncEngine`.
- OAuth credentials for the Google Drive transport removed from source code;
  credentials are now supplied via plugin settings (`oauthClientId`, `oauthClientSecret`),
  with auto-population from the gws CLI on desktop.

#### Security

- **Hardcoded OAuth client secret removed** from `driveAuth.ts` and the compiled
  bundle. The secret was previously embedded in the open-source repository. The
  prior commit (`eba73f95`) that introduced it is in git history — the secret should
  be considered public and rotated if sensitive. The new design loads credentials
  from plugin settings or the gws CLI at runtime, with no defaults in source.
- **Cryptographically random API key generation**: the Obsidian API key is now
  generated with `uuid::Uuid::new_v4()` (128 bits of entropy) instead of the
  previous `subsec_nanos + pid` approach (predictable, ~32 bits effective entropy).
- **Path-traversal protection** in `obsidian_api.rs`: vault path segments are
  validated for `..`, leading `/`, and null bytes before any filesystem access.
- **`.gitignore` additions**: `*.secret`, `config.local.yaml`, `config.local.yml`,
  `plugins/*/data.json` (plugin runtime state) added to prevent accidental commits
  of per-instance secrets or generated data.

#### Fixed

- `sync.rs`: missing newline at end of file.
- `SyncTransport.connect()`: race condition — concurrent calls during reconnect
  could open two WebSockets. Fixed with `isConnecting` guard.

## [0.8.7] - 2026-05-12

### Pi Harness, Deep Sleep Memory, Live Harness Switching & Code Cleanup

#### Added

- **Pi coding agent harness** (`hq-core`): New `PiConfig` struct and `PiHarnessMode`
  enum (`Print`, `Json`, `Rpc`) for headless pi.dev integration. Fields: `cwd`,
  `binary`, `default_model`, `session_dir`, `timeout_secs`, `min_version`.
- **`HqConfig::resolve_pi_model()`**: Precedence chain for Pi model selection —
  per-turn user override → `PiConfig::default_model` → `RelayConfig::model` →
  `HqConfig::default_model`. Returns `None` when all candidates are empty so Pi
  uses its own built-in default.
- **Pi and DeepSeek-TUI in harness discovery**: `discover_harnesses()` now probes
  for `pi`, `deepseek-tui` (`deepseek` binary), and `kimi-cli` (`kimi` binary) in
  addition to existing harnesses.
- **`TaskContext::active_harness()`** (`hq-daemon`): Re-reads `active_harness`
  from the config file at call time, enabling live harness switches without
  restarting the daemon.
- **T3 deep sleep cycle** (`hq-daemon`): `run_deep_sleep_cycle()` runs a full
  vault analysis using a dual-LLM approach — a dream model (T1) and a dedicated
  deep-sleep model (T3), scheduled at 2am when CPU < 15% and RAM > 8GB.
- **T1 nap engine** (`hq-daemon`): `start_nap_engine()` starts a filesystem
  watcher that returns a `NapEngine` + event receiver for caller-driven polling.
  Exports `DreamEngine`, `NapEngine`, `NapEvent` from `hq-memory`.

#### Changed

- **Default harness flipped to `"pi"`**: `active_harness` and `coding_default_agent`
  now default to `"pi"` instead of `"claude-code"`. Existing configs with an
  explicit `active_harness` value are unaffected. If `pi` is not installed, HQ
  falls back gracefully via the discovery check.
- **Removed Ollama warm-up keepalive loop** (`hq-daemon`): The 20-minute
  `qwen3.5:9b` keepalive ping was removed. Ollama now uses adaptive eviction
  natively — continuous pinging added overhead without measurable benefit.
- **Dream cycle model binding** (`hq-daemon`): `run_dream_cycle()` now calls
  `hq_memory::dream_model()` instead of hardcoding `"ollama/qwen3.5:4b"`, so
  the tier model registry is the single source of truth.

#### Fixed

- **Stale hardware test assertions** (`hq-core`): `test_recommend_24gb` and
  `test_recommend_32gb` were asserting `gemma4:e4b`/`gemma4:26b` after the
  recommend function was updated to use `qwen3:8b`/`qwen3:14b`. Tests updated to
  match current implementation.

#### Security & Repository

- **`.gitignore` additions**: Added `*.rdb` / `dump.rdb` (Redis snapshots) and
  `terraces-full.png` / `turboquant_arxiv.png` (stray root-level images) to prevent
  accidental commits of non-source artifacts.

#### Code Quality

- **Workspace-wide `rustfmt` pass** (~85 files): Import reordering, long-line
  wrapping, trailing whitespace removal across `hq-context`, `hq-core`,
  `hq-crypto`, `hq-daemon`, and `hq-db`. Zero functional changes.
- **`sysinfo` dependency** added to `hq-daemon` for system health and load-gating
  in the deep sleep scheduler.

## [0.8.6] - 2026-05-09

### Unified Multi-Harness Persona & System-Wide Harness Switching

All harnesses (Claude Code, Cursor, HQ/Ollama, Gemini CLI, Codex) now present as
the same HQ persona regardless of the underlying engine. Switching harness is a single
command (`hq harness switch claude-code`) that propagates everywhere.

#### Added

- **`active_harness` config field**: Single source of truth in `~/.hq/config.yaml`.
  All entry points (Telegram, Discord, Web API, CLI, relay commands) read and write
  this field. Default: `"claude-code"`.
- **`discover_harnesses()`** (`hq-core`): Probes PATH for known CLIs (`claude`,
  `cursor-agent`, `gemini`, `codex`). Returns `Vec<HarnessInfo>` with availability
  flags. Used everywhere harness validation occurs.
- **`HqConfig::set_key()`**: Line-based YAML patcher so any crate can write a
  top-level config field without a full parse-serialize round-trip.
- **`hq harness` CLI command** (alias `h`): `status`, `list`, `switch <name>`.
  Validates against discovered harnesses before writing.
- **`!harness` command in Telegram and Discord**: Shows available harnesses with
  current marker; switches per-chat and system-wide simultaneously.
- **`GET /api/harness`**: Returns `{ active, available[] }`. Replaces the former
  410 Gone response. Mobile app and web dashboard can now read and switch harnesses.
- **`POST /api/harness`**: Validates, writes config, broadcasts `harness_changed`
  WebSocket event. Handles 400 (unknown), 422 (not installed), 500 (config write).
- **Full multi-harness dispatch in Discord**: `dispatch_hq` now routes to
  `claude-code`, `cursor`, `gemini-cli`, `codex`, or any external CLI by harness
  name — Discord was previously hardcoded to `"hq"`.
- **`cc_initialized` session tracking** (`ChannelState`): Tracks whether the
  claude-code harness has HQ persona established for a channel. Resets on harness
  switch so the next call triggers a fresh `--system-prompt` injection.

#### Fixed

- **HQ persona injection order** (critical): Claude Code relay was using
  `--append-system-prompt` which places HQ identity _after_ CC's built-in
  "I am Claude Code, a coding assistant" instructions. CC's own identity was
  winning. Fix: fresh sessions now use `--system-prompt` (replaces CC default,
  HQ persona at top); subsequent `--continue` sessions carry the persona from
  session state with no re-injection.
- **Discord `--print` sessions**: Switched from `--append-system-prompt` to
  `--system-prompt` in `run_claude_code_headless_with_context`. Every Discord
  message is a stateless `--print` call, so HQ context must be injected fresh
  each time — at the top, not appended.
- **Relay model switched to `granite4.1:8b`**: `relay`, `plan`, `premium`, `mid`
  aliases all point to Granite 4.1 8B. Qwen 3.5 9B was generating text-format
  "would-do" tool results (fake checkboxes, empty bash blocks) instead of JSON
  function calls. Granite has stronger native tool-calling and uses 1.3GB less RAM.
- **`(no result captured)` from ClaudeCodeChatTool**: When CC ends a session on
  a tool call (no final text block), the `result` field in the NDJSON stream-json
  output is empty. `cli_runner.rs` now tracks `last_assistant_text` across all
  assistant events and uses it as fallback. The three-layer chain:
  `result field → last_assistant_text → "(no result captured)"`.
- **`validate_path` blocked extensionless paths**: The old `!path.contains('.')`
  condition rejected valid paths like `_moc/Dashboard`. Removed; only trailing
  slashes are now rejected (pointing at a directory).
- **`read_note` auto-.md extension**: `vault_read` with `_moc/Dashboard` failed
  because the file is `Dashboard.md`. Notes now try `{path}.md` automatically
  when the exact path doesn't exist and has no extension (Obsidian wikilink style).
- **Memory consolidator compile error**: `consolidate_cluster` was called with 2
  args in the dream cycle path but takes 3. Fixed with empty project-context string.
- **Memory model**: Switched from `gemma4:e2b` to `qwen3.5:4b` for background
  memory operations (smaller, faster, leaves more VRAM headroom).
- **Dream cycle model**: Switched from `qwen/qwen3.6-plus-preview:free` (cloud)
  to `ollama/qwen3.5:4b` (local). Dream synthesis runs in background — no cloud
  dependency needed.

#### Improved

- **Memory consolidation grounding**: Consolidator now loads up to 8 project
  READMEs from `Notebooks/Projects/` and injects them into the LLM prompt so
  insights are grounded in actual project state rather than abstract patterns.
- **`build_hq_context` used consistently**: Canonical HQ identity block function
  shared by Telegram and Discord; fresh sessions pass the full enriched system
  prompt (not the previous 1800-char truncated excerpt).
- **Session runner refactored**: `run_claude_code_headless` is now a thin wrapper
  around `run_claude_code_headless_with_context`; `run_external_cli_harness` added
  as a generic fallback for unknown CLI harnesses.

## [0.8.5] - 2026-04-09

### Ironclad Context Engine

- **Elastic Knapsack Allocation**: Transitioned from dropping over-budget items to elastic truncation (semantic reduction) for high-priority content (System Soul, User Query).
- **Zero-Copy Memory Architecture**: Internalized `Arc<str>` across the context pipeline to prevent OOM and minimize allocation overhead.
- **BPE Drift Protection**: Implemented a 1.10 `DRIFT_SAFETY_FACTOR` and `RwLock`-guaranteed LRU token caching for robust budget estimation.
- **Iterative Semantic Truncation**: Added safety loops to the `SemanticReducer` to guarantee budget compliance even with extreme token density and BPE variance.
- **JSON Structural Integrity**: Depth-aware truncation prevents JSON fragmentation and semantic hallucinations during budget emergency cuts.

## [0.8.4] - 2026-04-09

### Memory Consolidation Bridge

- **Stalled Plan Detection**: Integrated into the memory consolidation cycle, allowing LLM-generated insights to surface project bottlenecks and pending approval states.
- **Context Injection**: Consolidation prompts now include a dedicated section for stalled plans found in the database.
- **Centralized Schema**: Moved `StalledPlan` and retrieval logic to `hq-db` for shared access across workspace crates.

### Code Quality & Compliance

- **Full Clippy Compliance**: Resolved all existing warnings (collapsible if, needless clones, etc.) across 26 crates, achieving zero-warning build state.
- **Modernized Idiomatic Rust**: Refactored legacy code patterns to use modern Rust features like `is_some_and` and `let` chains.

## [0.8.3] - 2026-04-07

### Job Queue Retirement & Dead Code Cleanup

Full code review of 69 unpushed commits (97 files, +1405/-4991 lines). Identified and resolved 17 issues across Rust and TypeScript.

### Breaking Changes

- **Job queue fully retired**: Removed `_jobs/` filesystem queue, `worker.rs`, `jobs.rs`, `tasks.rs`, `atomic_queue.rs`, `cleanup.rs`. All work dispatches through sub-agents or relay notifications.
- **Web API**: `vault_status` endpoint no longer returns `jobs_pending/running/done/failed` fields (directories don't exist).

### Added

- **Model Card Collectibles**: Game-stat cards for 351 LLM models with AIDC benchmark integration and mobile UI
- **HQ Mobile App**: Expo Router app with vault browser, chat, and WebSocket sync
- **AIDC modules**: `harness.rs`, `pareto.rs`, `trace.rs` for harness specs, Pareto frontier, session traces
- **`hq review` command**: Daily review with vault activity gathering and LLM synthesis
- **Session trace tracking**: `hq-daemon/session_trace.rs` for daemon-level lifecycle logging
- **Bidirectional vault sync**: Peer-to-peer sync over Tailscale
- **Multi-provider LLM chat**: Centralized `/api/chat` with tool-aware routing
- **PWA control center**: 25 iterative improvements (offline, search, chat panel, keyboard shortcuts)

### Fixed

- **Stale closure bug** (mobile): Vault note viewer no longer overwrites cached content on network fetch errors
- **Timeout message** (control center): WS timeout error now correctly says "10 minutes" (was "5 minutes")
- **Stale benchmark reference**: Removed deleted `worker.rs` from codegraph file ratio test
- **Dead code removal**: Unused MCP tool-discovery scaffolding, `PENDING_OPT_SUBDIR`, 5 dead chat styles, unused `isThinking`/`Dimensions`
- **`gather_daily_activity`**: No longer scans retired `_jobs/` directories
- **`hq review`**: Removed model resolution ceremony that was silently ignored
- **Compiler warnings**: Resolved all warnings across 8 crates (unused imports, variables, suspicious clones)
- **Doc comments**: Updated `hq-agent` lib.rs and builder.rs to reflect sub-agent dispatch replacing workers

## [0.8.2] - 2026-04-05

### Code Review: Full 21-Crate Audit

Comprehensive code review across all 546 source files (8,900 AST nodes). 57 issues identified, all 57 fixed.

### Critical Bug Fixes

- **Streaming session divergence** (hq-agent): `prompt_stream` and `prompt_inner` converged via 4 shared helpers (`build_turn_request`, `check_limits`, `check_compaction`, `process_tool_results`). Streaming sessions were missing deferred tool catalog, plan-mode suffix, and post-turn callbacks.
- **Atomic job moves** (hq-vault): `move_job` now writes to temp file + `fs::rename` instead of write-then-delete, preventing job duplication on crash
- **UTF-8 truncation panic** (hq-tools): `truncate_output` in `coding.rs` now uses `floor_char_boundary` instead of raw byte slicing, preventing panics on non-ASCII shell output
- **FTS5 tag search false positives** (hq-db): `get_tagged_note_paths` now uses FTS5 column filter syntax (`tags:<term>`) instead of unscoped MATCH that searched all columns
- **Semantic search OOM prevention** (hq-db): Added `LIMIT 10000` to both `semantic_search` and `find_similar_notes` queries that previously loaded all embeddings into RAM
- **`!reset` command was a no-op** (hq-relay): Added `ThreadStore::reset_thread()` that actually deletes the thread file and clears the chat_map entry
- **Discord bridge blocked forever** (hq-relay-discord): Wrapped `serenity::Client::start()` in `tokio::spawn` (matching Telegram bridge pattern), stored shard_manager and JoinHandle for graceful shutdown
- **Novel pipeline stubs silent success** (hq-novel): All placeholder functions now emit `tracing::warn!`; `export_pdf` returns an error; `StateMachine::advance` now validates chapter output before advancing phases
- **Ingestion lock race condition** (hq-memory): Lock acquisition moved before contradiction detection loop so all `store_fact` calls happen inside the lock scope
- **Token estimation wrong slice** (hq-context): `estimate_tokens_mixed` now iterates actual code/prose segments instead of slicing a prefix of the full text
- **EditFileTool skipped staleness check** (hq-tools): Replaced inline validation with call to canonical `validate_file_edit()` + `apply_edit()` from `file_edit.rs`
- **Wildcard route specificity** (hq-llm): Added `is_wildcard` tiebreaker to `ScoredCandidate` so explicit routes always beat wildcard matches (e.g., `cerebras/llama-8b` beats `cerebras/*`)

### Bug Fixes

- **Title filter used filename** (hq-vault): `title_contains` in `NoteQuery` now checks frontmatter `title` field first, falling back to file stem only when no frontmatter title exists
- **Bare `.unwrap()` on user JSON** (hq-tools): Replaced 7 panic-prone `.unwrap()` calls in `antigravity.rs` with `.unwrap_or("unknown")` or `.context()?`
- **`UnifiedBot::new` required config** (hq-relay): Changed signature to accept `Arc<HqConfig>` parameter instead of silently using `HqConfig::default()` with no API keys
- **`md5_hash` was actually FNV-1a** (hq-memory): Renamed to `content_fingerprint` to match actual hashing algorithm
- **`unwrap()` after `is_some()` guard** (hq-memory): Replaced with proper pattern matching in `chain_insights`
- **Hard-coded GWS_BIN path** (hq-sync): Replaced `/opt/homebrew/bin/gws` constant with `gws_bin()` function that checks Homebrew ARM, Intel, and bare PATH fallback
- **Calendar agenda bare `gws`** (hq-daemon): `get_calendar_agenda` now uses same `gws_bin()` lazy lookup pattern
- **Legacy `.vault` relative path** (hq-daemon): `consolidate_memories` now warns about relative path; callers directed to `run_memory_cycle`
- **Web auth inconsistency** (hq-web): `wa_message_handler` now checks both `x-api-key` and `Authorization: Bearer` headers, matching all other endpoints
- **WhatsApp harness timeout** (hq-web): `spawn_harness_for_wa` now wraps child process in `tokio::time::timeout(300s)`, killing on timeout
- **Python method separator** (hq-codegraph): Changed `extract_python_class_body` from `"::"` to `"."` separator, fixing summarizer method detection
- **Go import qname collision** (hq-codegraph): Import nodes now include line number (`::import::L{line}`) instead of a single `::import` that clobbered all but the last import
- **`#[allow(dead_code)]` on used fields** (hq-agent, hq-llm): Removed incorrect suppressions on `AgentWorker.db` and `ScoredCandidate.cost_tier`
- **`unsafe set_var` in async context** (hq-cli): Moved env file loading to synchronous `fn main()` before tokio runtime starts, eliminating data race with worker threads
- **`unsafe impl Send` undocumented** (hq-audio): Added SAFETY comment explaining three invariants for `RecordingSession`
- **Novel chapter parsing** (hq-novel): Replaced `unwrap_or(0)` with proper error handling using `.context()`
- **`reqwest::Client` TLS panic** (hq-tools): `YahooClient::new()` now returns `Result<Self>` instead of panicking on TLS init failure
- **AIDC unsupported targets** (hq-tools): `AgentDef`/`CliBench` now rejected at creation time with clear error, not at runtime via `bail!`

### Structural Refactors

- **`config.rs` split** (hq-core): 1,051-line monolith decomposed into `config/mod.rs`, `config/crypto.rs`, `config/llm.rs`. All public types re-exported.
- **`types.rs` split** (hq-core): 1,123-line catch-all decomposed into `types/mod.rs`, `types/job.rs`, `types/relay.rs`, `types/session.rs`. All public types re-exported.
- **`lib.rs` split** (hq-web): 1,429-line monolith decomposed into `lib.rs` (router), `ws.rs`, `api.rs`, `mcp_http.rs`, `proxy.rs`, `whatsapp.rs`. All public types re-exported.
- **`parser.rs` split** (hq-codegraph): 1,734-line file decomposed into `parser/mod.rs`, `parser/helpers.rs`, and 7 language modules (`rust.rs`, `typescript.rs`, `python.rs`, `go.rs`, `java.rs`, `c_family.rs`, `ruby.rs`).
- **`querier.rs` split** (hq-memory): 966-line file decomposed into `querier/mod.rs`, `querier/temporal.rs`, `querier/ranking.rs`, `querier/delta.rs`.
- **`drive.rs` split** (hq-sync): 861-line file decomposed into `drive/mod.rs`, `drive/gws.rs`, `drive/scan.rs`.
- **Frontmatter dedup** (hq-core + hq-db): Extracted shared `frontmatter_utils.rs` into `hq-core`, replaced 60-line duplicated parsers in `hq-db/search.rs` with thin wrappers.
- **Daemon utils extraction** (hq-daemon): Duplicated `find_recently_modified`/`visit_md_files` extracted into shared `vault_utils.rs`
- **Tool utility dedup** (hq-tools): `now_iso()`/`generate_id()` extracted into shared `util.rs`
- **Dead code removal** (hq-core): Removed `detect_apple_chip` function duplicating `detect_gpu_type`
- **GovernedRegistry Deref** (hq-agent): Replaced 7 manual delegation methods with `impl Deref<Target = ToolRegistry>`

### Performance

- **Shared HTTP client** (hq-tools): `web_search` and `web_fetch` now use a `LazyLock<Client>` instead of constructing a new `reqwest::Client` per call, enabling TLS session and connection pool reuse
- **Lazy GWS binary lookup** (hq-sync, hq-daemon): `gws_bin()` uses `LazyLock` so the filesystem path check happens only once per process

### Improvements

- **Microcompact dedup** (hq-agent): `AgentSession::microcompact_result` now delegates to canonical `hq_context::microcompact::microcompact` instead of reimplementing with different thresholds
- **Worker clarity** (hq-agent): `unblock_dependents` renamed to `log_completed_dependents` with doc comment explaining the lazy unblocking design
- **Discarded mailbox visibility** (hq-agent): `process_mailbox` now logs at `warn!` level instead of `debug!`, making silently dropped messages visible
- **CLI stubs hidden** (hq-cli): 8 non-functional commands now `#[command(hide = true)]` and print "not yet implemented" instead of faking success
- **Usage subsystem documented** (hq-vault): `get_recent_activity` and `record_usage` annotated as intentionally separate subsystems (`_logs/` vs `_usage/daily/`)
- **Regex `.expect()` clarity** (hq-memory): Dynamic regex in `consolidator.rs` uses `.expect()` instead of bare `.unwrap()`
- **Thread map intent documented** (hq-relay): `ThreadStore::load_index` annotated explaining `chat_id == thread.id` by design

### Stats

- 57 files changed (44 modified, 13 new)
- Net: -7,118 lines (683 insertions, 7,801 deletions)
- Zero new compiler warnings
- All 57 code review issues resolved

## [0.8.1] - 2026-04-03

### New Crates
- **hq-tui**: Full TUI interface built on ratatui/crossterm with markdown rendering, ASCII welcome screen, session browser, cost tracker, task list, progress bars, and themed output

### New Features

#### Native Multi-Model Coding Pipeline (`hq code`)
- `CodingPipeline` in hq-agent: four-phase pipeline (context gather, plan, implement, verify) with per-phase model selection
- `ModelSelector` enum: `FreeFirst`, `BudgetAware`, `LocalFirst`, `Fixed` strategies for routing across free cloud and local models
- Phase 0 (context): file discovery, dependency scanning, codebase structure
- Phase 1 (plan): multi-model planning with adversarial critique (Phase 1.5)
- Phase 2 (implement): code generation with adversarial review (Phase 2.5)
- Phase 3 (verify): mechanical checks (compile, lint, test) with retry loop
- Free-first pipeline routing: common path costs $0 using Cerebras, Groq, and Ollama
- `hq plan` CLI command: standalone adversarial multi-model planning with wall-clock timing, effort estimation (LOC + complexity heuristic), and CC comparison table
- Prior plan learning: loads vault plan snippets as few-shot examples
- Task-type-aware adversarial critique with focused reviews per domain
- Mechanical plan validation: checks paths exist, edits are plausible, test commands are runnable

#### Adversarial Critic
- `adversarial_critic` module in hq-agent: devil's advocate reviews for plans and code
- Wired into pipeline as Phase 1.5 (plan critique) and Phase 2.5 (implementation critique)

#### MCP Coding Tools (21 new tools)
- File I/O: `file_read`, `file_write`, `file_edit`, `file_find`, `file_grep`, `list_dir`
- `file_edit_batch`: atomic multi-file edits in a single tool call
- Git: `git_commit_flow` (composite status+diff+stage+commit), `git_diff`, `git_status`, `git_log`, `git_pr`, `git_issue`
- Session: `session_export`, `session_save`, `session_resume`, `session_list`
- Notebook: `notebook_read`, `notebook_edit`
- Utilities: `clipboard_copy`, `repl_exec`, `native_code`
- 8 existing tools marked `read_only` for parallel execution

#### TUI Chat Interface
- Fullscreen `ChatApp` with ratatui: markdown rendering, ANSI helper, mouse scroll, tab completion
- Slash command registry with 13 built-in commands (`/code`, `/save`, `/resume`, etc.)
- Notification system with message timestamps
- Welcome screen with ASCII logo and keyboard shortcuts
- Session save/resume persistence to vault
- Agent session event renderer, cost tracker, progress bar widgets
- 14+ unit tests covering comprehensive TUI behavior

### Improvements
- **Gemma 4 model migration**: Replaced all local Ollama models from Qwen 3 to Gemma 4 (gemma4:e2b, gemma4:e4b, gemma4:26b) with updated VRAM estimates and 128K context windows
- SBLU trainer pivot models updated to Gemma 4 family
- Planning verification expanded to 23 features (11.5x CC coverage)

### Bug Fixes
- **`hq-context` test failures**: Repaired 2 pre-existing test failures
- **TUI chat loop**: Handle `AppAction::RunPipeline` in CLI event loop

### Infrastructure
- **cargo-gc script**: `scripts/cargo-gc.sh` + weekly launchd job (`com.agent-hq.cargo-gc`) to prevent 100GB+ `target/` bloat from 26-crate debug builds

## [0.8.0] - 2026-04-02

### Breaking Changes
- **Single harness consolidation**: Removed all 7 external CLI harnesses (Claude Code, Gemini, Codex, Opencode, Qwen, Kilo, Mistral Vibe). The internal `AgentSession` is now the only execution path for all agent work.
- **Removed `hq-harness` crate** from workspace. External CLI subprocess spawning is no longer supported. The research loop and sub-agent system use in-process execution.
- **Relay bots simplified**: Discord and Telegram bots no longer offer `!harness`/`!switch` commands. All messages route through the HQ agent.

### New Features

#### Single Harness Architecture
- `SessionBuilder` in hq-agent: centralizes session construction from `HqConfig` with LLM provider, tool registry, governance, and context assembly. Used by CLI, relay bots, and web API.
- `AgentBackend` in hq-relay: replaces `LocalHarness`/`RemoteHarness`/`CloudFallback` with a single implementation backed by `AgentSession`.
- Read-before-write governance: tracks files read during a session and warns when writing to unread files (inspired by Claude Code's staleness detection).
- Result disk-spill: tool outputs exceeding 64KB are written to `/tmp/hq-tool-output-{id}.txt` with a preview sent to the model.
- `SessionEvent::CostUpdate`: emitted after each LLM call with cumulative cost, token counts, and model name.

#### Instance Configuration (Local vs Cloud)
- `InstanceConfig` with `InstanceType` (Local/Cloud) and `InstanceFeatures` (local_ollama, browser_automation, audio_recording).
- Feature-gated Cargo dependencies: `hq-audio` and `hq-browser` are optional via `--features local`. Cloud builds are pure Rust with no C dependencies.
- Conditional tool registration: browser, audio, and meeting tools excluded on cloud instances.
- Instance-aware daemon: meeting watcher, audio tasks, and browser health skipped on cloud.
- Build modes: `cargo build --release` (local, all features) vs `--no-default-features` (cloud VPS).

#### Bidirectional Vault Sync
- `VaultSyncEngine` in hq-sync: detects local changes via vault scanning, applies remote changes with three-way conflict detection, manages persistent sync state.
- `SyncConfig` in hq-core: configurable sync directories, exclusion patterns, and scan intervals.
- Enhanced `BridgeEvent` with `SyncDelta`, `SyncFileRequest`, `SyncFileResponse`, `SyncAck` variants for real-time change propagation over WebSocket.
- Bidirectional bridge: both Mac (LocalBridge) and VPS (GatewayBridge) can send and receive sync events.
- Conflict resolution: last-write-wins with `.conflict-{device_id}.md` backup files.

### Improvements
- `target_harness` field on `Job` deprecated (all jobs use internal agent).
- `DelegateHarness` removed from gateway routing decisions.
- Gateway routing simplified to `SelfHandle`, `DelegateLocal`, `Queue`.
- Discord/Telegram relay bots: removed harness selection UI, always route through HQ.
- CLI harness module: removed streaming NDJSON parser, external CLI builder.

### Bug Fixes
- **UTF-8 panic prevention**: `microcompact_result` (session.rs) and `web_fetch`/`web_search` (web.rs) now use `floor_char_boundary`/`ceil_char_boundary` for safe string slicing, preventing panics on multi-byte characters.
- **Budget session cap enforcement**: `BudgetGuard.can_spend()` now checks the per-session spending cap in addition to the monthly cap. Previously `session_cap` was stored but never enforced.
- **PostToolUse hook feedback delivery**: Block messages from PostToolUse hooks are now appended to the tool result so the model sees them. Previously the feedback was matched but silently discarded.
- **OpenAI o3 pricing**: Corrected from $2/$8 to $10/$40 per million tokens, preventing budget cap miscalculation.
- **Bridge broadcast lag handling**: `connect_and_stream` now logs `RecvError::Lagged` instead of silently dropping events, improving sync reliability diagnostics.
- **Bridge reconnect backoff**: Fixed counter reset ordering so graceful close uses base delay instead of 2x base on the first reconnect.
- **Conflict path generation**: Sync engine now uses `Path::file_stem()` instead of `trim_end_matches(".md")`, correctly handling files with repeated extensions or non-.md files.
- **Telegram presence directory**: Added `create_dir_all` before writing `CHANNEL-PRESENCE.md`, preventing silent write failures when `_system/` does not exist.
- **Relay dead code cleanup**: Removed unused `messages_for_llm` bindings and prefixed unused parameters in Discord/Telegram dispatch functions.

### Code Quality
- Removed ~2000 lines of external harness spawning code.
- Inlined minimal CLI runner for research loop backward compatibility.
- 6 new sync engine tests (create, modify, delete, stable, conflict, directory filtering).
- Suppressed warnings for gateway streaming functions (planned for future use) with `#[allow(dead_code)]`.

## [0.7.3] - 2026-04-02

### New Crates
- **gateway**: Cloud-resident thin orchestrator with `RoutingAgent` (LLM-powered routing), `HealthPoller`, `RequestQueue`, and `VaultCache`
- **hq-audio**: Audio recording pipeline with silence detection, chunked transcription (Whisper via Ollama), and automatic vault ingestion
- **hq-novel**: AutoNovel pipeline for long-form fiction generation with foundation, drafting, revision, and export phases

### New Features

#### Gateway & Streaming
- SSE transport for MCP Streamable HTTP in hq-web
- Harness RPC handler with NDJSON streaming (`/api/harness` route)
- Streaming runner in hq-harness with NDJSON event protocol
- `GatewayConfig` in hq-core for gateway mode configuration

#### Agent System Uplift
- **Coding agent uplift**: Claude Code pattern extraction for tool descriptions and system prompts
- **Typed sub-agents**: `SubagentType` enum (General, Explorer, Planner, Verifier, Coder, Custom) with role-specific prompts, permission modes, and tool allow/deny lists
- **Permission mode system**: DontAsk, Plan, AcceptEdits, BypassPermissions with `DenialTracker` for session-scoped monitoring
- **`GovernedTool` wrapper**: Enforces call limits, path restrictions, and permission modes around every tool execution
- **Session checkpointing**: Append-only NDJSON format with truncation-safe recovery
- **Inter-agent messaging**: `send_message` tool with broadcast support and path-traversal-safe mailbox routing
- **Graceful shutdown pipeline**: Failsafe timers, cross-platform signal handling, orphan detection
- **Git worktree isolation**: Agent sessions can run in isolated worktrees
- **Task graph executor**: Topological sort with cycle detection for multi-step task execution
- Per-message token caching for O(1) estimation
- Feature-gated exact BPE token counting via `tiktoken-rs`
- 10 advanced bash security checks (stacked brace expansion DoS detection, etc.)

#### LLM Providers
- `TurboQuant` provider for local quantized model serving
- `LlmError` structured error enum with `is_transient()` and `is_context_overflow()` helpers
- `LlmRouter` Default impl and improved route resolution

#### Memory
- Transcript extractor for durable memory extraction from agent sessions
- HRR (Holographic Reduced Representations) module improvements

#### Tools
- `PlanDecomposeTool` for decomposing plans into TaskRecord entries with dependency wiring
- `file_edit` utilities: `find_actual_string` (curly-quote normalization), `strip_trailing_whitespace`, `FileStateCache`
- `CliBench` AIDC target kind for CLI benchmark optimization loops
- Expanded planning, skills, browser, financial, and research tool implementations

#### CLI & Infrastructure
- **`hq meeting`**: Meeting intelligence CLI (record, list, transcribe, summary)
- **`hq novel`**: AutoNovel pipeline CLI (init, run, status, export)
- **`hq profile`**: LLM-powered professional profile generation
- **`hq memory entities`**: Entity relationship graph queries
- Dockerfile and fly.toml for containerized deployment
- Meeting watcher, idle alignment, inbox triage, entity graph, presence tools, audio tools, novel tools, diagram expansion, skill hints enrichment

### Improvements
- Codebase-wide `rustfmt` pass for `let-chain` syntax alignment
- `SubagentType::default()` now uses `#[derive(Default)]` with `#[default]` attribute
- Heartbeat system expanded with idle detection and alignment triggers
- Memory ingester/querier/consolidator with entity graph integration
- DrawIt tools refactored with shared rendering logic
- Search: improved FTS5 query parsing and snippet extraction

### Code Quality (2026-04-02 review pass)
- **Microcompact deduplication**: extracted `microcompact_result()` helper in session.rs, replacing 40 lines of copy-paste between `prompt_inner` and `prompt_stream`
- **Daemon scheduler cleanup**: stripped `scheduler.rs` to trait definitions only; removed dead `DaemonScheduler` struct and re-export
- **Deduplicated `is_process_alive`**: removed copy from `memory.rs`, now imports `pub(crate)` version from `instance_lock.rs` with proper `#[cfg(unix)]` guard
- **Eliminated double sysctl subprocess**: `detect_gpu_type()` reuses the brand string instead of calling `detect_apple_chip()` a second time
- **Unified `TURBOQUANT_BASE_URL`**: both `router.rs` and `turboquant.rs` now use `http://localhost:14747/v1` as the canonical default
- **Shared `fnv1a_hash()`**: added to `hq-memory/lib.rs`, replacing identical copies in `querier.rs` (misnamed `md5_hash`) and `transcript_extractor.rs`
- **Removed redundant ALTER TABLE**: `access_count` column was declared in both CREATE TABLE and ALTER TABLE in `db.rs`
- **Removed dead `PendingDelta.candidate_id`**: field was set but never read, had `#[allow(dead_code)]`
- **Removed dead `next_provider()`**: unused round-robin method in `router.rs` diverged from the inline implementation
- **Cloud fallback system prompt rewritten**: VPS agent now correctly describes full CLI/shell access, web search, internet access, and vault cache capabilities; extracted into shared `cloud_system_prompt()` to prevent drift between streaming and non-streaming paths

### Bug Fixes
- **`BypassPermissions` governance**: Now enforces call limits and path restrictions (previously skipped all governance checks)
- **`classify_openai_error` string matching**: Rewrote to use pattern matching on `OpenAIError` enum variants and `ApiError.code` field instead of fragile string-contains on full error messages
- **`find_actual_string` byte corruption**: Fixed byte-index mapping when curly quotes (3-byte UTF-8) are normalized to ASCII (1-byte)
- **Path traversal in `send_message`**: Recipient directory sanitized against `/`, `\`, `..`
- **O(n^2) checkpoint loading**: Eliminated re-counting lines on every corrupt entry
- **Worktree directory conflict**: Creates parent dir only, letting `git worktree add` create the target
- **Transcript extractor budget**: Uses char count instead of byte length for Unicode correctness
- **Task graph panicked tasks**: Panicked tasks now trigger the halt logic instead of being silently dropped
- **`PlanDecomposeTool` persistence**: Decomposed tasks now written to `Plans/{plan_id}/tasks.jsonl` for `TaskGraphExecutor` consumption
- **Training pipeline panics**: Replaced `.parent().unwrap()` with `.context()` error handling on path operations
- **AIDC e2e test**: Fixed directory mismatch (used project ID instead of target ID)
- **Missing `CliBench` match arm**: Added to daemon AIDC periodic task handler
- Fixed unused imports and dead code warnings across multiple crates
- Clippy warnings: derivable Default for SubagentType, collapsible if statements in LlmRouter

### Security
- **Caddyfile untracked**: Removed from git tracking (contained personal Tailscale hostname)
- **Download files gitignored**: Changed `download.txt` to `download.*` pattern

---

## [0.7.2] - 2026-03-23

### New Features
- **`hq mcp-serve`**: Extracted MCP stdio server into dedicated CLI command with proper stderr tracing, replacing inline match arm.
- **`hq mcp doctor`**: New diagnostic subcommand to verify MCP connection health, binary path, and config validity.
- **Dynamic MCP instructions**: Server initialization now sends a full tool catalog (categories + tool descriptions) so agents know available tools without calling `hq_discover` first.
- **Skill-enriched system prompts**: `enrich_system_prompt` now accepts an optional `ToolRegistry` to inject a compact tool catalog alongside skill hints.
- **Registry catalog block**: `ToolRegistry::catalog_block()` generates a compact, category-grouped tool listing for system prompt injection (~50 bytes per tool).

### Security Hardening
- **Pre-commit hook**: Added `scripts/pre-commit` hook that blocks commits containing personal hostnames, hardcoded home paths, API key patterns (AWS, GitHub, Slack), and machine-specific identifiers. Install with `cp scripts/pre-commit .git/hooks/`.
- **Personal data removal**: Removed hardcoded hostname (a personal tailnet hostname) from `vite.config.ts`, replaced with `TLS_HOST` env var. Removed personal paths from `Caddyfile` (now gitignored, template at `Caddyfile.example`). Removed personal vault path from `.gemini/settings.json`.
- **Gitignore hardening**: Added `.claude/`, `.gemini/`, `download.txt`, and `Caddyfile` to `.gitignore` to prevent accidental commit of local settings, confidential documents, and machine-specific configs.

### Improvements
- **Stable MCP binary path**: `hq mcp install` now writes `/usr/local/bin/hq` to MCP configs instead of the current exe path, preventing sandbox SIGKILL issues on macOS.
- **Dynamic gateway schema**: `create_gateway_tools()` now accepts the registry to populate the `hq_discover` category enum from actual registered categories.
- **CLAUDE.md updates**: Updated binary size (20MB), install path (`/usr/local/bin/hq`), crate count (17), added `hq mcp doctor` and post-build install command.

---

## [0.7.1] - 2026-03-22

### New Features
- **`hq install` overhaul**: One-command zero-to-working setup. 7-step automated flow: platform detection, vault scaffolding (39 dirs), rich soul content injection, 6 embedded guide files, config generation with env var auto-detection, tool detection, CAPABILITIES.md generation. Idempotent, `--upgrade` refreshes system files while preserving user edits, `--minimal` for CI.
- **`hq onboard`**: Interactive 6-step walkthrough for progressive feature activation (API keys, agent harnesses, gws CLI with OAuth, MCP server, relay setup, personalization). Resumable via `_system/ONBOARD.md` progress tracker. `--step N` to jump, `--reset` to restart.
- **`hq service`**: New subcommand for service daemon management (launchd/systemd). Replaces the old `hq install` which only did service daemons.
- **Embedded vault content**: SOUL.md, MEMORY.md, PREFERENCES.md, HEARTBEAT.md, CONFIG.md, CAPABILITIES.md, ONBOARD.md, and 6 guide files (getting-started, vault-structure, agent-principles, tool-setup, workflows, memory-system) compiled into the binary via `include_str!()`.
- **Docker e2e test suite**: 108-test Dockerfile.test validates the full install lifecycle (fresh install, idempotency, upgrade, minimal, env var detection, service commands, onboard reset, health, aliases, custom vault paths, content integrity, multiple vaults).

### Security Hardening
- Secret prompting: `hq onboard` now uses `rpassword` to mask API keys and tokens during input (no terminal echo).
- YAML injection: All user-supplied strings (API keys, bot tokens, model names) are escaped before interpolation into config YAML, preventing config corruption via special characters.
- Test fixture privacy: Removed personal name from HRR memory test fixtures.

### Command Changes
- `hq setup` is now an alias for `hq install`
- `hq init` (hidden) delegates to `hq install` for backwards compatibility
- Old `hq install` (service daemons) moved to `hq service install/uninstall/status`

---

## [0.7.0] - 2026-03-22

### New Crates
- **hq-browser**: CDP-based browser automation (navigate, screenshot, evaluate, extract)
- **hq-codegraph**: Tree-sitter code graph with structural summaries and blast radius analysis (6-49x token reduction)
- **hq-harness**: Extracted CLI harness builder/runner from hq-cli

### New Features
- **Multi-agent collaboration**: Bulletin board, curiosity engine, mailbox (point-to-point messaging), heartbeat-based liveness tracking
- **Soul evolution**: Weekly daemon task that learns user behavioral patterns from consolidation insights
- **SBLU trainer**: Dual-gate quality scoring for local model fine-tuning iterations
- **Lesson extractor**: Post-mortem analysis of job exchanges with severity-weighted decay
- **Session summaries**: Automatic session summarization for memory consolidation
- **HRR cache**: Holographic Reduced Representation for fast in-memory fact retrieval
- **Bash policy**: Security denylist for agent bash commands with sanitized environment
- **Subagent support**: Agents can spawn sub-agents for parallel task decomposition
- **Context engine upgrades**: Cascade budget allocation, scoring weights, progressive budget distribution, section-boundary truncation
- **Financial tools**: Market data (Yahoo Finance), Black-Scholes options pricing, correlation matrix
- **Stitch tools**: Google Stitch project/screen management integration
- **CLI adapter tools**: Wrap external CLI tools as HQ tools via YAML specs
- **Checkpoint tools**: Content-addressed vault snapshots with dedup
- **Codegraph tools**: Build, query, impact analysis, and review via MCP
- **Control Center PWA**: Service worker for offline support, push notifications, room and terminal views
- **PTY agent**: WebSocket-based terminal server with tmux session persistence
- **Inbox watcher**: Drop files into `.vault/_inbox/` to create jobs automatically
- **Briefing generator**: Morning briefing from daemon observations

### Security Hardening
- XSS: MarkdownViewer sanitizes HTML via DOMPurify before rendering
- Path traversal: Heartbeat and mailbox modules validate harness IDs
- Shell injection: PTY tmux-control uses `spawnSync` argument arrays instead of `execSync` string interpolation
- Timing-safe auth: PTY API key comparison uses `crypto.timingSafeEqual`
- CORS: hq-web restricts origins to localhost instead of wildcard
- Adapter name validation: CLI adapter save/load rejects path traversal
- Stitch injection: Strict alphanumeric validation on all stitch arguments
- Regex safety: Soul evolution uses `regex::NoExpand` to prevent `$` backreference injection
- LLM fact labeling: `query_with_fallback` prefixes LLM-inferred facts with `[llm-inferred]`
- Personal hostnames removed: Caddyfile and vite.config.ts use env vars
- UTF-8 safety: Fixed 3 byte-indexed truncation panics on multi-byte characters

### Bug Fixes
- SQL alias in WHERE: `query_facts_by_tags` repeated match expression in WHERE (SQLite rejects aliases there)
- Double compilation: `format_compiled_context` called `compile_temporal_context` twice per cache miss
- Fabricated API removed: `finance_hormuz_monitor` returns unavailable status instead of calling non-existent endpoint
- Hash misnames: Renamed `md5_hash` to `prompt_fingerprint`/`content_hash`, switched to stable FNV-1a

### Refactors
- Harness module decomposed into `hq-harness` crate
- Chat panel, stream buffers, thread hooks removed from control center (replaced by room/terminal)
- Budget profiles expanded with cascade weights and scoring parameters
- Memory DB schema extended with lessons, session summaries, HRR tables

---

## [0.6.0] — 2026-03-21

> **Note**: Continues from v0.5.0 (Bun/TS monorepo). This release marks the full Rust rewrite.
> The legacy npm package is deprecated — the canonical distribution is now
> the `hq` binary from [GitHub Releases](https://github.com/CalvinMagezi/hq/releases).

### Major: Modular Architecture Refactor

The monolithic `start.rs` (3,084 lines) has been decomposed into a clean 24-file module hierarchy under `crates/hq-cli/src/commands/start/`.

- **`start/mod.rs`** — Component orchestrator
- **`start/common.rs`** — Shared types (ChannelState, model aliases)
- **`start/agent.rs`** — Job polling worker
- **`start/harness/`** — CLI subprocess runner with 30-minute timeout + NDJSON parsing
- **`start/daemon/`** — Scheduler + tasks organized by cycle speed (fast/periodic/scheduled/slow)
- **`start/relay/`** — Discord + Telegram bot implementations
- **`start/touchpoints/`** — Reactive file-change engine with 6 handlers + synaptic chains

### Major: Touch Points Engine (Restored)

Reactive vault-change handlers wired to `hq-sync::FileWatcher`, restoring the old Bun/TS behavior:

- **frontmatter-fixer** — Adds missing YAML frontmatter on note create/modify, chains to tag-suggester
- **size-watchdog** — Warns at 10KB, alerts at 25KB, writes alert notes at 50KB
- **tag-suggester** — Keyword-based tag suggestion (projects + topics)
- **folder-organizer** — Suggests moves based on frontmatter `project:` field (path-sanitized)
- **conversation-learner** — Extracts decisions/learnings from completed threads
- **stale-thread-detector** — Urgency-aware staleness, auto-archives 3x stale threads
- Config read from `.vault/_system/TOUCHPOINT-CONFIG.md`, logs to `TOUCHPOINT-LOG.md`
- Synaptic chain propagation with depth limit of 3

### Major: Memory System Wired

All `hq-memory` crate subsystems connected to the daemon and agent worker:

- **Consolidation**: `memory-consolidation` daemon task now calls `hq_daemon::run_memory_cycle()` (was a TODO stub)
- **Forgetting**: `memory-forgetting` calls `MemoryForgetter::run_cycle()` — 3-tier synaptic homeostasis decay
- **Ingestion**: Agent worker ingests job results as memories via `MemoryIngester::ingest()` (best-effort, Ollama-dependent)
- **Awake Replay**: Forward replay surfaces precedent memories on job start; reverse replay does credit assignment on job completion
- **Context Engine**: Agent worker now uses the 5-layer `ContextEngine::build_frame()` with SOUL, memory, pinned notes, and budget-aware token allocation

### Major: Morning Brief Pipeline (Restored)

Replaced all three stub tasks with real implementations:

- **6:00 AM EAT** — LLM-generated [S1]/[S2] conversational script + Kokoro TTS audio
- **6:30 AM EAT** — NotebookLM notebook creation with curated sources
- **7:00 AM EAT** — Rich markdown brief with calendar agenda (via `gws`), pending jobs, news highlights

### Added

- **Evening reflection** — New daemon task at 8:45 PM EAT generates introspective self-analysis, ingested as long-term memory
- **Daily synthesis** — LLM-powered reflection on the day's activity (was a static stub)
- **Proactive bot notifications** — 5-minute check for stuck/unclaimed jobs, sends alerts to last-active Telegram channel
- **Channel presence tracking** — Telegram relay writes `CHANNEL-PRESENCE.md` on every message
- **Vault cleanup task** — Daily: reconciles stale jobs (3-day threshold), prunes old touchpoint backups, detects empty stubs
- **Frontmatter audit task** — Daily: backfills tags on 20 untagged notes/cycle using keyword matching
- **FTS5 indexing** — `embeddings` task now indexes 50 notes/cycle into SQLite FTS5 (was a TODO)
- **Graph link building** — `vault-health` task scans wikilinks and populates `graph_links` table
- **Vault health metrics** — Reports wikilink count, dead links, link health percentage
- **5-minute harness heartbeat** — Discord/Telegram show "Still working… (Xm elapsed)" during long harness runs
- **30-minute harness timeout** — Kills runaway CLI harness subprocesses
- **Rust CI workflow** — `cargo check` + `cargo test` + `cargo clippy` + release builds for macOS ARM + Linux x64
- **Regional news feeds** — Added Al Jazeera and TechCabal to news-pulse

### Fixed

- **RSS parsing** — Fixed Atom format support (HN, Guardian now parse correctly)
- **Port binding panic** — Graceful error instead of `unwrap()` on WS port conflict
- **`hq restart`** — Now robustly kills processes by name matching (not just PID files), launches background process
- **`hq stop`** — Three strategies: PID files, `pgrep` name matching, port freeing via `lsof`
- **`hq` command** — Unified binary name (`hq` primary, `hq-rs` backward-compat symlink)
- **`.gitkeep` false positive** — Proactive check now filters non-`.md` files
- **Stale job threshold** — Reduced from 7 days to 3 days

### Security

- Removed hardcoded personal identity from LLM prompts (now reads from gitignored SOUL.md)
- Added path traversal validation in `parse_skill()` and `folder_organizer`
- Bot token no longer leaked in download error messages
- Hardcoded `localhost:5678` replaced with `config.ws_port`

### Removed

- Placeholder daemon tasks that did nothing: `sblu-retraining`, `team-optimizer`, `plan-extraction`
- Ghost touchpoint configs: `news-clusterer`, `news-linker`, `news-digest` chain (not implemented)
- `MORNING_BRIEF_ENABLED` env var gate (morning brief is now a core feature)

## [0.1.0] — 2026-03-20

Initial Rust rewrite. Single binary replacing the Bun/TypeScript monorepo.
