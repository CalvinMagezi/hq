# Multi-harness dispatch / sub-task fleet — Retired 2026-09-21

**Status: fully removed from the codebase.** This is a historical record,
not a description of anything currently running. Do not follow any command
or code reference below — every one of them refers to code that no longer
exists.

## Why it was retired

HQ is now its own single harness everywhere it is interacted with — web,
Telegram, Discord, and the CLI. The two things this doc describes both
worked against that: automatic per-message routing across external
harnesses (Telegram's `select_harness`, Discord's inline `!harness` match,
`config.active_harness`), and an explicit sub-task fan-out fleet
(`hq dispatch`, `/v1/fleet`, the OpenAI/Anthropic-compat proxy, the
`harness_dispatch`/`harness_status` MCP tools) for routing cheap reasoning
work to whichever of a dozen external CLIs/providers was free or fast. Both
fragmented the implementation across three inconsistent switching
mechanisms (Telegram's per-channel pin, Discord's system-wide config write,
`hq agent`/`hq chat`'s CLI-level Pi wrapper) instead of scaling hq's own
native capability. The only axis of fallback that remains is LLM
*provider*-level (`/backend`, `hq-llm`'s provider chain) — a different
concern (which LLM backend answers) from "harness" (which execution engine
answers).

The host (`docs/AGENT_SESSIONS.md`) is unrelated and unaffected: it runs actual
coding-agent CLIs (Claude Code, Codex, Cursor, …) as long-lived panes by
deliberate, explicit user request, not as an automatic routing layer HQ's
own responses moved through.

## What was removed

**Automatic routing (relay/web layer):**
- Telegram: `select_harness`, `within_stickiness_window`,
  `telegram_can_invoke`, the `/harness` pin command, `dispatch_harness`/
  `dispatch_cursor`/`dispatch_pi`/`dispatch_claude_code`/
  `dispatch_external_or_hq`, and the self-learning feedback chain
  (`report_hq_turn`, the 👍/👎 `handle_reaction` handler, the
  `reaction_quality` module) — all in `crates/hq-relay/src/telegram/`.
- Discord: the inline `match harness.as_str()` dispatch and the `!harness`
  command in `crates/hq-relay/src/discord.rs`.
- `crates/hq-web/src/ws.rs` and `api.rs`: the `active_harness == "pi"` and
  `!is_native_harness` (external-harness-via-proxy) branches in the chat
  handlers — the native `AgentSession` path was already a strict superset
  (it had thread sync and event types the other two paths lacked).
- `hq_core::config::HqConfig::active_harness` (and the deprecated,
  already-dead `coding_default_agent` predecessor), `current_harness()`.
- `ChannelState.pinned_harness`/`last_routed_harness`/`last_routed_at`
  (`crates/hq-relay/src/relay_common.rs`) — `ChannelState.harness` itself
  stays (status-display use only, always `"hq"` now).

**CLI-level switching (explicit, not automatic — removed on the same
reasoning: full commitment to a single harness, including the CLI):**
- `hq agent <harness>` (`crates/hq-cli/src/commands/agent.rs`) and its
  `Commands::Agent` variant.
- `hq chat`'s `maybe_delegate_pi_interactive` Pi-wrapper and its own
  in-REPL `/harness` slash command (`crates/hq-cli/src/commands/chat.rs`).

**The sub-task fan-out fleet:**
- `crates/hq-web/src/harness_proxy.rs` (2239 lines, trimmed to ~220 keeping
  only `dispatch`/`dispatch_stream` for the native chat path, then deleted
  outright once `ws.rs`/`api.rs` stopped calling into it at all),
  `fleet.rs`, `fleet_cards.rs`, `harness_rpc.rs`.
- `crates/hq-tools/src/harness_router.rs` (`HarnessRouter`, the learned
  per-task-type quality scorer) and `harness_tools.rs`
  (`HarnessDispatchTool`/`HarnessStatusTool`, the MCP tools).
- `AgentService::for_harness_dispatch`/`for_harness_dispatch_with` and
  `HarnessBackend` (`crates/hq-agent/src/backend/harness.rs`) — the
  in-process translation target the deleted `harness_dispatch_handler` used;
  orphaned once that handler was gone.
- `hq-cli`'s `dispatch`/`harness` subcommands
  (`hq dispatch`/`hq dis`, `hq harness {status,list,check,switch,probe}`).
- `crates/hq-daemon/src/proxy_logger.rs` (the hourly `DISPATCH-POLICY.md`/
  `HARNESS-ROUTING.md` writer) and `crates/hq-db/src/proxy_log.rs`.
- DB tables `proxy_calls` (migration `046_drop_proxy_calls.sql`) — single-user
  instance, no external readers, dropped rather than kept as an unread
  historical table.
- The web dashboard's dispatch-policy/scores/task-scoreboard/audit endpoints
  and their handlers in `crates/hq-web/src/lib.rs`/`api.rs`.

**Factory/missions**, HQ's separate autonomous self-improvement pipeline,
was retired in the same pass for an unrelated reason (HQ no longer uses
missions to improve its own codebase or any other codebase) — see the
`045_drop_missions_factory.sql` migration; it is not part of the harness
story above, just adjacent in time. It was the one dependency keeping
`HarnessRouter` alive after the fleet proxy itself was cut, so its removal
is what let `HarnessRouter` go too.

`hq_tools::research.rs` also defines a struct literally named `HarnessRouter`
— an unrelated escalation-level routing type for the research loop, not
part of any of this. It was not touched.

## What replaced it

Nothing — there is nothing to route. Every interaction surface calls hq's
own native `AgentSession`/`SessionBuilder` directly. If a future need for
cheap-model fan-out reappears, design it fresh against the current
`AgentService`/`spawn_subagents` substrate rather than reviving anything
described above.
