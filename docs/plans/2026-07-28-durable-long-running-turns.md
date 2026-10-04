> Historical design record; the code may have changed since. This plan shipped; paths such as `native_hq.rs` and `discord.rs` are now `native_hq/` and `discord/`.

# Durable Long-Running Turns Implementation Plan

> **For agents:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Relay turns never die at a hardcoded 270s wall. Turns that outlive the ack window detach into supervised background tasks whose results are delivered asynchronously, sub-agent completion can wake a parked parent, and daemon restarts re-attach stranded monitoring sessions instead of orphaning them.

**Architecture:** Three layers. (1) Detach-on-timeout: the four synchronous turn guards gain a configurable ack window; on expiry the in-flight session moves to a persisted `background_turns` registry and the chat gets an immediate ack plus async delivery on completion. (2) Durable substrate: work expected to run hours-to-days is promoted to tmux-backed harness sessions / mission steps (already restart-proof and resumable), with a supervisor patience window of days rather than a tokio future. (3) Wake-up and recovery: a completion bus lets child agents, harness sessions, and background turns post results that resume or notify the originating thread; on daemon boot a reconciler re-attaches orphaned tmux sessions to monitor turns and marks interrupted in-process turns with a resume affordance.

**Tech Stack:** Rust (edition 2024), tokio, SQLite (vault.db, WAL, migrations in `crates/hq-db/sql/`), teloxide (Telegram), tmux.

**Decisions (confirmed with the owner 2026-07-28):**
- Interactive chat model: detach pattern. Short ack window, async delivery. No literal multi-hour held-open turns.
- Sub-agent wake-up gap is in scope (verified: `AgentService::execute` is fully synchronous, no completion-notification path exists).

---

## Current state (verified 2026-07-28)

Hardcoded turn guards:
- `crates/hq-relay/src/telegram/session.rs:1625` — `timeout: Some(Duration::from_secs(270))` on `run_native_hq`
- `crates/hq-agent/src/native_hq.rs:214-229` — on timeout, drops the `session.prompt_stream` future, returns "Request timed out. Try breaking it into smaller steps."
- `crates/hq-relay/src/session_runner.rs:62` — `SESSION_TIMEOUT = 270s`
- `crates/hq-relay/src/discord.rs:1013` — `SESSION_TIMEOUT = 270s`
- `crates/hq-cli/src/commands/start/relay/agent.rs:72` — `SESSION_TIMEOUT = 180s`

Existing machinery to reuse:
- Harness-session manager: `crates/hq-tools/src/harness_session/` (tmux spawn, registry in `harness_sessions` table, resume tokens, supervisor `session_supervisor.rs` reconciles every minute). Survives daemon restarts today.
- Config: `crates/hq-core/src/config/relay.rs:31` `RelayConfig` (add new fields here).
- DB migrations: next number is `034` (`crates/hq-db/sql/`).
- Thread recording: `record_thread_turn` in `native_hq.rs` already persists turn outcomes.

Gaps confirmed:
- Dropping the tokio future on timeout kills the in-process agent loop; harness sessions it spawned lose their monitor and stray.
- `AgentService::execute` (`crates/hq-agent/src/agents/service.rs`) joins all children synchronously inside the parent's tool call. No way for a child to finish later and wake the parent or the chat.
- No persistence of in-flight relay turns; a daemon restart loses everything in memory.

---

## Phase 1: Configurable ack window + detach-on-timeout (Telegram first)

### Task 1: Add config fields

**Objective:** Make the ack window and detach behavior configurable.

**Files:**
- Modify: `crates/hq-core/src/config/relay.rs` (RelayConfig at :31)

**Implementation:**

```rust
/// Seconds a relay turn may run before it detaches into a background
/// task and the chat receives an async result. Default 270 (legacy behavior).
#[serde(default = "default_turn_ack_timeout_secs")]
pub turn_ack_timeout_secs: u64,

/// Maximum days a detached background turn may run before the supervisor
/// fails it. Default 5.
#[serde(default = "default_background_turn_max_days")]
pub background_turn_max_days: u64,
```

with `fn default_turn_ack_timeout_secs() -> u64 { 270 }` and `fn default_background_turn_max_days() -> u64 { 5 }`. Verify `RelayConfig` derives `Deserialize` with serde defaults; add the two fns beside existing defaults in that file.

**Test:** config parse test in `hq-core` — a YAML without the new fields deserializes to 270 and 5.

**Commit:** `feat(hq-core): configurable relay ack window and background-turn max age`

### Task 2: `background_turns` table

**Objective:** Persist detached turns so they survive restarts.

**Files:**
- Create: `crates/hq-db/sql/034_background_turns.sql`
- Create: `crates/hq-db/src/background_turns.rs` (mirror the shape of `harness_sessions_registry.rs`)

**Implementation:**

```sql
CREATE TABLE IF NOT EXISTS background_turns (
    id TEXT PRIMARY KEY,
    platform TEXT NOT NULL,           -- telegram | discord | web | cli
    chat_id TEXT NOT NULL,
    thread_id TEXT,
    identity TEXT,                    -- caller identity for thread recording
    prompt TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'running',  -- running | completed | failed | interrupted
    created_at INTEGER NOT NULL,
    completed_at INTEGER,
    result_text TEXT,
    child_session_ids TEXT,           -- JSON array of harness session ids
    cancel_token TEXT
);
CREATE INDEX IF NOT EXISTS idx_background_turns_status ON background_turns(status);
```

Registry module: `insert`, `get`, `list_running`, `mark_completed`, `mark_failed`, `mark_interrupted`, `attach_child_session`. Follow `harness_sessions_registry.rs` patterns (same pool access, same error style).

**Test:** insert/list/complete round-trip against an in-memory or temp-db pool, mirroring existing registry tests in `hq-db`.

**Commit:** `feat(hq-db): background_turns registry for detached relay turns`

### Task 3: Detach path in `run_native_hq`

**Objective:** On ack-window expiry, keep the session alive in the background instead of dropping it.

**Files:**
- Modify: `crates/hq-agent/src/native_hq.rs:209-231`
- Modify: `crates/hq-agent/src/native_hq.rs:32` (`NativeHqHooks`)

**Implementation:** Add to `NativeHqHooks`:

```rust
/// When set and the ack window expires, the session continues in the
/// background; on completion the sink receives the final text for
/// async delivery. When None, legacy kill-on-timeout behavior applies.
pub on_detached: Option<DetachedTurnSink>, // type alias for Arc<dyn Fn(DetachedTurnOutcome) + Send + Sync>
pub turn_id: Option<String>,               -- background_turns row id, registered by caller
```

Rework the timeout branch: instead of `tokio::time::timeout` dropping the future, `tokio::select!` on (a) the ack window timer and (b) `session.prompt_stream(prompt)`. If the timer fires first and `on_detached` is set, spawn the `prompt_stream` future into a supervised task (store the `JoinHandle` in a global detach registry keyed by turn id), return a `NativeHqResult` with a new `detached: true` flag and ack text like "This needs longer than a chat turn. Parked as task `{id}`, I'll report back when it's done." If `on_detached` is None, keep the legacy behavior exactly.

**Test:** a hooks-equipped run with a stub backend that sleeps past the ack window returns `detached: true` promptly, the background task completes, and the sink fires with the final text. Stub backends exist in `agents/tests.rs` (see the `work: Duration` sleeper pattern).

**Commit:** `feat(hq-agent): detach-on-timeout with completion sink in run_native_hq`

### Task 4: Wire the Telegram path

**Objective:** Telegram turns detach and deliver asynchronously.

**Files:**
- Modify: `crates/hq-relay/src/telegram/session.rs:1612-1643`

**Implementation:** Before `run_native_hq`, insert a `background_turns` row (platform telegram, chat/thread ids, prompt). Pass `on_detached`: a sink that (a) marks the row completed with result text, (b) sends the result as a new message to the chat via the bot (the module already holds bot handles for the ticker), (c) records the thread turn. On `detached: true`, send the ack text instead of the timeout text. Use `config.relay.turn_ack_timeout_secs` for the timeout value.

**Test:** integration-style test is heavy here; at minimum unit-test the sink closure's DB transitions with a mock sender. Manual verification: send a sleep-inducing prompt over Telegram, watch ack arrive at the window, result arrive later.

**Commit:** `feat(hq-relay): telegram turns detach past the ack window and deliver async`

### Task 5: Port to Discord, session_runner, CLI relay

**Objective:** Same behavior on the other three guards.

**Files:**
- Modify: `crates/hq-relay/src/discord.rs:1013`
- Modify: `crates/hq-relay/src/session_runner.rs:133`
- Modify: `crates/hq-cli/src/commands/start/relay/agent.rs:97`

Same pattern as Task 4. If a surface lacks an easy async-send handle, deliver via the existing relay notification path used by `session_supervisor.rs` FYI posts.

**Commit:** `feat(hq-relay): detach-on-timeout across discord, web session runner, cli relay`

## Phase 2: Sub-agent wake-up

### Task 6: Async mode for `spawn_subagents`

**Objective:** Children can run past the parent's turn and report back.

**Files:**
- Modify: `crates/hq-agent/src/agents/types.rs` (plan/spec types)
- Modify: `crates/hq-agent/src/agents/service.rs` (`execute`)
- Modify: `crates/hq-agent/src/agents/tool.rs` (tool schema)

**Implementation:** Add `blocking: bool` (default true = current behavior) to the plan. When `blocking: false`, `execute` registers each child in `background_turns` (or a sibling `background_children` table keyed to the parent turn id), spawns the JoinSet into the supervisor, and the tool returns immediately with child ids. On each child completion, post to the completion bus (Task 7) with the parent turn/chat identity.

**Test:** extend `agents/tests.rs` — a non-blocking parallel plan returns immediately, both sleeper children complete, and two completion events arrive on a test bus.

**Commit:** `feat(hq-agent): non-blocking spawn_subagents with completion events`

### Task 7: Completion bus + parent wake-up

**Objective:** A single channel where background children, detached turns, and harness sessions report completion, and the originating thread gets the result.

**Files:**
- Create: `crates/hq-agent/src/completion_bus.rs` (tokio broadcast channel + DB-backed event log for restart durability)
- Modify: `crates/hq-relay/src/telegram/session.rs` (subscribe; on event for this chat, either inject into a live parked turn or send a new message)

**Implementation:** Event shape: `{ source: ChildAgent|DetachedTurn|HarnessSession, parent_turn_id, chat routing, text, success }`. Delivery rule: if the parent turn is still parked (row status running), append the child result and, when all children are done, resume a follow-up turn with the child results injected as context ("your sub-agents finished, here are their reports, continue"). If the parent already completed or was interrupted, deliver a standalone notification.

**Test:** bus unit tests (publish/subscribe, persistence replay after simulated restart) plus one end-to-end: parked parent + two children → single wake-up turn with both reports.

**Commit:** `feat(hq-agent,hq-relay): completion bus wakes parked turns with child results`

## Phase 3: Strand recovery

### Task 8: Startup reconciler

**Objective:** On daemon boot, no session is left strayed.

**Files:**
- Modify: daemon startup (find where `session_supervisor.rs` is spawned; co-locate the reconciler)
- Create: `crates/hq-daemon/src/turn_reconciler.rs`

**Implementation:** On boot: (1) `list_running` from `background_turns` — rows whose in-memory task is gone (always, after a restart) get marked `interrupted` unless they own live tmux harness sessions; (2) cross-reference `child_session_ids` against the `harness_sessions` table and live tmux (`has-session`) — live orphans get a fresh monitor turn registered that waits on exit and delivers the result to the original chat; (3) interrupted turns post one message per chat: "N tasks were interrupted by a restart. Reply 'resume {id}' to retry with prior context."

**Test:** seed a temp DB with running rows + fake tmux sessions, run the reconciler, assert correct interrupted/re-attached classification.

**Commit:** `feat(hq-daemon): startup reconciler re-attaches orphaned sessions, flags interrupted turns`

### Task 9: `resume <id>` command on relay surfaces

**Objective:** User can restart an interrupted turn without retyping context.

**Files:**
- Modify: Telegram/Discord command handlers (wherever `/`-commands or prefix commands are parsed)

**Implementation:** `resume <id>` loads the row (prompt + partial result + child session ids), re-dispatches through `run_native_hq` with history seeded from the original prompt plus a "previous attempt was interrupted at {time}, partial progress: {result_text}" preamble, then updates the row to running with a new attempt. Reject resume of rows not in `interrupted`/`failed`.

**Commit:** `feat(hq-relay): resume command restarts interrupted background turns`

### Task 10: Enforce the 5-day ceiling + docs

**Objective:** Supervisor fails turns older than `background_turn_max_days`; document the model.

**Files:**
- Modify: `session_supervisor.rs` (or turn_reconciler periodic pass): age check, mark failed, notify chat
- Modify: `README.md` or `docs/` — the turn lifecycle doc: ack window → detach → completion bus → reconcile
- Update stale enumerations: grep for the old "3 minutes" timeout strings and replace with the configurable wording.

**Commit:** `feat(hq-daemon): enforce background-turn max age; docs for turn lifecycle`

## Verification checklist (whole feature)

- [ ] `cargo test -p hq-db -p hq-agent -p hq-relay -p hq-daemon` green
- [ ] Manual: Telegram prompt that runs > ack window → ack message, then async result
- [ ] Manual: kill daemon mid-background-turn → restart → interrupted notice, `resume` works
- [ ] Manual: non-blocking sub-agent spawn → parent parks → children finish → parent wakes with reports
- [ ] No stray `hq-*` tmux sessions without a registry row after any of the above

## Status

Implemented 2026-07-28: ack-window detach on telegram/discord/web/cli, background_turns registry, startup + periodic reconciler, resume <id> command, non-blocking spawn_subagents with child-completion delivery.
