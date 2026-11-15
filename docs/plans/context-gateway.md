# Context gateway: shared contract (phase 1)

Coding sessions that HQ launches need to read the owner's notes without being handed the
whole vault. The gateway is one place that decides who is calling and what they may read.
Phase 1 is only that decision: identity and scope. Retrieval, MCP tools and adapters come later.

Code: `crates/hq-tools/src/context_gateway/`. Grant lookups: `crates/hq-db/src/context_grants.rs`.

## Identity

The caller is proven by the session secret (`hqs_...`, `hq_db::session_tokens`). From the proven
session id the resolver reads, and nothing else:

1. the secret's row: gone means `Revoked`, past `expires_at` means `Expired`;
2. the session row in `harness_sessions`: not running means `NotRunning`; its `mission_id` is
   copied into the grant for the audit trail;
3. the open `task_work_sessions` row of that session: none means `NoTask`;
4. the task's `task_context_scope` row: none means `Unmarked`.

No argument can name a task or a session. A failure is a typed `Unavailable`; there is no
fallback to a wider tool, and a database error is also a refusal (`Backend`).

## Scope

`task_context_scope.allow_prefixes` is a JSON list of vault folders per task. A task without a
row is unmarked and owner-only. Every folder and every candidate is parsed as a `SourceId`,
which uses `hq_vault::rel_path::RelPath` and additionally refuses the vault root, `_system`,
`_threads`, `_data` and the instruction folders (`skills`, `_skills`, `_commands`, `agents`,
`_mailboxes`). A folder that does not parse is dropped; if none remain the task is unmarked.
Matching is by whole path components. Chat evidence refs are not sources yet, so they are denied.

Denied sources appear only as `GapKind::Denied { count }`. The gap names nothing.

## Types

`SessionGrant`, `SourceScope`, `Caps` (read only for now), `SourceId`, `ContextOp`,
`ContextEntry`, `ContextResult`, `ScopeCheck` (`PrefixScope`), `GrantResolver`
(`DbGrantResolver`). `Freshness`, `EntryProvenance` and `PacketGap` are reused from
`hq_memory::context_packet`, which gained `GapKind::Denied { count }` and `GapKind::RateLimited`.

## Phase 2: retrieval

`context_gateway::retrieval` is the one front door: `Retriever::run(grant, op, now)`. A missing or
expired grant returns `Unavailable` and reads nothing. Every candidate is parsed as a `SourceId`
and checked against the grant before it is read, including each edge of a graph chain; a chain is
cut at its first out-of-scope edge and its text names only the kept nodes. The excerpt budget is
split per source and what does not fit is an `OverBudget` gap. Calls are rate limited per grant
(`RATE_MAX_CALLS`, `RATE_MAX_CHARS` per `RATE_WINDOW_SECS`, in memory) and every call writes one
`context_gateway_audit` row with counts only. `Read.section` is not honoured yet.

## Not yet built

Adapters for other harnesses, `ask_hq`, `include_task_links`, and evaluation.

## Phase 3: the tools

`context_orient`, `context_search`, `context_read`, `context_expand` and `context_related`
(`context_gateway::tools`) are read-only wrappers over `Retriever`, in `SESSION_ALLOWLIST` so a
session token can call them. The session comes only from the id the gateway attested from the
`hqs_` token; the schemas carry no task, session or scope field. A call with no proven session, or
one whose grant cannot be resolved, returns a typed `unavailable` result and reads nothing. One
rate limiter is shared across the five tools.
