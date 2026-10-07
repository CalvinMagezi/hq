# Capability seams: Service Definition / Provider / Consumer (hq-tools audit)

> **Historical audit (2026-09-25).** The Definition / Provider / Consumer
> pattern described next is still how new capabilities should be built, and
> `hq-llm::LlmProvider` is still the reference example. The module-by-module
> audit below is mostly history: `hq-tools/src` now holds about 30 modules,
> not ~79. Removed since the audit, with their seams: `cursor_agent.rs`,
> `claude_code.rs`, `notebooklm.rs`, `novel/*`, `crypto.rs`,
> `remote_mcp.rs`, `memory_tool.rs`,
> `trace_concept.rs`, `obsidian.rs`, `cards.rs`, `cli_adapter/`,
> `learn_skill.rs`, `vault_publish.rs`, `checkpoints.rs`, `codegraph.rs`,
> `dev_tools.rs`, `coding/pipeline.rs`, `opencode_runner.rs`, `pi_runner.rs`,
> `acp.rs`, `coding_agents.rs`, `quota.rs`, `harness_tools.rs`,
> `harness_router.rs`, `tmux.rs`, `missions.rs`, `aidc.rs`, `benchmark.rs`,
> `webmail.rs`, `financial.rs`, `tts.rs`, `drawit.rs`, `canvas.rs`,
> `stitch.rs`, `schedule.rs`, `meeting_notes.rs`, `shortcuts/{fleet,codegraph,dev}.rs`
> and the former host singleton module. So the `MemoryStore`, `ConceptTraceProvider`,
> `ObsidianBridgeProvider`, `AdapterExecutor` and `UrlFetcher` seams no
> longer exist in code. Rows for removed modules are kept as the record of
> what was decided; check `ls crates/hq-tools/src` before trusting a path.

Full audit of `hq-tools`' ~79 modules (55k lines) for the Service Definition
/ Provider / Consumer split, inspired by DeepSeek's Cordis plugin framework
(`deepseek-ai/deepseek-harness`) and matching a pattern agent-hq already gets
right in one place.

## The pattern, in agent-hq's own vocabulary

- **Definition** — a trait that is the contract. Nothing outside the trait
  method signatures is assumed.
- **Provider** — a concrete struct implementing the Definition. There can be
  more than one; a new one should never require touching the Consumer.
- **Consumer** — the model-facing `AgentTool`/`HqTool` surface (schema +
  dispatch). It depends only on the Definition, never on which concrete
  Provider is behind it.

**Reference example**: `hq-llm::LlmProvider`
(`crates/hq-llm/src/provider.rs`):

```rust
pub trait LlmProvider: Send + Sync {
    fn name(&self) -> &str;
    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse>;
    async fn chat_stream(&self, request: &ChatRequest)
        -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>>;
}
```

Definition = the trait. Providers = OpenRouter/Ollama impls. Consumer =
anything calling `.chat()`/`.chat_stream()` — it never branches on which
vendor answered. A new LLM vendor is a new impl, zero Consumer changes.

`hq-agent::backend::SessionBackend` (in-process vs. external-CLI harness
execution) is a second existing example of this pattern already done right —
out of scope for this audit, not re-derived here.

## Classification tiers

- **(a) already cleanly separated** — a real trait/struct boundary exists; a
  second Provider could be swapped in without touching the tool struct or
  its JSON schema.
- **(b) coupled but low-risk/cheap to split** — no trait boundary yet, but
  the concrete backend logic (inline HTTP/DB/subprocess calls) is small and
  self-contained enough that extracting a `XxxProvider`/`XxxBackend` trait
  is a mechanical, behavior-preserving refactor with real future payoff.
- **(c) coupled and high-risk to split, or single-provider-forever by
  design** — either the coupling is deep (shared mutable state, heavy
  external-process lifecycle, DB-schema-bound), or there is no realistic
  second provider ever likely to exist (dsh's own rule: don't split
  preemptively for a capability that is definitionally singular, e.g. "the
  vault," "this mission-tracking schema"). (c) is a legitimate, common
  answer here, not a cop-out.

## Seam-by-seam classification

### Vault & knowledge

| Seam | Files | Class | Reasoning |
|---|---|:-:|---|
| Vault core | `vault.rs` | c | Thin wrappers over one `VaultClient`; vault semantics are definitionally singular. |
| Vault reorg/publish | `vault_reorg.rs`, `vault_publish.rs` | c | Backend-specific orchestration (DB sync, one publish script), not a reusable seam. |
| Memory / session search | `memory_tool.rs`, `session_search.rs` | **b → done** | Was direct file/SQLite I/O, no trait boundary. Split into `MemoryStore` and `SessionSearchProvider`; see below. |
| Concept tracing | `trace_concept.rs` | **b → done** | Was a thin wrapper around `trace_concept()`/`resolve_code_refs_for_concept()` called directly from the tool. Split into `ConceptTraceProvider`; see below. |
| Obsidian bridge | `obsidian.rs` | **b → done** | Was six tools each doing detect-mode-then-raw-HTTP inline, zero tests. Split into `ObsidianBridgeProvider`; see below. |
| Model cards | ~~`cards.rs`~~ (removed) | **b → done, then retired** | Was six tools each wrapping raw `hq_db::cards::*` calls in their own `spawn_blocking`, zero tests. Split into `ModelCardProvider`; see below. The entire ModelCard system (this file, `hq_db::cards`, `hq_core::types::ModelCard`, `hq-agent`'s `card_picker`) was deleted in the VPS-era simplification pass — see `CLAUDE.md`'s retirement note. This row and the `ModelCardProvider` detail below are historical record only. |
| Skills catalog/load | `skills.rs` | **b → declined** | Looked like a provider split, wasn't one; see below. |
| Skill management | `skill_manage_tool.rs` | **b → declined** | Same verdict as `skills.rs`, same reason: plain `tokio::fs` mutation against `skills_dir`, no external backend, already covered by 5 tests against a real `tempfile::tempdir()`. |
| Skill minting | `learn_skill.rs` (retired, pruned for zero calls) | **b → done, then removed** | Only its `source: "url"` path crosses a real backend (live HTTP fetch); `text`/`transcript` need no I/O and `file` reads the local vault, already tempdir-tested for real. Split a narrow `UrlFetcher`, not the whole tool; see below. |
| Prose lint | `prose_lint.rs` | c | Already a clean consumer of `hq_core::prose_quality::SlopDetector`; the rule-set *is* the backend. |
| Brand assets | `brand.rs` | c | File-backed knowledge tied to vault paths by design. |
| NotebookLM brief | `notebooklm.rs` | c | Heavy `nlm` CLI process lifecycle + vault synthesis; splitting is plumbing, not payoff. |
| Checkpoints | `checkpoints.rs` | n/a | Data types only, no `AgentTool`/`HqTool`. |

**Implemented as a follow-up pass**: the three smallest (b)-tier candidates
here — `MemoryStore { read/write(path) }` (`crates/hq-tools/src/memory_tool.rs`,
capacity/match logic stays in the Consumer, only file I/O moved behind the
trait), `SessionSearchProvider { search(filters) }`
(`crates/hq-tools/src/session_search.rs`), and `ConceptTraceProvider { trace(seed, depth) }`
(`crates/hq-tools/src/trace_concept.rs`) — following the `AdapterExecutor`
template exactly: a Provider struct wrapping today's single concrete backend
(`FsMemoryStore`, `SqliteSessionSearchProvider`, `VaultConceptTraceProvider`),
a `#[cfg(test)] with_store`/`with_provider` injection constructor, and a fake
Provider test per seam proving the tool never touches the real filesystem/DB/
vault in that test. Existing behavior-level tests were left untouched (only
`memory_tool.rs`'s and `trace_concept.rs`'s free-function tests, which don't
go through the tool at all, plus `session_search.rs`'s SQLite-backed tests,
which still exercise `SqliteSessionSearchProvider` for real).

`ObsidianBridgeProvider { search/read/write/patch/tags/open }`
(`crates/hq-tools/src/obsidian.rs`) followed the same template in a second
pass: one operation per tool rather than raw HTTP verbs, so mode detection
and the Live-vs-HqLocal gating (`obsidian_patch` erroring in HqLocal,
`obsidian_search`'s dataview-mode check, `obsidian_open`'s deliberate
"not running" *success* response) live in the Provider next to the
detection that decides them. Kept as a single `HttpObsidianBridge` rather
than the `LiveObsidianProvider`/`HqLocalObsidianProvider` split floated
below — mode selection happens per-call inside one provider, not as two
swappable backends, matching the pre-split code's own `detect_mode()`
shape. Not cached (each call still probes Obsidian fresh); caching would
change behavior (missing Obsidian starting/stopping mid-session) and was
left as a `TECHDEBT.md` candidate, not folded into this split.

`ModelCardProvider { list_cards/get_card/get_synergies/top_synergies/
upsert_card/write_vault_card }` (`crates/hq-tools/src/cards.rs`) followed
the same "only I/O moves behind the trait" rule as `MemoryStore`: comparison
math (`compare_models`'s per-stat winner selection), leaderboard bar
rendering, and the seeding loop over `hq_llm::models::all_known_models()`
all stayed in the Consumer. Each provider method preserves the original's
`tokio::task::spawn_blocking` wrapping around `Database::with_conn` — this
crate's DB access was never made non-blocking, so that has to survive the
split, not get dropped for a cleaner-looking trait signature.

Known tradeoff, not fixed: `compare_models` and `seed_model_cards`
previously ran their per-model `get_card`/`upsert_card` calls inside one
shared `spawn_blocking`/`with_conn` (one pooled connection, looped over
in-process); each is now a separate `self.provider.*` call, so a compare
of N models or a seed of the full known-models registry now does N (or
2N, seed also writes) pool checkouts and blocking-task spawns instead of
one. `compare_models` is bounded to a handful of user-supplied IDs and
`seed_model_cards` is a rare one-time operation, so the overhead is
small in practice — but it's a real change from the original code, not
just a rename, and wasn't called out when this split shipped.

**Declined**: `SkillProvider { list()/load(name) }` for `skills.rs` looked
like the same shape as the four splits above but isn't one. Its backend is
`list_skills(&dir)`/`parse_skill(&dir, name)` reading files under a
`skills_dir` — already fast, deterministic, and offline to test directly
against a `tempfile::tempdir()` (exercising the real frontmatter parser,
where skill bugs actually live), so a fake provider would test nothing the
direct call doesn't already cover more faithfully. Worse, `list_skills`/
`parse_skill`/`enrich_system_prompt`/`validate_skills` have twelve direct
callers across five crates (`hq-agent`, `hq-cli`, `hq-web`, `hq-relay`) —
a trait covering only `ListSkillsTool`/`LoadSkillTool` wouldn't abstract
anything the other ten respect. And the one genuinely untested behavior,
`LoadSkillTool`'s miss path (writes `_gaps/{slug}.md`, guarded so a repeat
miss never clobbers it, returns `Ok(json!({"error": ...}))` — a *success*
response, not an `Err`, the same shape as `obsidian_open`), is Consumer-side
logic a `list`/`load` trait wouldn't reach anyway. Added tempdir-based
tests for that behavior directly instead (`skills.rs`'s `mod tests`); no
trait, no injection constructor.

`skill_manage_tool.rs` (`SkillManageProvider` in the original proposal) is
**declined** for the identical reason: `create`/`write_file`/`remove_file`/
`delete` are plain `tokio::fs` calls against `skills_dir`, already covered
by 5 tests against a real `tempfile::tempdir()` exercising the real
name/file validation and `_proposed/` staging logic. No external backend
to fake.

(Retired: `learn_skill.rs` and its `UrlFetcher` were later pruned for zero calls.)
`learn_skill.rs`'s `UrlFetcher { fetch(url) }` was **done**, but narrower
than the `SkillProposalProvider { learn(...) }` originally proposed here —
wrapping the whole tool would have faked argument parsing and the local
vault-file read (`file` source), neither of which needed it (the `file`
path is already vault-tempdir-tested for real, same reasoning as the two
declines above). Only `source: "url"`'s live HTTP fetch crosses a real
backend boundary, so only that got a trait; `HttpUrlFetcher` is the sole
real implementation. Two new tests: a fake fetcher proving the full
fetch→truncate→preview→description→proposal-write pipeline without
network, and a failing fetcher proving a fetch error surfaces as
`Ok(json!({"error": ...}))`, not `Err` — matching `obsidian_open`'s and
`LoadSkillTool`'s success-not-error shape.

Every (b)-tier candidate in this section has now been dispositioned (done
or declined, with reasoning above) — none remain proposed-but-unactioned.

### Coding & dev-agent runners

| Seam | Files | Class | Reasoning |
|---|---|:-:|---|
| Coding core I/O | `coding/{mod,file_ops,edit,git,shell,session,todo}.rs` | a | Tool structs are MCP façades only; real work already lives in `file_edit`, `TodoStore`, git/shell helpers. |
| Coding pipeline/config | `coding/pipeline.rs` | a | Delegates plan generation to `hq_llm`; consumer already separated. |
| Code graph | `codegraph.rs` | retired | Removed with the `hq-codegraph` crate on 2026-09-24. |
| Dev orchestration | `dev_tools.rs`, `dev_pipeline.rs`, `dev_harness.rs` | a | `Dev*Tool` only enqueue/query/cancel `DevJob`; execution isolated in `dev_pipeline::run_job`. |
| File editing substrate | `file_edit.rs` | a | Shared primitive module, not a tool file — already the backend boundary others use. |
| **CLI adapter framework** | `cli_adapter/*` | **b → done** | Generic spec/schema layer, but execution was hardcoded to shell/HTTP/browser functions with no trait. **Split implemented this audit** (see below). |
| Cursor runner | `cursor_agent.rs`, `cursor_runner.rs` | **b → declined** | Not a trait candidate; a real duplication problem instead. See below. |
| Claude Code runner | `claude_code.rs`, `claude_runner.rs` | **b → declined** | Same finding as cursor; same fix. See below. |
| OpenCode runner | `opencode_runner.rs` | c | Single binary, direct `Command` spawn, no realistic second backend. |
| Pi runner | `pi_runner.rs` | c | Direct protocol runner, no multi-provider story. |
| ACP integration | `acp.rs` | c | Single-protocol integration by design. |
| Coding agent dispatcher | `coding_agents.rs` | **b → declined** | Subprocess (`cli_runner`) and quota (`quota.rs`, already class `c`) backends were already factored out; the actually-untested part was pure routing logic, now extracted and tested directly. See below. |
| Task classifier | `task_classifier.rs` | c | Pure decision utility, not a provider boundary. |

**Flagship finding, implemented this audit**: `cli_adapter` was a generic
*definition* layer (adapter YAML spec, param templating, security) with no
generic *execution* layer — `CliAdapterRunTool::execute` matched
`CommandType::{Shell,Http,BrowserPipeline}` and called
`execute_shell`/`execute_http`/`execute_browser_pipeline` inline, while
`cursor_agent.rs`/`claude_code.rs`/`opencode_runner.rs`/`pi_runner.rs` each
kept independent subprocess-management logic entirely outside `cli_adapter`.
Added `AdapterExecutor` trait (`crates/hq-tools/src/cli_adapter/executor.rs`)
— `async fn execute(&self, spec, cmd, args) -> Result<String>` — with
`DefaultAdapterExecutor` as the sole Provider today (the same three engines,
now dispatched through the trait instead of matched inline).
`CliAdapterRunTool` now holds `executor: Arc<dyn AdapterExecutor>` (Consumer
side); a `#[cfg(test)] with_executor()` constructor lets tests inject a fake
Provider and prove the tool never spawns a real shell/HTTP call — see
`cli_adapter::tools::run_tool_executor_injection_tests`. Behavior for the
three existing engines is unchanged.

**Cursor/Claude runners, declined**: on inspection this section's own
framing ("tool still owns process lifecycle") wasn't the actual problem.
`tmux_session_alive`/`tmux_pane_pid`/`tmux_kill_session`/`tmux_send_keys`
were four functions copied verbatim (down to matching comments) across
`cursor_agent.rs`, `claude_code.rs`, *and* `harness_session/mod.rs` — a
third copy the doc didn't even list here. Each is a one-shot status check
or command against a process-global named tmux session (`TMUX_SESSION`
consts, `SessionState` statics); there's no second backend to swap in, so
a `TmuxController`-style trait would fake nothing the four-line bodies
don't already say. Moved all three copies to a shared `crate::tmux`
module instead (deduplication, not abstraction) — `session_alive`,
`kill_session`, `pane_pid`, `send_keys`, `pub(crate)`. `harness_session`'s
`tmux_session_alive`/`tmux_kill_session` stayed `pub` as thin forwarders,
since `hq_tools::harness_session::{tmux_session_alive,tmux_kill_session}`
is real external API (`hq-cli`'s `session_supervisor.rs`). The one
non-trivial piece — `pane_pid` parsing `tmux list-panes` output, first
line of possibly several, numeric-or-None — was split into a private
`parse_pane_pid` and given 4 tests (multi-pane, no trailing newline,
non-numeric, empty). `TMUX_SESSION` singletons, `SessionState`, spawn
orchestration, and `run_cursor_agent_headless`/its Claude Code equivalent
were left untouched — genuinely singular system-resource control, not a
provider seam, and the NDJSON parsing beneath them already has
golden-fixture tests.

**Update 2026-09-18, tmux replaced by a host, then by the built-in host (2026-10)**:
the "no second backend" premise above no longer holds, and `crate::tmux` is deleted.
Sessions run in a host, and there is a real second backend: the same calls reach the
host on this machine over its unix socket, or one on a paired machine over ssh through
`hq host gate`. Both are `NativeBackend` (`agent_host/native.rs`) behind the
`HostBackend` trait; the ssh half lives in `agent_host/transport.rs`. Tests fake the
boundary with `ScriptedHost` (`agent_host/scripted.rs`, behind the `test-support`
feature) for the session logic, and with a real in-process `hq_host::Server` for the
backend itself.

`coding_agents.rs` (multiplexes codex/qwen/cursor/claude), now examined:
same "declined for a trait, real fix was elsewhere" shape as the
runners above, for a different reason. Its "hardcoded inline"
per-tool logic was already mostly factored: subprocess execution goes
through the shared `cli_runner::{run_plain_text,run_stream_json}`, and
quota tracking through `quota.rs` (already classed `c` in this doc's
Tools & agent runtime section, below). What was genuinely untested —
and risky to get wrong, since it's the exact table a future contributor
edits when adding an agent — was `CodingAgentDispatchTool`'s selection
logic: `prefer_agent` validation order (unknown → not-installed →
quota-exhausted → in-cooldown), and task_hint-based auto-ordering
(`fix`/`test` → codex-first, else cursor-first) gated by installed +
quota + cooldown. Extracted as a pure `choose_agent(statuses,
status_json, prefer, task_hint, in_cooldown) -> Result<&'static str,
Value>` — no DB, no trait, no subprocess; `in_cooldown` is a plain
`impl Fn(&str) -> bool`, the real `agent_in_cooldown` free function at
the call site and a closure over a fixed set in tests. 10 tests pin the
routing table directly, including the one order-dependent case a
careless refactor would silently break: an agent that is both
not-installed *and* in cooldown must report `agent_not_installed`, not
`agent_in_cooldown`. The `AGENT_COOLDOWN` static, the four `*_binary()`
discovery functions, `record_perf`, and the per-agent quota/subprocess
dispatch arms were left untouched — the same "singular system-resource
control, not a provider seam" reasoning as the tmux singletons above.
Proposed shapes for that follow-up: `CursorBackend { create_chat, run_headless }`,
`ClaudeBackend { spawn_session, run_headless }`,
`CodingAgentBackend { name, install, check, quota, chat, run_task }` (impls
for Codex/Qwen/Cursor/Claude).

### Harness dispatch / sub-agents / missions

| Seam | Files | Class | Reasoning |
|---|---|:-:|---|
| Harness dispatch/status | `harness_tools.rs`, `harness_router.rs`, `harness_chunk.rs` | **b → declined** | Grouped three files that don't share this problem; see below. |
| Harness session manager | `harness_session/{mod,spec,tools}.rs`, `agent_host/` | **b → done (2026-09-18)** | Now runs on the host with a local/ssh host split (`NativeBackend`); see the update under the cursor/claude runners above. |
| Agents catalog | `agents.rs` | **b → declined** | Same shape as `skills.rs`: filesystem reads, tempdir-testable directly. Found and fixed a real bug along the way; see below. |
| Agent mailbox comms | `agent_comm.rs` | c | Canonical vault mailbox is intentionally singular. |
| Missions | `missions.rs` | c | Mission state is a specific DB schema + executor semantics. |
| AIDC | ~~`aidc.rs`~~ (removed) | c | One concrete optimize/evaluate/iterate pipeline by design. The whole `hq-aidc` crate and this MCP tool file were deleted in the VPS-era simplification pass (low value in practice) — see `CLAUDE.md`'s retirement note. Row kept as historical record. |
| Self-update lifecycle | `self_update.rs` | c | Tightly bound to repo checkout, cargo, binary snapshot/restore. |
| Benchmark | `benchmark.rs` | c | Single external integration path (OpenRouter judge loop). |
| Quota | `quota.rs` | c | Helper-only DB logic, no tool surface. |
| Absorb pipeline | `absorb/{mod,triage,analyze,plan}.rs` | c | One-off repo-ingest pipeline; a trait split would just rename stages. |
| Shortcuts: fleet | `shortcuts/fleet.rs` | **b → done** | Already had an unused `with_client` injection point and an unused `wiremock` dev-dependency; wired up. See below. |
| Shortcuts: vault | `shortcuts/vault.rs` | **b → partial** | `VaultFindShortcut` declined (shared, already-tested DB function); `VaultNoteShortcut`/`VaultLogShortcut` were genuinely untested local fs logic, pinned directly. See below. |
| Shortcuts: codegraph | `shortcuts/codegraph.rs` | retired | `hq_find_code` was removed with the `hq-codegraph` crate on 2026-09-24. |
| Shortcuts: dev | `shortcuts/dev.rs` | c | Cargo invocation *is* the tool. |
| Shortcuts: gateway | `shortcuts/gateway.rs` | c | Resolver/dispatcher over other shortcut tools, not its own seam. |

**Harness dispatch/status, declined**: this row bundled three files that
turned out not to share the problem. `harness_router.rs` (965 lines) has
no `HqTool` impl at all — it's a pure scoring/routing library
(`HarnessRouter`, `CapabilityProfile`, `HarnessMetrics`), already carrying
12 tests. `harness_chunk.rs` is one pure function, already 2 tests. Only
`harness_tools.rs` actually has the `execute()`-hardcodes-HTTP problem
this row named, and there it's thinner than `ObsidianBridgeProvider`'s
case justified: `HarnessDispatchTool`/`HarnessStatusTool`'s HTTP calls are
build-JSON-body → POST/GET → map a connection failure to a friendlier
message → return the response as-is, with no live/local mode gating or
success-shaped-as-error behavior to lose in a refactor. A
`HarnessDispatchBackend { dispatch, status }` trait would fake a POST call
with almost no decision logic behind it — not worth it. What genuinely
needed coverage were two already-pure, already-untested helper functions:
`build_dispatch_body` (hint field included/omitted) and
`collect_subtask_outcome` (its `latency_ms` fallback: use the harness
response's own value when present and positive, else the measured
wall-clock elapsed time) — 5 tests added directly, no trait.

**Harness session manager, declined**: `tools.rs`'s 7 tool structs are
truly thin (parse args, call one `super::` function; no logic to test).
The real logic is in `mod.rs`'s `spawn`/`resume`/`status`/`list`/`send`/
`stop`/`tail_log` — but that's tmux+DB session-lifecycle orchestration
against a single named tmux pane per session (create, poll liveness,
kill), the same "genuinely singular system-resource control, no second
backend to swap in" shape declined for the cursor/claude runners above.
`spec.rs` (harness definitions: binaries, resume strategy, ready/trust/
token regex patterns) is pure static config, already 4 tests. `mod.rs`'s
own genuinely-pure logic, `build_args` (resume-token substitution,
session-dir arg injection per `ResumeStrategy`), was already 4 tests
before this pass. The one real gap: `harvest_resume_token` regex-matched
a harness's token straight out of `captures_iter(...).last()` inline,
untested — a session's logfile can carry several stale tokens from
earlier resumes, and only the *last* match is still valid, which is
exactly the kind of one-word-different-behavior (`.next()` vs `.last()`)
a careless edit could flip silently. Extracted as pure `fn
extract_resume_token(pattern: &str, content: &str) -> Result<Option<String>>`
and given 2 tests (last-match-wins with two tokens present, none-when-
absent) — no trait, matching `harness_tools.rs`'s and `tmux.rs`'s
`parse_pane_pid` precedent of pinning the one genuinely pure, previously
untested piece rather than wrapping the I/O around it.

(2026-09-18: the logfile is gone with tmux. `harvest_resume_token` now
scans the screen text the supervisor already read, and `pane_pid` /
`parse_pane_pid` were deleted with `tmux.rs`. `extract_resume_token` and
its tests are unchanged.)

**Agents catalog, declined**: `AgentCatalog { list, load }` for `agents.rs`
was proposed here on the same reasoning already retired for `skills.rs` —
`parse_agent_file`/`list_agents` read files under an `agents_dir`,
tempdir-testable directly and more faithful than a fake (exercises the
real gray_matter/serde_yaml frontmatter parser). `ListAgentsTool`/
`LoadAgentTool` had zero external callers, unlike `skills.rs`'s twelve, so
the "wouldn't abstract anything other callers respect" objection doesn't
even apply here — this one is just filesystem work that's cheap to test
for real, full stop. Added 8 tempdir tests, since the file had none.

Investigating those tests surfaced a real, unrelated bug worth calling
out: `AgentDefinition::instruction` (`hq-core/src/types/mod.rs`) had no
`#[serde(default)]`. `parse_agent_file` deserializes *only the YAML
frontmatter* into `AgentDefinition` — `instruction` is filled in from the
markdown body afterward, so it's never present in the frontmatter map.
A required field missing from the input makes `serde_yaml::from_value`
fail entirely, and the caller's `.ok().unwrap_or_else(...)` fallback then
discarded every other frontmatter field (`displayName`, `tags`,
`baseRole`, `preferredHarness`, `preferredModel`, `maxTurns`, `autoLoad`,
`fallbackChain`) on *every* real agent file — silently, since `name` gets
overridden from the filename regardless and `vertical` gets separately
inferred from the directory name, so nothing about the tool's output
looked wrong without inspecting those specific fields. Confirmed with a
throwaway test before fixing (frontmatter tags came back `[]`), fixed by
adding `#[serde(default)]` to `instruction` (and `name`, for the same
reason, though it's less exposed), and the fix is now the regression test
`frontmatter_fields_survive_parsing`. Surfaced to the user before fixing,
consistent with how the `brand.rs`/Caddyfile findings were handled
earlier in this pass — user chose fix-now.

**Shortcuts: fleet, done**: `FleetBackend` wasn't needed as a new trait —
`DelegateTool`/`RunParallelTool` already had `with_client(ws_port,
reqwest::Client)` constructors sitting unused, and `wiremock` was already
an unused dev-dependency in this crate's `Cargo.toml`. Someone had set up
the injection seam and never followed through with tests. Added 8 tests:
`wiremock`-backed success/non-success-status/invalid-JSON-response paths
for `dispatch_task`, a real-connect-refused test (point `with_client` at
port 1, nothing listens there) proving the "fleet dispatch not available"
friendly-error path specifically (not the generic error branch), and the
pure argument-validation paths (empty task, fewer than 2 tasks, a blank
task in the list) that need no HTTP at all.

**Shortcuts: codegraph, declined (retired 2026-09-24)**: `HqFindCodeTool`'s two-stage fallback
(codegraph DB index query, then a `grep -r` subprocess) is real external-
backend orchestration — `hq_codegraph::graph::query_nodes` is a whole
separate crate, already presumably tested at its own layer, and a trait
here would mostly be faking "here are some nodes" with no interesting
gating logic behind it (unlike Obsidian's mode detection). What was
genuinely untested was `extract_keywords`, the pure stop-word-filtering/
punctuation-stripping/lowercasing function that both fallback stages key
off of — 4 tests added directly (stop-word removal, punctuation
stripping keeps mid-token underscores, order preservation, all-stop-words
yields empty).

**Shortcuts: vault, partial**: `VaultFindShortcut` wraps
`hq_db::search::keyword_search`, which has 21 tests of its own in
`hq-db/src/search.rs` and 5 callers beyond this one — `hq-tools`'s own
`vault.rs`, two in `hq-web`, one in `hq-cli`, one in `hq-daemon` — the
same "shared function, wouldn't abstract anything other callers respect"
reasoning that declined `skills.rs`. Declined a trait there; the tool's own wrapping is a
default-limit and an empty-results message, too thin to bother pinning
separately. `VaultNoteShortcut` and `VaultLogShortcut`, though, are local
to this file, have no shared backend, and had zero tests despite real
logic: title inference (strip leading `#`, truncate to 60 chars,
`"Untitled"` fallback when nothing usable remains), path-traversal
stripping applied to *both* folder and title (`..`/`/`/`\` all stripped,
not rejected — verified the write still lands under `Notebooks/` even
when every input is adversarial, and falls back to `Inbox` when
stripping empties the folder entirely), and `vault_log`'s create-with-
header-then-append-without-repeating-it two-branch logic for the daily
log file. 7 direct tempdir tests, no trait — matches `VaultNoteShortcut`'s
own class (`skill_manage_tool.rs`'s "pure filesystem mutation" shape,
just previously untested where `skill_manage_tool.rs` already had 5).

This closes out every (b)-tier candidate in the "Harness dispatch /
sub-agents / missions" section of this doc.

### Integrations, comms, and misc capability

| Seam | Files | Class | Reasoning |
|---|---|:-:|---|
| Registry / lib | `registry.rs`, `lib.rs` | a | `HqTool` is the contract; `ToolRegistry` is a pure catalog, no backend branching. |
| Browser | `browser/*` | a | Tools are thin consumers over `SessionManager`/`ExtBridge`/CDP; backend work delegated to `hq-browser`. |
| Novel pipeline | `novel/*` | **b → declined** | Misdiagnosed: `LlmProvider` already exists as a trait upstream (`hq_llm`); the cloud branch is an honestly-stubbed incomplete feature, not a missing seam. One pure unit (`budget.rs`) pinned; see below. |
| GWS / webmail / Trello / remote MCP | `gws.rs`, `webmail.rs`, `trello.rs`, `remote_mcp.rs` | c | Intentionally single-integration seams — the direct HTTP/bridge I/O is the product. |
| Financial / research / web | `financial.rs`, `research.rs`, `web.rs` | c | Domain-specific fetch/analysis; a provider split would be speculative. |
| Imagegen / TTS / audio | `imagegen.rs`, `tts.rs`, `audio.rs` | c | Single-service/pipeline capabilities by design. |
| Drawit / canvas / stitch / convert | `drawit.rs`, `canvas.rs`, `stitch.rs`, `convert.rs` | c | Orchestration/format-conversion tools, no meaningful provider seam. |
| Crypto / system info / presence / capture | `crypto.rs`, `system_info.rs`, `presence.rs`, `capture.rs` | c | Trivial utility tools. |
| Schedule / workflow | `schedule.rs`, `workflow.rs` | c | Stateful orchestration, not multi-provider adapters. |
| Meeting notes / meetings | `meeting_notes.rs`, `meetings.rs` | c | Cohesive single-domain capture/transcribe/notes tools. |
| Planning | `planning/*` | c | Domain pipeline, not an integration seam. |
| Safehouse | `safehouse/*` | c | Single-purpose security boundary; should stay cohesive. |
| Computer use | `computer_use/*` | a | Consumer wrappers over a local automation backend, already separated. |
| Earn | `earn/*` | c | Business logic/state types, no provider split needed. |
| Slash commands / util | `slash_commands.rs`, `util.rs` | c | Glue/helpers, not a backend seam. |

**Novel pipeline, declined**: `pipeline.rs::get_provider()` does hardcode
`OllamaProvider::new()` on both branches of `if self.project.config
.prefer_local`, exactly as this row said — but the fix isn't a new trait.
`LlmProvider` already exists as a trait in `hq_llm`, and `OllamaProvider`
already implements it; the `else` branch is an explicit, honest stub —
`// STUB: cloud provider not implemented, falling back to Ollama`, logged
via `tracing::warn!` every time it's hit. Wiring in a real cloud
`LlmProvider` (picking a provider, API keys, config plumbing) is a
feature addition, not a refactor, and out of scope for a seam-review
pass. `run_job_collaborative` carries the same honesty: `// TODO: Replace
with AgentSession-based collaborative execution` and an explicit `bail!`
rather than silently degrading.

The wider `novel/*` module (1404 lines across 12 files: `drafter.rs`,
`evaluator.rs`, `exporter.rs`, `foundation.rs`, `revision.rs`,
`state.rs`, `tools.rs`, `types.rs`, `prompts.rs`) had zero tests
anywhere before this pass — not a seam gap, a coverage gap, and a much
larger undertaking than this row's scope (most of those files involve
real LLM calls, vault I/O, or the pipeline's state machine, each needing
the same per-file due diligence as everything else in this doc, not a
single follow-up commit). One file was a clean, self-contained,
zero-I/O exception: `budget.rs`'s `BudgetManager` (cost accumulation by
harness/phase, budget-limit boundary check, tier-based model selection)
— pinned directly with 6 tests, no trait needed. The rest of `novel/*`
is explicitly **not** covered by this entry; treat it as a separate,
larger test-coverage project if picked up later, not as done.

This closes out the last (b)-tier row in this document — every seam
candidate across all sections is now either done, declined with
reasoning, or (for `novel/*`'s untested remainder) explicitly flagged as
out of scope rather than silently skipped.

Novel's (b) candidate: inject `Box<dyn hq_llm::LlmProvider>` into
`FoundationGenerator`/`ChapterDrafter`/`Evaluator`/`RevisionEngine` instead of
letting `pipeline.rs`'s `NovelOrchestrator` select an `OllamaProvider`
fallback itself — reuses the existing `LlmProvider` Definition rather than
inventing a parallel one. Deferred to a follow-up PR (touches the whole
pipeline's construction, not a single-file change).

## What was actually split in this audit

Given the size of this seam (79 modules, ~15 (b)-tier candidates found), the
full set is being landed as separate, reviewable PRs per seam per the plan's
own risk note, not as one diff. **This pass implements the single flagship
finding** — `cli_adapter`'s `AdapterExecutor` split — as the reference
example for the rest: real duplication found, single-file scope, existing
test coverage to prove behavior preservation, immediate payoff (a fake
executor can now be injected in tests instead of every test needing a real
shell/HTTP/browser engine).

**Follow-up pass**: `MemoryStore`, `SessionSearchProvider`, and
`ConceptTraceProvider` (the memory/session-search/concept-tracing row above)
landed next, same template, same one-seam-per-PR discipline.

Remaining (b)-tier items above are documented with proposed trait shapes so
each can be picked up independently without re-auditing. (c)-tier items are
intentionally left alone — dsh's own rule against preemptive splitting for
genuinely single-provider capabilities.
