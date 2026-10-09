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

## Recording and budgets (phase 3)

**Where calls are recorded.** Every concrete provider is wrapped once, at construction, in
`hq_llm::InstrumentedProvider`. It asks the budget gate before each attempt and writes the ledger row
when the attempt ends, so it does not matter whether a call comes from the router, the backend chain
or a session's own backend. Production code shares one process-wide `Instruments` handle;
`hq_agent::install_ledger(db)` points it at the database and is called by `hq start`, `hq web` and the
session builder. Tests build isolated handles.

**Budgets.** Nothing is enforced until `budgets:` is set in `config.yaml`:

```yaml
budgets:
  - name: month
    scope: global            # or provider:<backend>, model:<id>, origin:<name>
    period: month            # day, week or month, in UTC
    limit_usd: 20
    soft_pct: [50, 80]       # alerts, once per period each; 100 always alerts
    action: block            # block (default), downgrade, notify
  - name: memory-daily
    scope: origin:memory
    period: day
    limit_usd: 1
    action: downgrade
    downgrade_model: openrouter/some-cheap-model
background_run_usd: 0.50     # ceiling for one background, watch or sub-agent run
allow_unpriced_models: []    # models allowed under a blocking budget although they have no price
```

How the gate decides, per attempt against one provider:

- A call estimated at zero dollars (local models, subscription backends) is always allowed, so a
  used-up budget lets the backend chain fall through to a local model.
- Otherwise the budget refuses when spend is already at the limit or the call's worst-case cost (the
  prompt plus `max_tokens`, or 4096 when unset) would pass it.
- A refused attempt on one backend moves the chain to the next backend; if every backend is refused
  the caller gets the budget's own message, not "all providers failed". A refusal is not recorded as a
  provider failure and writes no ledger row.
- A model with no known price cannot be counted, so a blocking budget refuses it unless it is listed
  in `allow_unpriced_models`.
- Limits are re-read every 30 s and spend every 5 s, and ledger rows are written asynchronously, so a
  burst of parallel calls can overshoot a limit by the last few seconds of spend plus what is in flight.
- A model-scoped budget matches the model id the provider reports, or a dated snapshot of it
  (`id-2026...`); a provider that answers under a different slug is not counted against it.
- A budget that is malformed (non-positive limit, repeated name, downgrade with no target) is left out
  of enforcement and logged; an unreadable `config.yaml` keeps the last good budgets in force.
- A call that was downgraded is asked about again on its new model.

`GET /api/budgets` shows each budget against the ledger; `PUT /api/budgets` replaces them after
validation. `hq usage budgets` prints the same view.

## Forecasting (phase 4)

`hq_llm::forecast` is pure (events and a clock in, a forecast out), like `copilot_burn` but fed by the
ledger's incremental spend instead of a provider's running counter, so it works for every provider.

- Rate: dollars per hour over the last 24 hours, averaged over the history that exists when there is
  less than a day. Windows of 1, 6, 24 and 168 hours are reported too.
- Projection: spend so far plus the rate times the hours left in the period. With 14 or more days of
  history it uses each weekday's own average instead, so a Monday-heavy pattern is not flattened;
  before that the forecast says why it is flat.
- Exhaustion: when a budget limit is given, the hour it is used up and whether that is before the
  period resets.
- Confidence is low with fewer than 3 spend events or 6 hours of history, medium below 72 hours.

`GET /api/usage/forecast` returns the month forecast (against the tightest global monthly budget),
one forecast per budget, and the biggest drivers by model and origin over the last 7 days.
`hq usage forecast` prints the same.

## Cost-aware routing and guidance (phase 5)

**Routing under budget pressure.** The gate tells the router how close each budget is
(`Instruments::set_pressure`): 0 below half the limit, rising to 1 at the limit, per provider for a
`provider:` budget and for all providers for a `global` one. The router's provider score
(`compute_score_pressured`) multiplies the cost weight by up to 7x at full pressure and shrinks the
health, reliability and speed weights to keep the sum at one, so calls drift to cheaper providers as a
budget drains instead of hitting a wall. With no pressure the score is identical to before. Pressure
only reorders candidates the router already has; a single backend chain is ordered by its
configuration and relies on the gate's failover instead.

**Cheap by default for background work.** There is no separate switch: give `origin:memory`,
`origin:skill_review`, `origin:embeddings` and similar a `downgrade` budget with a cheaper
`downgrade_model` (see the example above), or a `block` budget to cap them.

**Efficiency.** The forecast drivers (`GET /api/usage/forecast`, `hq usage forecast`) now carry average
prompt size and cache hit rate per model and origin. A rising prompt size means context is growing; a
low cache hit rate means the prompt prefix keeps changing.

**Seeing its own budget.** The read-only `budget_status` tool returns the budgets, the burn rate, the
projection and the drivers. `budgets.guidance: true` adds one line to the system prompt at session start
once any budget is past 80%, naming the tightest one. It is off by default because whether it lowers
cost without lowering quality has not been measured.

## Harness spend (phase 6)

Coding agents the host runs (Claude Code, Codex, Pi, Kimi, OpenCode, Copilot CLI) spend tokens HQ never
sees through its own providers, usually the most of any consumer. A daemon task (`harness-usage`, every
5 minutes) reads what each agent already writes about itself and records one ledger row per call with
origin `harness`. Only numbers, model names and ids are read; no prompt or reply text is parsed into a
value or stored.

| Harness | Source | Notes |
|---|---|---|
| Claude Code | `~/.claude*/projects/**/*.jsonl` | one record per content block, so a `message.id` counts once; subagent files included |
| Codex | `~/.codex/sessions/**/rollout-*.jsonl` | per-call `last_token_usage`; the running total is ignored |
| Pi | `~/.pi/agent/sessions`, `~/.pi/hq-sessions` | records its own dollar cost per call, used as given |
| Kimi | `~/.kimi/sessions/*/*/wire.jsonl` | no model name in the record, so rows are unpriced |
| OpenCode | `~/.local/share/opencode/opencode.db` | assistant messages; a cost of 0 means free or unpriced, not a billed zero |
| Copilot CLI | `~/.copilot/session-store.db` | AI units, not dollars: tokens recorded, cost `flat` |
| Cursor, Antigravity, Qwen | none | Cursor keeps usage server side; Antigravity stores opaque protobuf; Qwen is dormant here. They are untracked, not zero |

Input is normalised to the ledger's convention (cache reads included, cache writes excluded). Each call
has a stable `external_id`, so reading a growing file again counts nothing twice; unchanged files are
skipped by size and mtime, and SQLite sources resume from a cursor.

A call is filed under the HQ session that was running that harness in that directory when it happened.
Two candidates, or none, match nothing and the row is filed as `external:<harness>:<id>` rather than
guessing. Cost is the recorded dollars where the agent has them, `flat` for subscriptions, and
otherwise list price, which is what the call would have cost on the API and not necessarily what was
billed. Harness rows never count toward a `global` budget (they would block HQ's own chat for work the
owner did elsewhere); budget them with `origin:harness`.
