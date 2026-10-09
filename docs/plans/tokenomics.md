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

The provider's own billed figure wins when it reports one. OpenRouter returns `usage.cost` on
every response; streams read it from the raw SSE (`StreamChunk::Billing`) and the row is stored with
`cost_source='provider'` and `provider_cost_usd`. Both the ledger and the session budget then use that
figure, so `hq usage reconcile` compares OpenRouter's number with itself plus any calls HQ never saw.
Reasoning tokens are recorded when the provider reports them.

Pricing lives in one function, `hq_llm::cost::price_call`. The router ledger and the session budget
both call it, so a cached call costs the same in both.

Attribution: sessions tag their calls (`chat`, `background`, `subagent`); other call sites use
`hq_llm::with_origin(origin::MEMORY, fut)`. A call nobody scoped is recorded as `unknown` and counted
by `hq_llm::unscoped_calls()`; `hq doctor` warns when more than 5% of recent calls have no origin.

## Commands

- `hq usage` / `hq usage origin` / `hq usage daily`: spend, tokens, cache hit rate, unpriced warning.
- `hq usage reconcile`: compares the ledger with OpenRouter's billed spend for today, this week and
  this month (UTC). The daemon runs the same check daily and logs a warning on drift beyond 10%.

## Provider usage adapters (phase 2)

`GET /api/usage/providers` returns one row per enabled backend: what the provider says (balance,
spend over the UTC day, week and month) and what HQ's own ledger recorded, each labelled with its
`source` (`provider` or `ledger`) so an estimate is never shown as a provider fact. `status` is
`ok`, `refused`, `error`, `no_key`, `local`, `subscription` or `ledger_only`.

Capability matrix, from each provider's official documentation as of 2026-10-09. "Not documented"
means the official page did not state it; nothing here is built on a third-party claim.

| Provider | Balance | Spend | Key needed | Billed cost in responses | Adapter |
|---|---|---|---|---|---|
| OpenRouter | `GET /api/v1/key` (limit, `limit_remaining`); `/credits` | `usage_daily`, `usage_weekly`, `usage_monthly` (UTC) on `/key`; `/activity` (management key) | `/key`: the inference key. `/credits` and `/activity`: documented as management key only | Yes, `usage.cost` | `openrouter` |
| DeepSeek | `GET /user/balance` (`balance_infos[].total_balance`) | none | inference key (Bearer) | no | `deepseek` |
| Moonshot (Open Platform) | `GET /v1/users/me/balance` (`data.available_balance`) | none | inference key | not documented | `moonshot` |
| Anthropic | none | `GET /v1/organizations/cost_report` (UTC days, cents) | admin key; unavailable to individual accounts | no, tokens only | `anthropic_admin`, opt in with `usage_ledger.anthropic_admin_key_env` |
| OpenAI | none | `GET /v1/organization/costs` | admin key | no, tokens only | ledger only (endpoint not verified against OpenAI's own reference) |
| Google Gemini, Groq, Cerebras | none | none (console only) | n/a | no | ledger only |
| Kimi Code, GitHub Copilot | subscription quota | console or `/api/copilot-usage` | n/a | no | `subscription` |
| Ollama and other loopback endpoints | n/a | n/a | n/a | n/a | `local`, tokens tracked, nothing billed |

Open questions, so nobody builds on them unverified: OpenRouter's OpenAPI marks `/credits` as
requiring a management key, while the existing settings card reads it with the inference key and
falls back to `/key` figures when refused; check against a real key.

Not built here: rate-limit response headers (`x-ratelimit-*` for OpenAI and Groq,
`anthropic-ratelimit-*` for Anthropic) as a free quota signal. They need a hook in each provider's
HTTP layer and are tracked as a follow-up.
