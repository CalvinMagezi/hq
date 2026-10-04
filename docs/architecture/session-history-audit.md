# Session history audit: agent-hq vs. the event-sourced model

Scope: audit + gap-closing only (per decision on this plan), not a storage
rewrite. dsh's rule under audit: "message history is *derived* from an
append-only log, never stored separately; replay = re-derivation."

## What already matches the model

**Cross-interface conversation continuity — genuinely event-sourced.**
`crates/hq-agent/src/threads.rs` is a true append-only log: every interface
(Telegram, Discord, web, CLI) calls `append_thread_entry()` to append one
JSON line per turn to `.vault/_threads/{interface}.jsonl`. Conversation
continuity for a fresh `AgentSession` is re-derived by
`load_merged_thread()`, which reads every interface's file tail, merges by
timestamp, and reconstructs the view — no separate cached copy of "the
conversation" is trusted as ground truth. `rotate_thread_file()` trims the
log itself (returning the trimmed lines for memory consolidation) rather
than replacing it with a derived snapshot. This is the reference example
for the rest of the codebase, alongside `hq-llm::LlmProvider`.

**Background-turn interruption — also event-sourced, and self-aware about
it.** `background_turns` (`hq-db/src/background_turns.rs`) is one row per
detached relay turn (prompt, status, harness child session ids). The startup
+ 6h `turn_reconciler` (`hq-daemon/src/turn_reconciler.rs`) sweeps stranded
`running` rows on restart, marks them `interrupted`, and — critically —
calls `record_interrupt_thread_entry()` to append the interruption FYI into
the *same* `_threads/*.jsonl` file a normal completion would have landed in.
The doc comment on that function states the principle explicitly: the
mailbox delivery reaches the chat surface but bypasses thread history
entirely, so without this call the derived log would silently omit that a
turn died. This is the exact "derived history must honestly reflect what
happened, including failures" property dsh's model requires — already
implemented, not just accidentally compliant.

**External harness sessions — delegated, not duplicated.** `ChannelState`
(`hq-relay/src/relay_common.rs`) stores `session_ids: HashMap<String,
String>` mapping a chat thread to a harness's own session id (used for
`--resume <id>`-style CLI flags). The actual transcript for those sessions
lives in the external harness's own store (cursor, claude-code, etc.) —
agent-hq holds a pointer, not a redundant copy. No drift risk: there's only
one owner of that transcript.

## The gap found and closed

**`ChannelState.messages` (per-channel OpenRouter-resubmission history) had
no failure marker on a harness dispatch error.** This field is a different,
legitimate concept from `_threads/*.jsonl` — not a duplicate of it. It's the
literal message list resubmitted to the LLM for the native `hq` harness's
context (`_threads/*.jsonl` is a lightweight, 2000-char-capped, cross-surface
awareness signal for the context engine's Injection layer; `ChannelState.
messages` is the exact per-channel API history). Both `discord/dispatch.rs`'s
`dispatch_hq` and `telegram/dispatch.rs`'s `dispatch_hq` pushed
the user's message onto `state.messages`, then dispatched to a harness with
the `?` operator (or a `match` that just propagated `Err`) — on failure, the
function returned before ever reaching the "push the assistant reply" step.
The persisted state therefore kept a dangling, unanswered user turn with **no
record that a turn had failed** — the exact case
`record_interrupt_thread_entry` exists to prevent for `_threads/*.jsonl`, but
this second history store had no equivalent.

Concretely, without the fix: turn N fails → `messages` ends
`[..., user: "<message N>"]`. Turn N+1 succeeds → `messages` becomes
`[..., user: "<message N>", user: "<message N+1>", assistant: "<reply>"]` —
two consecutive user turns resubmitted to the LLM with no signal a failure
ever happened in between.

**Fix**: added `ChannelState::record_turn_outcome(&Result<String>)`
(`relay_common.rs`) — on success, appends the assistant reply (unchanged
behavior); on failure, appends a `[turn failed: <error>]` assistant-role
marker instead of nothing. Both `discord/dispatch.rs`'s `dispatch_hq` and
`telegram/dispatch.rs`'s `dispatch_hq` now call this one shared method
instead of duplicating the push logic inline, closing the gap identically on
both surfaces. `dispatch_hq` was additionally restructured so the harness
selection `match` (previously using `?` to return straight out of the
function) now resolves to a `Result<String>` value first, so both outcomes
flow through the same recording step before the function returns — behavior
for the success path is unchanged.

**Tests**: `relay_common::tests::record_turn_outcome_appends_assistant_reply_on_success`
and `record_turn_outcome_appends_failure_marker_instead_of_leaving_a_dangling_user_turn`
— the second asserts the marker's content, and that a subsequent turn's
messages alternate `user, assistant, user, assistant` again instead of ever
landing two `user` entries back to back.

## What was deliberately not touched

- `AgentSession.messages` (in-process, per-turn `Vec<ChatMessage>`,
  `hq-agent/src/session/mod.rs`) has no durable persistence of its own —
  by design. A crash mid-turn loses the detailed tool-call-level transcript
  for that turn, but not the coarse conversation record (`_threads/*.jsonl`
  already has the prior completed turns; `background_turns` already records
  that this specific turn failed to complete, per the section above). Adding
  a full mid-turn event log for `AgentSession` itself would be a genuine
  storage rewrite, out of scope per this plan's own scoping decision, and
  there is no evidence of it causing an actual reported problem — `resume
  <id>` already handles the practical case (rerun the original prompt) that
  this would otherwise exist to serve.
- `chat_messages` (`hq-db/sql/013_chat_threads.sql`) is a separate table used
  by `hq-web`'s thread API / mobile chat UI, not the agent session loop —
  out of scope; it is its own already-append-only table (one row per
  message, ordered by `created_at`), not a derived cache of anything else.
