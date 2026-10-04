# Structured decisions: the "System 1" gate

Some choices in agent-hq are yes/no or pick-one questions (is this turn worth
remembering, is this email worth the owner's attention). A structured-decision model
such as TypeSafe's Jev answers those with calibrated probabilities in about half
a second for a fraction of a cent, so it can sit in front of a generative call
and decide whether that call is needed. It generates no text and reasons over
at most a 32k-token context, so it gates work and never replaces it.

## Shape

Same Definition / Provider / Consumer split as `capability-seams.md`.

- **Definition**: `hq_llm::decision::DecisionProvider`. `LlmProvider` is chat
  shaped, so decisions get their own trait.
- **Provider**: `HttpDecisionProvider`. OpenRouter and TypeSafe's native API take
  the same `{model, state, questions}` body, so one struct serves every route.
  `DecisionChain` tries routes in order and moves on after any failure.
- **Consumers**, one per site:

  | Site | Where | Default | Acts on |
  |---|---|---|---|
  | `email_fyi` | `hq_daemon::email_gate`, FYI branch of `email_triage::handle` | enforce, hide below 0.30 | keeps promotions and newsletters out of the owner's chat |
  | `memory_turn` | `hq_memory::turn_gate::admit`, native chat turns | enforce, skip below 0.15 | skips extraction for confidently ephemeral turns |
  | `notify_gate` | declared in `hq_core::config::decisions` (`SITE_NOTIFY_GATE`); no consumer calls it yet | shadow | reserved for `hq_daemon::notif_gate::gate_decision`, which today decides drop/digest/urgent for relay messages from learned `_system/USER.md` traits |
  | `task_placement` | `hq_tools::tasks::choose_placement_via_jev`, called from the `task_create_from_note` tool | off | picks the best-fitting existing Space/Folder/List for a note being promoted to a task |

  Consumers hold an `Option<Arc<Decisions>>` and never see a URL, key, or model id.
  `task_placement` fires for the web vault UI's "Convert to Task" button because
  that path runs inside `hq start`'s single process (the tool registry the web
  chat's agent turn calls into is built in the same process that calls
  `decision::init` at daemon startup — see `crates/hq-cli/src/commands/start/mod.rs`).
  It behaves as off, same as every other site, only for a caller running outside
  that process (a bare `hq` CLI invocation, or `hq mcp-serve` launched standalone
  by an external harness). Its incumbent, a tag/folder/title-word match against
  existing Lists, is what every other site here calls "the incumbent path" — the
  same fall-back-on-`None` pattern, just with a heuristic instead of a hardcoded
  default.
- **Config**: the `decisions:` section (`hq_core::config::DecisionsConfig`),
  off by default. Credentials are named by env var, never stored.

```yaml
decisions:
  enabled: true
  timeout_ms: 3000
  routes:
    - endpoint: https://openrouter.ai/api/alpha/decisions
      model: typesafe/jev-1.13
      credential_env: OPENROUTER_API_KEY
  sites:                       # optional: a site not listed uses its default above
    email_fyi: { mode: enforce, threshold: 0.30 }
    memory_turn: { mode: enforce, threshold: 0.15 }
```

`decisions:` is read once at daemon start; changing it needs a restart. The
handle is initialized only in the daemon process, so an `hq` CLI run or any
other separate process has no handle and behaves as if the section were off.
Every gate above runs inside the daemon (email triage, native sessions, chat
ingest), so all of them are covered.

## Site modes

- `off`: the model is never called.
- `shadow`: called in the background beside the incumbent, never blocking. Each
  call appends a line to `_system/decision-shadow/YYYY-MM-DD.jsonl` with the
  incumbent's decision, the model's answers, the model build that answered,
  latency, and cost. An incumbent that could not answer is recorded as
  `unavailable`, not as a decision.
- `enforce`: the answer is acted on. Any error, timeout, or unusable answer
  falls back to the incumbent behavior, so a provider outage never loses data.

## Watching what it does

```bash
hq decisions                 # last 7 days, every site
hq decisions --days 1 --site email_fyi
```

Per site it prints calls, errors, cost, outcomes, latency percentiles, a score
histogram, the model builds seen, and the most recent items it hid (sender and
subject for email, an excerpt for memory text). Enforced sites log every decision,
admits included, so a suppress rate is always computable. Hidden email is still
saved under `_events/`, and a hidden turn or insight can be found by its hash.

The email gate scores only the sender and subject, so a terse subject on an
important email is its failure mode. Two guards limit the damage: mail the model
labels as a real person or an account notice is never hidden, and the 0.30
cutoff sits well below the lowest borderline score seen (0.34 to 0.42), because
scores drift about 0.02 between identical calls. Raise the threshold once the
report shows what it is hiding.

## Swapping something

- **URL moved** (the OpenRouter path is `/api/alpha/`): edit `routes[].endpoint`.
- **New model or version**: edit `model`, set the site to `shadow`, let it run,
  compare against the log (the response's `model` field names the exact build),
  then flip to `enforce`. Thresholds are calibrated per version, which is why
  the default pins `typesafe/jev-1.13` instead of the floating alias.
- **Provider outage or shutdown**: append a second route. TypeSafe's native API
  is `https://api.typesafe.ai/v1/systemone` with model `jev-latest` and its own
  key env var. Model ids differ per route: `~typesafe/jev-1.13` is rejected by
  OpenRouter, and a bare TypeSafe id is rejected by the other route.
- **Incompatible wire format**: implement `DecisionProvider` again and select it
  in `build()`. No consumer changes.

## Adding a consumer

Pick a site name in `hq_core::config::decisions`, ask a `noul` or `choice`
question through `Decisions`, and treat any `Err` as "use the existing path".
Redact anything sent with `hq_core::redact::redact_secrets`. Consumer tests use
`FakeDecisionProvider`, so they need no network.

If a state can plausibly fit none of a `choice` question's options, use
`choice_or_abstain` instead of `choice`: it always offers `UNCLEAR` alongside
your options, the way jev-browser's own action loop lets the model return
`REVIEW` rather than force a guess. Read the answer with
`DecisionResponse::choice_or_abstain`, which returns `Picked` or `Unclear`
instead of a bare label. Control flow after `Unclear` is the same as after an
error (fall back to the incumbent), but call `.record(...)` first: that log
line is the only way to tell "the model was unsure" apart from "the provider
failed" or "the model confidently picked something below threshold" after the
fact. Offering `UNCLEAR` also redistributes probability mass across the whole
option set, so a site that thresholds on a choice probability is calibrated
against that option set, not just against the model version — recalibrate if
you add it to an existing `choice` question rather than a new one.

## Checking it live

```bash
OPENROUTER_API_KEY=... cargo test -p hq-llm --test decision_live -- --ignored
```
