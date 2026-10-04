# Middleware seams (Cortex-style adoption)

This document maps **natural handler boundaries** in Agent-HQ where composable middleware (`Chain`, retry/timeout policy, typed errors, auth gates) can wrap existing logic without rewriting domain code.

References: shared kernel in `hq-core::middleware` ([`crates/hq-core/src/middleware/`](../crates/hq-core/src/middleware/)).

## hq-agent

| Seam | Location | Today | Middleware opportunity |
|------|-----------|--------|-------------------------|
| Turn loop | [`crates/hq-agent/src/session/loop.rs`](../crates/hq-agent/src/session/loop.rs), `run_turns`, `build_turn_request` | Build request, model, tools, append; compact and retry once on context overflow | Outer `Gate`s for memory/skills/context; loop boundary for budget/rate limits |
| Tool dispatch | [`crates/hq-agent/src/session/routing.rs`](../crates/hq-agent/src/session/routing.rs), `execute_tool`, `execute_tools_parallel` | Hooks, policy, registry; timing via [`middleware_runtime.rs`](../crates/hq-agent/src/middleware_runtime.rs) | Per-tool `Chain` (timeout, output cap, audit); `Fan` for read-only parallel tools |
| Sub-agents | [`crates/hq-agent/src/agents/service.rs`](../crates/hq-agent/src/agents/service.rs) (`AgentService`), [`agents/tool.rs`](../crates/hq-agent/src/agents/tool.rs) (`spawn_subagents`) | Single, parallel, race and graph modes in process | Shared cancellation, `Fan` failure modes, dependency `Gate`s |
| Session build | [`crates/hq-agent/src/builder.rs`](../crates/hq-agent/src/builder.rs), `SessionBuilder` | Wires provider, tools, guardian | **Validated pipeline** at build time (ordering, missing tools) |

## hq-web

| Seam | Location | Middleware opportunity |
|------|-----------|-------------------------|
| API-key gate | [`crates/hq-web/src/auth.rs`](../crates/hq-web/src/auth.rs) | Already one gate over `http_auth::resolve_api_key_candidate`, used by [`mcp_http.rs`](../crates/hq-web/src/mcp_http.rs) |
| Router / WS | [`crates/hq-web/src/lib.rs`](../crates/hq-web/src/lib.rs), [`api.rs`](../crates/hq-web/src/api.rs), [`ws/mod.rs`](../crates/hq-web/src/ws/mod.rs) | Request ID, timeout envelopes, vault path validation extractors |

## hq-relay

| Seam | Location | Middleware opportunity |
|------|-----------|-------------------------|
| Session runner | [`crates/hq-relay/src/session_runner.rs`](../crates/hq-relay/src/session_runner.rs) | Per-turn timeouts; align with `hq_core::middleware::default_timeout_for_interval` and shared transient classification |
| Channel bots | [`telegram/`](../crates/hq-relay/src/telegram/), [`discord/`](../crates/hq-relay/src/discord/), [`relay_common.rs`](../crates/hq-relay/src/relay_common.rs) | Command and chunking duplication; `Pipe`: normalize, command, session, deliver |

## hq-cli / daemon scheduling

| Seam | Location | Middleware opportunity |
|------|-----------|-------------------------|
| Chat UX | [`crates/hq-cli/src/commands/chat/mod.rs`](../crates/hq-cli/src/commands/chat/mod.rs) | Shared transient detection for suppressed errors / rate-limit messaging (`middleware::unstructured_llm_error_looks_transient`) |
| Task tick | [`crates/hq-cli/src/commands/start/daemon/mod.rs`](../crates/hq-cli/src/commands/start/daemon/mod.rs), `dispatch_task` | Per-task timeout already comes from `hq-core::middleware::default_timeout_for_interval`; add metrics |

## hq-llm

| Seam | Location | Middleware opportunity |
|------|-----------|-------------------------|
| Provider errors | [`crates/hq-llm/src/provider.rs`](../crates/hq-llm/src/provider.rs), `LlmError` | `LlmError::runtime_error_kind()` maps to `hq-core::middleware::RuntimeErrorKind` |
| Router | [`crates/hq-llm/src/router/`](../crates/hq-llm/src/router/) | Cooldown / telemetry; circuit breaker as middleware ring |

## Adoption order (recommended)

1. **Kernel + LLM mapping**: `hq-core::middleware` and `LlmError::runtime_error_kind`, in place.
2. **Web API-key gate**: done, one gate in `hq-web/src/auth.rs`.
3. **Agent turn loop**: single source of truth for backoff and transient detection.
4. **Relay session runner**: conditional retry and named timeouts.
5. **Daemon**: tier timeouts already come from the kernel; add metrics.
