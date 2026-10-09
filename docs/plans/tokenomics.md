# Tokenomics: spend tracking, budgets and efficiency

Goal: every LLM call HQ makes is counted once, priced honestly, attributed to the work that caused
it, checked against budgets and forecast forward, and routed with its cost in mind. The phases are
tracked as an epic in HQ tasks (Tokenomics initiative); this page records the design as it lands.

## The spend ledger (phase 1)

One row per LLM call in `task_outcomes`, written when the call **ends**, so it carries the usage the
provider reports in its final chunks. Streamed calls used to be recorded at the first chunk with no
token counts, which made most rows read as zero.

| Column | Meaning |
|---|---|
| `input_tokens`, `output_tokens` | Prompt tokens (cached ones included) and completion tokens. |
| `cache_read_tokens`, `cache_write_tokens`, `reasoning_tokens` | Splits the provider reports; cache reads bill cheaper. |
| `cost_usd`, `cost_source` | `table` (tokens times the price table), `provider` (the provider's billed figure), `free` (local inference), `flat` (subscription quota), `unpriced` (price unknown, cost is **not** really zero), `none` (call failed). |
| `provider` | The backend that answered. With a backend chain this is the configured backend name, not `backends`. |
| `origin` | `chat`, `background`, `subagent`, `memory`, `skill_review`, `embeddings`, `supervisor`, `cli`, or `unknown`. |

Rules for anything that reads the ledger:

- Use `hq_db::usage_ledger`; it counts `unpriced_calls` next to dollars. A total with unpriced calls
  is a lower bound.
- `cost_usd` is never NULL. Check `cost_source` before treating 0.0 as a real zero.
- Raw rows older than `usage_ledger.retain_raw_days` (default 90, never below 35) fold into
  `usage_daily` once a day.

Pricing lives in one function, `hq_llm::cost::price_call`. The router ledger and the session budget
both call it, so a cached call costs the same in both.

Attribution: sessions tag their calls (`chat`, `background`, `subagent`); other call sites use
`hq_llm::with_origin(origin::MEMORY, fut)`. A call nobody scoped is recorded as `unknown` and counted
by `hq_llm::unscoped_calls()`; `hq doctor` warns when more than 5% of recent calls have no origin.

## Commands

- `hq usage` / `hq usage origin` / `hq usage daily`: spend, tokens, cache hit rate, unpriced warning.
- `hq usage reconcile`: compares the ledger with OpenRouter's billed spend for today, this week and
  this month (UTC). The daemon runs the same check daily and logs a warning on drift beyond 10%.
