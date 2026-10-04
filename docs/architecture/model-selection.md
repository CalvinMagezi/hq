# Model selection for HQ and delegated agents (FR-065)

## What "supported" means

A model is supported when it is reachable through a declared, enabled entry
under `backends:` in the config. Nothing else is assumed callable. In
particular, a model that appears in GitHub Copilot's live `/models` listing is
not thereby callable: the listing carries per-model token limits only, and a
listed model can still fail with `model_not_available_for_integrator` when the
request does not use the integration the model requires.

Constraints established from the code (no live Copilot calls were made):

| Backend kind | Wire | Constraint |
|---|---|---|
| `github-copilot-api` | Anthropic Messages at `<api base>/v1/messages` | Claude and Gemini family models. The raw GitHub token is exchanged for a session token first (`copilot.rs`), otherwise about 45% of calls fail as `model_not_available_for_integrator`. Business and enterprise seats get a different API base from the exchange. |
| `openai-compatible` pointed at a Copilot host | `chat_completions` or `responses` | GPT-family models such as `gpt-6-luna` are served only over the Responses API, so the entry needs `wire: responses`. |
| `github-copilot-cli` | buffered subprocess | No tool loop, so it cannot serve a delegated child agent. Rejected for overrides. |
| `openrouter`, `kimi-code`, `anthropic-compatible` | their own APIs | Model ids are whatever the entry declares. |

Copilot also enforces its own `max_prompt_tokens` per model, often below the
native window. That limit is read live for context budgeting only
(`copilot::live_context_window`).

## Selecting the model for HQ's own session

Already supported, unchanged: the primary backend's `model` drives turns
(`hq_core::config::resolve_session_model`), and the chat command `model
<backend or model id>` switches the primary (`hq-relay`, `chat_commands.rs`).

## Selecting the model for a delegated agent

`spawn_subagents` accepts, per task:

- `model`: a declared backend name, or that backend's declared model id. The
  child runs in-process, with tools, on that backend's own provider.
- `on_model_unavailable`: `reject` (default) or `inherit`.

Resolution (`crates/hq-agent/src/agents/model_select.rs`) happens before
dispatch:

1. No `model`: the child inherits the parent's model, or the role's router
   alias for coder, planner, explorer and verifier.
2. `model` with a backend chain configured: it must match a declared backend
   or model, that backend must have a tool loop and a pinned model, and, for a
   child on an external backend, it must be that same backend. Otherwise the
   child is `rejected` with the list of available `backend (model)` pairs.
3. `model` with no chain (legacy router): the id's shape is checked (no
   whitespace or control characters, at most 128 bytes, nothing credential
   shaped). The router cannot be enumerated, so an unknown id surfaces as a
   provider failure at run time.

A backend chain pins each backend's configured model onto every request. That
is why an override is resolved to a backend's own provider rather than only
written into the child's session config: the latter would be overwritten and
the override silently ignored. The override provider is a single backend, never
the whole chain, so chain failover cannot switch the model behind the caller's
back.

## Visibility and fallback

Every dispatched child's outcome carries `effective_model`: the model, the
backend (or `null` under the legacy router), `source` (`inherited`,
`role_alias` or `override`) and `fallback_from`. The text summary shows it as
`[model m-b, override]`.

Fallback is explicit. `on_model_unavailable: inherit` runs a child whose
override was unsupported on the parent's model and records the rejected request
in `fallback_from`. It applies only to pre-dispatch validation. A provider
failure during the run is reported as a `failed` outcome naming the override
model; the model is never swapped and the run is never retried on another model.

## Credentials

The override string is never echoed in an error and is refused if it matches a
known secret shape. Outcome `output`, `error` and rejection reasons pass through
`hq_core::redact::redact_secrets`. The model id is not written into the child's
system prompt or task text.

## Known limits

- Legacy router mode cannot pre-validate a model id against a catalog.
- The live Copilot `/models` catalog is deliberately not consulted for
  validation: tests cannot reach it, and a listing does not prove callability.
- Redaction is pattern based (see `hq-core/src/redact.rs`) and best effort.
