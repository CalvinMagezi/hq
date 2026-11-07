# Context fold: long conversations that remember

Status: proposal. Nothing here is built yet.

## The problem

A conversation with HQ forgets everything older than about 20 messages, and it does so silently.

| Surface | What the model sees each turn | What happens to older messages |
|---|---|---|
| Web and phone chat | The newest 20 rows of `chat_messages`, text only (`thread_history`, `crates/hq-web/src/ws/mod.rs:531`) | Never sent again. Still in the table, but only a `LIKE` scan (`session_search`) can find them |
| Telegram, Discord | `ChannelState.messages`, cut from 30 to 20 with no summary (`crates/hq-relay/src/relay_common.rs:126`, `:240`) | Dropped from the API history. Mirrored in `chat_messages` by `thread_sync` |
| One long turn (tool loops, sub-agents) | Everything, until 75% of the window, then the newest 10 messages plus one summary of about 800 tokens (`crates/hq-agent/src/session/context.rs:71`) | Gone. The summary is re-summarized the next time, so detail decays every round. If the summary call fails, the messages are just dropped |

So "what did we decide about the school fees plan last week" fails unless the per-turn memory
ingester happened to pick it up. An owner who uses one Telegram thread as their HQ loses
continuity every 10 to 15 exchanges. The in-session compactor is not the main problem: chat
sessions are rebuilt from stored history every turn, so it rarely fires there at all.

Three other costs sit next to this:

- The system prompt is rebuilt every turn with content that changes every turn (recent memories,
  merged thread tails, concept-page matches, budget note; `crates/hq-agent/src/builder/prompt.rs`).
  Providers that cache by prefix (DeepSeek, OpenAI, Gemini, Anthropic) can then reuse almost
  nothing past the first few hundred bytes. OpenRouter calls send no cache hints at all.
- Memory ingestion runs an extraction call (`bulk` alias) after every turn with 100 or more reply
  characters (`crates/hq-agent/src/native_hq/ingest.rs:77`). That is one extra model call per turn,
  extracting from raw single turns rather than from distilled history.
- Tool output over about 4,000 tokens is cut to its head and tail (`hq-core/src/microcompact.rs`)
  and the agent cannot read the rest.

## What we take from billion-context, and what we leave

Validated in the review of `ranxianglei/billion-context` (its paper's own controlled pilot and
kernel code, not its README headline numbers):

Take:

1. **Fold only the increment.** Each fold summarizes the messages added since the last fold and
   leaves earlier summary blocks untouched. Early content is summarized once in its life instead
   of once per round, so it stops decaying.
2. **Keep the originals and make every block restorable.** A summary carries the ids of the
   messages it replaced. The agent can search and restore them.
3. **The summary rules** (MIT, `kernel/src/compression-rules.ts`): keep paths, exact errors,
   exact values and decisions with their reasons verbatim; carry `Open objectives:` forward with
   message refs; quote the user only with a ref; label recorded task state as history
   ("TASK AS OF THIS BLOCK"), never as a live instruction.
4. **Append-only layout with volatile text at the tail.** A fold only adds a block, so the bytes
   before it never change. Anything that changes per turn goes after the history, not in the
   system prompt.
5. **Stable message refs in context.** Their only control run (n=1) found that ref tags plus the
   rules cut re-reads by 62% with zero compressions. Cheap to try, must be measured here.
6. **A byte-prefix test.** Assert that turn N+1's request shares its prefix with turn N except
   at a fold point.

Leave:

- The model deciding when to compress, through a `compress` tool. Their own pilot found a fixed
  schedule as good or cheaper, models never used restore without being told to, and the roughly
  2,000-token rule text on every call costs about 25% at small windows. HQ decides when; the
  model only writes the summary.
- Tier 3 and the "pagoda" capacity claims. In their production data tier 2 touched 60 of 1,711
  blocks. One distillation tier is enough.
- The proxy and certificate interception. HQ owns its own request assembly.

## Design

### The view the model sees

```
[system prompt: soul, tools, stable instructions]        stable for days
[fold block b1  m0001..m0048]                             append-only
[fold block b2  m0049..m0097]                             append-only
[raw tail: messages after the last fold]                  grows, then folds
[turn context: date, recent memories, thread tails, ...]  volatile, at the end
[current user message]
```

Each fold block is one message, wrapped as derived history:

```
[Earlier conversation, m0049 to m0097, 2026-10-02 to 2026-10-05. Summary written by HQ.]
- Goal as of this block: ...
- Decided: pay the term fees in two parts, because the second salary lands on the 25th (m0071)
- Open objectives: send the landlord the revised budget (m0090)
- Values: deposit UGX 450,000; account ending 2231 (no full numbers)
```

### Where the state lives

The originals already exist. `chat_messages` is append-only and shared by web, phone and both
relays. The fold adds one table (migration 115):

```sql
CREATE TABLE thread_folds (
  id            TEXT PRIMARY KEY,
  thread_id     TEXT NOT NULL REFERENCES chat_threads(thread_id),
  tier          INTEGER NOT NULL DEFAULT 1,     -- 1 = from messages, 2 = from blocks
  first_msg_id  TEXT NOT NULL,
  last_msg_id   TEXT NOT NULL,
  summary       TEXT NOT NULL,
  source_tokens INTEGER NOT NULL,
  summary_tokens INTEGER NOT NULL,
  tainted       INTEGER NOT NULL DEFAULT 0,     -- range held untrusted content
  model         TEXT NOT NULL,
  superseded_by TEXT,                           -- set when a tier-2 block absorbs it
  created_at    TEXT NOT NULL
);
CREATE INDEX thread_folds_thread ON thread_folds(thread_id, created_at);
```

Ranges are keyed by message id, never by index. That avoids the stale-index bug in today's
background summarizer (`covers_up_to` is an absolute index that goes wrong when another compaction
runs in between).

### When a fold runs

- **Who decides:** the runtime, by budget. The raw tail has a target (default about 12,000
  estimated tokens or 30 messages, whichever comes first). When the tail passes twice the target,
  the oldest part folds down to the target.
- **Where:** after the reply has been delivered, as a background job keyed by thread. It is never
  on the critical path of a reply.
- **Cut points:** only at turn boundaries (after an assistant reply), so a user message and its
  answer, or a tool call and its result, are never split.
- **Tier 2:** when the fold blocks of one thread pass a budget (default about 6,000 tokens), the
  oldest blocks are distilled into one tier-2 block with the tier-2 rules. Rare by design.
- **Model:** the `bulk` alias by default (`context.fold.model`). One cheap call per roughly 15
  exchanges.
- **Failure:** if the summary call fails, nothing is lost. The thread falls back to today's
  newest-N window and the fold is retried later. The originals stay in `chat_messages` either way.
- **Offline or data saver on a phone:** folds wait until a model is reachable. Search over the
  originals keeps working offline (below).

### The summary rules

Port `HOW_TO_COMPRESS_RULES` and `TIER2_DISTILL_RULES`, adapted for HQ's conversations, which are
mostly not coding:

- Keep verbatim: amounts and currencies, dates and deadlines, names of people and places, file
  paths, commands, errors, decisions with their reason.
- Carry forward: open objectives, and **commitments HQ made** ("I'll remind you on Friday").
- Quotes only with a message ref, otherwise mark them as paraphrased.
- History, not instructions: "Goal as of this block", never "Current task".
- Never write secrets. Run the summary through the same redaction the ingester uses before it is
  stored.
- Write in the conversation's language (their opt-in language rule). Many owners mix languages,
  sometimes in one message.

The rules are versioned text in the binary like the shipped skills, so a change ships and can be
reverted like code.

### Finding and restoring

Replace `session_search`'s `LIKE` scan with one tool pair:

- `history_search(query, thread?)`: FTS5 over `chat_messages` (a new `chat_fts` content table kept
  in sync by triggers) plus semantic search over **fold summaries only**, embedded with the local
  Model2Vec model (`potion-base-8M`, already on the phone, about 36 MB resident, works offline).
  Fold summaries are few and dense, so embedding them costs almost nothing. Embedding every chat
  message on a phone is not worth it. A semantic hit on a block narrows the FTS search to that
  block's range.
- `history_restore(block_or_msg_ref, max_tokens?)`: returns the original messages of a block,
  bounded, as a tool result at the tail. Nothing earlier in the request changes, so the cache
  survives.

Because models do not restore on their own, every block carries its refs, and the tool
description says when to use it: "the summary lacks an exact value, quote or step you need".

Embeddings stay keyed by note path today (`embeddings.note_path`, and search requires a live
`notes_fts` row). Fold vectors go in their own small table (`thread_fold_embeddings`) with their
own scan, rather than bending the note index.

### Memory and the graph

The fold is a better ingestion point than the single turn:

- **Ingest at fold time, not every turn.** The fold summary is already distilled, deduplicated
  and tied to message refs. Feed it to `MemoryIngester` with `source_ref = chat:<thread>/<first>..<last>`.
  If the numbers hold (fewer calls, same or better recall), retire per-turn ingestion for threads
  that fold, which removes one model call per turn.
- **Typed relations with evidence.** The graph audit notes nothing writes typed relations yet. The
  fold prompt can end with a short structured section that becomes concept-page lines:
  `- decided_for: [[term-fees-plan]] (src: chat:<thread>/m0071)`. That needs one change in
  `graph_paths` and in `context_packet::verify_citations`: an evidence kind `chat:` that re-reads
  the message from `chat_messages` and checks it still exists and still mentions both endpoints,
  the same way a note is re-read today.
- **Context packets cite chat.** A task's context packet can then include what the owner said about
  that task, which helps task verification and briefs for coding agents.
- **Deletion stays honest.** `delete_messages_from` (edit and regenerate) must drop the folds whose
  range reaches the deleted messages, and their embeddings and graph evidence with them. The
  `chat:` evidence check already drops an edge whose message is gone.

### Cache-friendly assembly

Separate from the fold, and possibly worth more money:

- Move per-turn content (recent memories, merged thread tails, concept matches, budget note) out
  of the system prompt into the turn-context block at the tail. The system prompt then changes only
  when the soul, the tools or the date change.
- Send `cache_control` on OpenRouter when the model is an Anthropic one (today only the direct
  Anthropic base gets it). Others cache by prefix automatically once the prefix is stable.
- Read the effect from the ledger: `task_outcomes.cache_read_tokens` is already recorded per call.

### Safety

- **Taint carries over.** A summary written from untrusted content (web pages, email, MCP callers)
  can carry an injection forward forever, now re-sent every turn. A block whose range held tainted
  content is stored with `tainted = 1`, and loading it marks the session tainted
  (`governance/taint.rs`), exactly as reading the original would have.
- **Blocks are history.** They are wrapped as derived history, and the rules forbid writing
  instructions as live directives.
- **Adoption data stays out.** Folds contain conversation text only. The "Things you can offer"
  block and anything from `hq_db::adoption` never enter a fold.

### The single long turn

The in-session compactor gets the same treatment, smaller:

- Incremental: keep earlier `[CONTEXT SUMMARY]` blocks as they are and summarize only the new
  increment.
- Turn-safe cut: move the cut point so a tool batch is never split (today `sanitize_tool_messages`
  drops the orphans afterwards).
- Include tool calls and their arguments in what is summarized (today only `content` is rendered).
- Never drop on failure: fall back to cutting tool results, and keep the messages.
- Spill large tool outputs to `_data/spill/<session>/<call>.txt` with a `tool_output_read(ref)`
  tool, which is what the doc comment at `context.rs:301` already promises.

## Measuring it

Nothing above should be tuned by feel.

- **Recall test** (`crates/hq-agent/tests/fold_recall.rs`): a scripted 200-turn conversation with
  planted facts (an amount, a decision and its reason, an open ask, a preference that is later
  changed). Run the fold N times, then ask for each. Mechanics tests use a deterministic fake
  summarizer and are regression gates. A real-model recall run is `#[ignore]` with an opt-in
  command, like the other model-backed diagnostics.
- **Prefix test:** assemble turns N and N+1 and assert the shared byte prefix covers everything up
  to the turn-context block, except at a fold point.
- **Ledger numbers before and after:** cache read share per origin, input tokens per chat turn,
  and calls per turn (including ingestion).
- **Baseline first:** how many threads pass 20 messages, and how often the owner asks about
  something older than the window.

## Phases

| Phase | What | Gate |
|---|---|---|
| 0 | Measure: cache read share by origin, thread lengths, the volatile parts of the system prompt | Numbers written down here |
| 1 | `thread_folds`, fold job, ported rules, assembly for web and phone chat, deletion handling, prefix test, recall test with the fake summarizer | Recall test green; no change for threads under the budget |
| 2 | `chat_fts`, `history_search` and `history_restore` replacing `session_search`, fold embeddings on the local model | Search works offline on the Mobile profile |
| 3 | Relays read history from `chat_messages` plus folds instead of `ChannelState.messages` | Telegram thread keeps continuity past 30 messages |
| 4 | Turn context moved to the tail, `cache_control` on OpenRouter for Anthropic models | Cache read share up in the ledger |
| 5 | Fold-time ingestion, `chat:` evidence in the graph and context packets, typed relations | Fewer calls per turn at equal or better recall |
| 6 | The single long turn: incremental, turn-safe, tool spill | Existing compaction tests plus a spill round trip |

Phases 1 and 4 can ship independently. Phase 5 is the most valuable and the riskiest, so it waits
for the recall test to exist.

## Config

```yaml
context:
  fold:
    enabled: true          # off: today's newest-N window
    tail_tokens: 12000
    tail_messages: 30
    blocks_budget_tokens: 6000
    model: bulk
```

## Open questions

- Should a fold of a family member's Discord request ever reach the owner's memory? Probably not:
  folds inherit the thread's caller and the memory ingester's existing caller rules.
- Is 12,000 tokens right for cheap models with 32K or 64K windows? Default it from the resolved
  model's window, and measure.
- Relay `ChannelState.messages` holds exact API history including harness session ids. Phase 3
  needs to check nothing else reads it before it stops being the source.
