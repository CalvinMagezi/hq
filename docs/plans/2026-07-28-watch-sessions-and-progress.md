> Historical design record; the code may have changed since. This plan shipped; paths such as `native_hq.rs` and `discord.rs` are now `native_hq/` and `discord/`.

# Watch Sessions and Progress Heartbeats Implementation Plan

> **For agents:** Use subagent-driven-development skill to implement this plan task-by-task. Implementers write code only (max `cargo fmt`); the orchestrator compiles, tests, reviews, and commits. Never `git add -A` (peer has unrelated uncommitted work).

**Goal:** Parked turns report progress while they run, and "watch X and keep me posted" becomes a durable, restart-proof recurring turn instead of one frozen silent turn.

**Architecture:** Two mechanisms on top of the background_turns registry (migration 034). (1) Progress: a ProgressSink on NativeHqHooks, fired by a heartbeat ticker in the detached supervisor and by a new `report_progress` agent tool. (2) Watch: rows with `kind='watch'` carry an interval and expiry; a relay-side scheduler polls for due watches and re-dispatches the stored prompt through the normal dispatch path, so watches survive restarts by construction.

**Tech Stack:** Rust (edition 2024), rusqlite, teloxide, serenity, tokio.

**Context:** Phase 1 (durable long-running turns) is committed. Registry API in `crates/hq-db/src/background_turns.rs`: insert/get/list_running/mark_completed/mark_failed/mark_interrupted/attach_child_session/mark_running/list_recent_interrupted. Hooks in `crates/hq-agent/src/native_hq.rs`: NativeHqHooks { on_detached, on_child_completion, turn_id, detached flag }. Detached supervisor spawns the prompt future and calls the sink on completion. Telegram wiring in `crates/hq-relay/src/telegram/session.rs` (~1610-1740), Discord in `crates/hq-relay/src/discord.rs`. Config knobs in `crates/hq-core/src/config/relay.rs` (turn_ack_timeout_secs, background_turn_max_days). Migrations live in `crates/hq-db/sql/`, next number is 035. Reconciler in `crates/hq-daemon/src/turn_reconciler.rs`.

**Registry-lifecycle invariant (from phase 1):** any row created for work that may finish synchronously MUST be closed on the synchronous path too, or the reconciler reports phantoms.

---

### Task 1: Watch columns + registry helpers (hq-db, migration 035)

**Objective:** background_turns can represent recurring watch turns.

**Files:**
- Create: `crates/hq-db/sql/035_background_turn_watches.sql`
- Modify: `crates/hq-db/src/background_turns.rs`
- Modify: migration registry where 034 is listed (find via grep `034_background_turns`)

**Migration:**

```sql
ALTER TABLE background_turns ADD COLUMN kind TEXT NOT NULL DEFAULT 'turn';
ALTER TABLE background_turns ADD COLUMN watch_interval_secs INTEGER;
ALTER TABLE background_turns ADD COLUMN watch_until INTEGER; -- unix epoch, NULL = no expiry
ALTER TABLE background_turns ADD COLUMN watch_last_fired INTEGER; -- unix epoch
```

**New helpers (same `&rusqlite::Connection` style as siblings):**

```rust
pub fn insert_watch(conn, id, platform, chat_id, thread_id, identity, prompt, created_at, interval_secs, watch_until) -> Result<()>
pub fn list_due_watches(conn, now: i64) -> Result<Vec<BackgroundTurn>> // kind='watch', status='running', watch_last_fired + watch_interval_secs <= now
pub fn mark_watch_fired(conn, id, fired_at: i64) -> Result<()>
pub fn mark_cancelled(conn, id, completed_at: i64) -> Result<()> // status='cancelled'
```

`BackgroundTurn` struct gains the four fields; update row-mapping in `get`/`list_running`/etc. `insert` keeps working (kind defaults to 'turn'). Reconciler-visible statuses unchanged ('cancelled' is terminal, never picked up by list_running/list_due_watches).

**Tests (in background_turns.rs mod tests, follow existing test style with temp db):** insert_watch + list_due_watches returns due only; mark_watch_fired pushes it out of the due set; mark_cancelled removes from list_running; plain insert rows map with kind='turn' and NULL watch fields.

---

### Task 2: Progress sink + heartbeat in the detached supervisor (hq-agent)

**Objective:** a detached turn emits a heartbeat event every `progress_interval_secs` while it runs.

**Files:**
- Modify: `crates/hq-agent/src/native_hq.rs`
- Modify: `crates/hq-agent/src/builder.rs` if hooks are constructed there

**Design:**

```rust
pub struct ProgressEvent {
    pub turn_id: String,
    pub elapsed_secs: u64,
    pub note: Option<String>, // Some(_) = volunteered by agent via report_progress; None = heartbeat tick
}
pub type ProgressSink = Arc<dyn Fn(ProgressEvent) + Send + Sync>;
```

`NativeHqHooks` gains `on_progress: Option<ProgressSink>` and `progress_interval_secs: Option<u64>`. In the detached supervisor (the spawned task that awaits the prompt future after detach), wrap the prompt future in a tokio::select! loop with `tokio::time::interval` firing at progress_interval_secs; on each tick call the sink with `note: None`. Interval only starts ticking after detach (heartbeats are pointless before the ack message). Default-construct the new fields everywhere NativeHqHooks is built (there is a precedent commit "default-construct NativeHqHooks for new detach fields").

**Test:** existing native_hq tests must pass unchanged; add one test driving the detached path with a 1-tick interval and a slow fake prompt future, asserting a heartbeat event fired before completion (follow existing detach test style from phase 1).

---

### Task 3: `report_progress` agent tool (hq-agent)

**Objective:** the agent can volunteer a substantive progress note mid-turn.

**Files:**
- Modify: `crates/hq-agent/src/agents/tool.rs` (or wherever built-in tools are registered; grep `spawn_subagents` tool registration)
- Modify: `crates/hq-agent/src/agents/types.rs` if tool context needs the sink

**Design:** new tool `report_progress` with args `{ message: string }`. The tool's context gains `progress_sink: Option<ProgressSink>` and `turn_id: Option<String>` (same threading pattern as completion_sink in phase 1 Task 6). On call: if sink present, fire ProgressEvent { note: Some(message) } and return "Progress reported."; else return "Progress reporting not available in this context." Register in the default tool list with a description that tells the model to use it during long tasks to update the user. No-op safety: must not panic when sink is None.

**Test:** call the tool with and without a sink; assert event fired / graceful no-op.

---

### Task 4: Telegram + Discord progress wiring (hq-relay)

**Objective:** heartbeats and notes reach the chat without spamming.

**Files:**
- Modify: `crates/hq-relay/src/telegram/session.rs`
- Modify: `crates/hq-relay/src/discord.rs`

**Design:** when constructing NativeHqHooks for a turn, set on_progress:

- note: Some(text) → send chat message "Task `<id>`: <text>" (truncate ~3500 chars, same as completion sink), record in thread log, best-effort.
- note: None (heartbeat) → send "Task `<id>` still running (elapsed <Xm>)." ONLY if elapsed >= heartbeat_floor. Heartbeats must be quiet by default: telegram sink skips heartbeats entirely unless `progress_interval_secs >= 120` was configured (the supervisor interval is the throttle; sinks don't add extra throttling). Keep the exact policy: send every heartbeat the supervisor emits; the supervisor interval controls cadence.

Wire `progress_interval_secs` from config (`relay.background_progress_secs`, Task 7). All sends best-effort with tracing::warn on failure, never blocking the turn.

**Test:** none new here (surfaces are thin); rely on compile + live smoke.

---

### Task 5: `/watch` and `/unwatch` commands (hq-relay, telegram + discord)

**Objective:** users start and stop durable watch turns from chat.

**Files:**
- Modify: `crates/hq-relay/src/telegram/commands.rs` (command registration + help)
- Modify: `crates/hq-relay/src/telegram/session.rs` (handler, near the resume handler)
- Modify: `crates/hq-relay/src/discord.rs`
- Modify: `crates/hq-relay/src/relay_common.rs` if parsing is shared (resume parsing lives there; follow that pattern)

**Syntax:** `/watch <minutes> <prompt>` and `/watch <minutes> for <hours>h <prompt>` (keep parsing minimal: first token = interval minutes integer 1..1440; optional `for <N>h` = expiry; rest = prompt). `/unwatch <id>`.

**Semantics:**
- `/watch 15 check whether the agy review is done` → insert_watch (kind='watch', interval 900s, watch_until = now + for-hours or NULL), reply "Watching every 15m as task `<id>`: '<excerpt>'. /unwatch `<id>` to stop." Then dispatch the prompt immediately once through the normal dispatch path (first firing is synchronous-ish, detaches naturally past the ack window like any turn).
- `/unwatch <id>` → get; wrong chat/platform → refuse; not a watch or not running → report status; else mark_cancelled + "Stopped watch `<id>`."
- Guards: interval clamped to [1, 1440]; prompt non-empty else usage text.

**Tests:** parsing unit tests in relay_common (valid forms, bad interval, missing prompt), same style as resume parsing tests.

---

### Task 6: Watch scheduler in hq-relay

**Objective:** due watches re-dispatch automatically, surviving restarts.

**Files:**
- Modify: `crates/hq-relay/src/telegram/session.rs` and `crates/hq-relay/src/discord.rs` (spawn scheduler task alongside the listener), OR a shared `crates/hq-relay/src/watch_scheduler.rs` used by both (prefer shared module; both surfaces have db Arc and a dispatch entry point).

**Design:**

```rust
pub async fn run_watch_scheduler(
    db: Arc<hq_db::Database>,
    platform: &'static str,
    poll_every: Duration, // 30s
    dispatch: impl Fn(WatchDispatch) -> Fut, // re-dispatch through the surface's normal path
)
```

Loop: `list_due_watches(now)` filtered to this platform → for each: check expiry (watch_until <= now → mark_completed with final text "Watch `<id>` expired." delivered to chat, skip dispatch) → else mark_watch_fired(now) BEFORE dispatch (so a crash doesn't double-fire) → dispatch stored prompt as a new turn carrying the watch's turn_id (results deliver to the same chat; each firing closes its own registry lifecycle like a normal turn). Errors: tracing::warn and continue; one bad watch must not stall others.

Restart safety: scheduler starts with the relay; due watches fire on the first poll. This replaces any "resume watches on startup" logic; the reconciler (phase 1) MUST NOT mark running watches interrupted (Task 7).

**Test:** scheduler tick logic as a pure function over a temp db (due selection, expiry close-out, fired-marking) with a recording dispatch closure; no real telegram.

---

### Task 7: Reconciler + config updates (hq-daemon, hq-core)

**Objective:** watches coexist with the stranded-turn reconciler; new knob documented.

**Files:**
- Modify: `crates/hq-daemon/src/turn_reconciler.rs`
- Modify: `crates/hq-core/src/config/relay.rs`

**Changes:**
- Reconciler: `list_running` rows with kind='watch' are NOT marked interrupted (the relay scheduler owns their lifecycle). Watches with watch_until older than `background_turn_max_days` ARE interrupted silently (abandoned). Add a test row of each kind to the reconciler tests.
- Config: `background_progress_secs: Option<u64>` default 300 on RelayConfig, next to the phase-1 knobs, with a YAML round-trip test.

---

### Task 8: Docs + plan status

**Objective:** keep docs truthful.

**Files:**
- Modify: `AGENTS.md` (one line on watch turns near the background_turns line)
- Modify: `CLAUDE.md` (extend the long-turns bullet with watch + progress)
- Modify: `README.md` (extend the Long-running turns subsection: heartbeats, report_progress, /watch //unwatch)
- Modify: this plan file, append `## Status` paragraph when done.

---

## Execution notes

- Sequencing: 1 → 2,3 (parallel ok) → 4 → 5 → 6 → 7 → 8. Task 6 depends on 5's dispatch entry points; Task 7 can run any time after 1.
- Live smoke at the end: ack window still at 15s in ~/.hq/config.yaml (restore to 270 after all smoke tests), `/watch 2 <prompt>` in Telegram, observe two firings + heartbeat + /unwatch.

## Status

Implemented 2026-07-28: watch columns on background_turns (migration 035), progress heartbeats + report_progress tool, telegram/discord progress delivery, /watch and /unwatch commands, relay watch scheduler with restart-safe re-dispatch, watch-aware reconciler, background_progress_secs config.
