use serde::{Deserialize, Serialize};

/// Known model information for context window sizes and cost estimation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelInfo {
    pub id: &'static str,
    pub context_window: u32,
    pub input_cost_per_million: f64,
    pub output_cost_per_million: f64,
    /// Cost per million tokens for prompt cache reads (typically 10% of input cost).
    /// Zero for providers that don't support prompt caching.
    pub cache_read_cost_per_million: f64,
    /// Cost per million tokens for prompt cache writes (typically 125% of input cost).
    /// Zero for providers that don't support prompt caching.
    pub cache_write_cost_per_million: f64,
}

/// Get model info by ID. Returns None for unknown models.
///
/// Resolves through an ordered list of [`pricing_candidates`]: the raw id, a
/// date-stripped snapshot, the historical bare-`claude-*` canonicalization, and
/// — for first-party vendors (`openai/`, `anthropic/`, `google/`) — the id with
/// its provider prefix added or removed. This lets a provider-qualified id such
/// as `openai/gpt-5.4` price identically to its bare registry key (`gpt-5.4`)
/// and vice versa, while OpenRouter community ids (e.g. `qwen/…`, `moonshotai/…`)
/// keep their prefix as the canonical key. Genuinely unknown ids still return
/// `None` (priced at zero).
pub fn get_model_info(model_id: &str) -> Option<ModelInfo> {
    pricing_candidates(model_id)
        .iter()
        .find_map(|candidate| KNOWN_MODELS.iter().find(|m| m.id == candidate).cloned())
}

/// First-party model vendors whose `vendor/model` id and bare `model` id denote
/// the *same* underlying model, so pricing lookups may fall back between the two
/// forms (add or strip the prefix) for them.
///
/// OpenRouter community vendors (e.g. `qwen/`, `moonshotai/`, `minimax/`,
/// `novita/`, `siliconflow/`) are deliberately excluded: their prefixed id IS
/// the canonical registry key and must never be deprefixed, or a community model
/// could collide with an unrelated first-party entry. Extending this list is the
/// intended way to onboard another first-party namespace.
const CANONICAL_VENDORS: &[&str] = &["openai", "anthropic", "google"];

/// Ordered, de-duplicated registry keys to try when pricing `model_id`, from
/// most to least specific. The first candidate that matches [`KNOWN_MODELS`]
/// wins (see [`get_model_info`]).
///
/// Candidates, in order: the raw id; its date-stripped form; the historical
/// bare-`claude-*` → `anthropic/claude-*` canonicalization ([`normalize_model_id`]);
/// then, for [`CANONICAL_VENDORS`] only, the prefix-removed form (when the id is
/// `vendor/model`) or each prefix-added form (when the id is bare). Community
/// `vendor/model` ids fall through with only their raw form, so they never
/// deprefix.
fn pricing_candidates(model_id: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let push = |out: &mut Vec<String>, candidate: String| {
        if !candidate.is_empty() && !out.iter().any(|existing| existing == &candidate) {
            out.push(candidate);
        }
    };

    push(&mut out, model_id.to_string());
    push(&mut out, strip_date_snapshot_suffix(model_id).to_string());
    // Preserve the historical bare-`claude-*` canonicalization exactly.
    push(&mut out, normalize_model_id(model_id));

    match model_id.split_once('/') {
        Some((vendor, rest)) if CANONICAL_VENDORS.contains(&vendor) => {
            // Provider-qualified first-party id: also try the bare form so
            // `openai/gpt-5.4` resolves to the bare `gpt-5.4` registry entry.
            push(&mut out, rest.to_string());
            push(&mut out, strip_date_snapshot_suffix(rest).to_string());
        }
        Some(_) => {
            // Community `vendor/model` id (OpenRouter et al.): the prefix is
            // canonical — never strip it. Only the raw form above is tried.
        }
        None => {
            // Bare id: also try each first-party prefix so a bare `o3` /
            // `gemini-2.5-pro` still resolves to its `openai/`/`google/` entry.
            let dated = strip_date_snapshot_suffix(model_id);
            for vendor in CANONICAL_VENDORS {
                push(&mut out, format!("{vendor}/{model_id}"));
                push(&mut out, format!("{vendor}/{dated}"));
            }
        }
    }
    out
}

/// Normalize a provider-reported model id into the registry's pricing-canonical
/// form so cost lookups succeed for snapshotted or unprefixed ids.
///
/// The Anthropic Messages API reports the concrete model it served — often a
/// dated snapshot like `claude-sonnet-4-6-20260514` and without the
/// `anthropic/` routing prefix the registry keys on. This maps those bare
/// `claude-*` ids to `anthropic/claude-*` and strips a trailing `-YYYYMMDD`
/// date snapshot so they line up with [`KNOWN_MODELS`] and, in turn, with
/// [`calculate_cost_with_cache`]'s usage assumptions.
///
/// Ids that already carry a `provider/` prefix, or that aren't Anthropic
/// `claude-*` ids, are returned unchanged.
pub fn normalize_model_id(model_id: &str) -> String {
    // Already provider-qualified (e.g. `anthropic/…`, `openai/…`): leave as-is.
    if model_id.contains('/') {
        return model_id.to_string();
    }
    // Only Anthropic bare ids need canonicalization here.
    let Some(rest) = model_id.strip_prefix("claude-") else {
        return model_id.to_string();
    };
    let base = strip_date_snapshot_suffix(rest);
    format!("anthropic/claude-{base}")
}

/// Strip a trailing `-YYYYMMDD` (8 ASCII digit) date snapshot segment, if present.
/// `sonnet-4-6-20260514` -> `sonnet-4-6`; `sonnet-4-6` -> `sonnet-4-6`.
fn strip_date_snapshot_suffix(name: &str) -> &str {
    match name.rsplit_once('-') {
        Some((head, tail)) if tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => name,
    }
}

/// Vision-capable model ids (FR-017). A standalone allowlist rather than a
/// `ModelInfo` field: `KNOWN_MODELS` has dozens of literal entries with no
/// capability metadata today, and adding a required field to all of them
/// would be far more invasive than this list. Extend as providers add
/// vision support and it's verified against their docs.
pub fn model_supports_vision(model_id: &str) -> bool {
    const VISION_MODELS: &[&str] = &[
        // Per DeepSeek's API docs: deepseek-flash accepts images alongside
        // text (screenshots, charts, photos). This is the default model.
        "deepseek-flash",
    ];
    VISION_MODELS.contains(&model_id)
}

/// Get the context window size for a model, defaulting to 128K if unknown.
pub fn context_window(model_id: &str) -> u32 {
    get_model_info(model_id)
        .map(|m| m.context_window)
        .unwrap_or(128_000)
}

/// Calculate the cost of an LLM call based on the model ID and token counts.
pub fn calculate_cost(model_id: &str, input_tokens: u32, output_tokens: u32) -> f64 {
    calculate_cost_with_cache(model_id, input_tokens, output_tokens, 0, 0)
}

/// Calculate cost with cache-aware pricing.
///
/// `cache_read_tokens` are subtracted from `input_tokens` and billed at the
/// cheaper cache-read rate. `cache_write_tokens` are billed at the write rate.
pub fn calculate_cost_with_cache(
    model_id: &str,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
) -> f64 {
    match get_model_info(model_id) {
        Some(info) => {
            let fresh_input = input_tokens.saturating_sub(cache_read_tokens);
            (fresh_input as f64 * info.input_cost_per_million / 1_000_000.0)
                + (cache_read_tokens as f64 * info.cache_read_cost_per_million / 1_000_000.0)
                + (cache_write_tokens as f64 * info.cache_write_cost_per_million / 1_000_000.0)
                + (output_tokens as f64 * info.output_cost_per_million / 1_000_000.0)
        }
        None => 0.0,
    }
}

static KNOWN_MODELS: &[ModelInfo] = &[
    // GitHub Copilot subscription models (routed via `gh copilot` CLI).
    // Sonnet 5 exposes a 1M-token context window to Copilot clients.
    ModelInfo {
        id: "claude-sonnet-5",
        context_window: 1_000_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    // Anthropic: cache_read = 10% of input, cache_write = 125% of input
    ModelInfo {
        id: "anthropic/claude-sonnet-4-6",
        context_window: 200_000,
        input_cost_per_million: 3.0,
        output_cost_per_million: 15.0,
        cache_read_cost_per_million: 0.30,
        cache_write_cost_per_million: 3.75,
    },
    ModelInfo {
        id: "anthropic/claude-sonnet-4",
        context_window: 200_000,
        input_cost_per_million: 3.0,
        output_cost_per_million: 15.0,
        cache_read_cost_per_million: 0.30,
        cache_write_cost_per_million: 3.75,
    },
    ModelInfo {
        id: "anthropic/claude-opus-4",
        context_window: 200_000,
        input_cost_per_million: 15.0,
        output_cost_per_million: 75.0,
        cache_read_cost_per_million: 1.50,
        cache_write_cost_per_million: 18.75,
    },
    // Fresh-install defaults. Prices and context from OpenRouter's catalog, 2026-10-09.
    ModelInfo {
        id: "anthropic/claude-haiku-5.5",
        context_window: 1_000_000,
        input_cost_per_million: 0.10,
        output_cost_per_million: 0.50,
        cache_read_cost_per_million: 0.01,
        cache_write_cost_per_million: 0.125,
    },
    ModelInfo {
        id: "openai/gpt-6-luna",
        context_window: 1_050_000,
        input_cost_per_million: 0.10,
        output_cost_per_million: 0.50,
        cache_read_cost_per_million: 0.01,
        cache_write_cost_per_million: 0.125,
    },
    ModelInfo {
        id: "anthropic/claude-haiku-4",
        context_window: 200_000,
        input_cost_per_million: 0.80,
        output_cost_per_million: 4.0,
        cache_read_cost_per_million: 0.08,
        cache_write_cost_per_million: 1.0,
    },
    ModelInfo {
        id: "google/gemini-2.5-pro",
        context_window: 1_000_000,
        input_cost_per_million: 1.25,
        output_cost_per_million: 10.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "google/gemini-2.5-flash",
        context_window: 1_000_000,
        input_cost_per_million: 0.15,
        output_cost_per_million: 0.60,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    // Served through the GitHub Copilot subscription proxy, not Google's own
    // API: Copilot caps prompts at 200K regardless of the model's native 1M
    // window (live-verified against GET https://api.githubcopilot.com/models
    // — max_prompt_tokens=200000, max_context_window_tokens=265536). Billed
    // through the Copilot subscription's request multiplier, not a per-token
    // rate, hence zero cost fields.
    ModelInfo {
        id: "gemini-3.8-flash",
        context_window: 200_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    // OpenAI: cache_read = 50% of input
    ModelInfo {
        id: "openai/gpt-4.1",
        context_window: 1_000_000,
        input_cost_per_million: 2.0,
        output_cost_per_million: 8.0,
        cache_read_cost_per_million: 0.50,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "gpt-4.1-nano",
        context_window: 1_000_000,
        input_cost_per_million: 0.05,
        output_cost_per_million: 0.20,
        cache_read_cost_per_million: 0.025,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "gpt-4o-mini",
        context_window: 128_000,
        input_cost_per_million: 0.15,
        output_cost_per_million: 0.60,
        cache_read_cost_per_million: 0.075,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "gpt-4.1-mini",
        context_window: 1_000_000,
        input_cost_per_million: 0.40,
        output_cost_per_million: 1.60,
        cache_read_cost_per_million: 0.10,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "gpt-5.4-nano",
        context_window: 272_000,
        input_cost_per_million: 0.20,
        output_cost_per_million: 1.25,
        cache_read_cost_per_million: 0.10,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "gpt-5.4-mini",
        context_window: 272_000,
        input_cost_per_million: 0.75,
        output_cost_per_million: 4.50,
        cache_read_cost_per_million: 0.375,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "o4-mini",
        context_window: 200_000,
        input_cost_per_million: 1.10,
        output_cost_per_million: 4.40,
        cache_read_cost_per_million: 0.275,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "o3-mini",
        context_window: 200_000,
        input_cost_per_million: 0.55,
        output_cost_per_million: 2.20,
        cache_read_cost_per_million: 0.275,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "gpt-5.4",
        context_window: 272_000,
        input_cost_per_million: 2.50,
        output_cost_per_million: 15.0,
        cache_read_cost_per_million: 1.25,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "openai/o3",
        context_window: 200_000,
        input_cost_per_million: 10.0,
        output_cost_per_million: 40.0,
        cache_read_cost_per_million: 2.50,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "minimax/minimax-m2.7",
        context_window: 1_000_000,
        input_cost_per_million: 0.50,
        output_cost_per_million: 2.00,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "qwen/qwen3.6-plus-preview:free",
        context_window: 128_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "moonshotai/kimi-k2.5",
        context_window: 131_072,
        input_cost_per_million: 0.60,
        output_cost_per_million: 2.50,
        cache_read_cost_per_million: 0.15,
        cache_write_cost_per_million: 0.0,
    },
    // Kimi Code subscription (api.kimi.com/coding) — billed against a fixed
    // monthly quota, not pay-per-token, so there's no real $/M-token figure.
    // Zeroed here (matching this file's convention for other flat-rate/free
    // entries, e.g. qwen3.6-plus-preview:free above) rather than guessing a
    // number — the real gap this leaves is that GovernanceEnvelope's
    // spend_cap_usd tracking won't see any cost for Kimi Code usage at all.
    // Flagged, not fixed: quota consumption isn't $-denominated the same way,
    // so "fixing" this needs a token-budget concept this router doesn't have
    // yet, not just a nonzero placeholder price.
    ModelInfo {
        id: "kimi-for-coding",
        context_window: 262_144,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "kimi-for-coding-highspeed",
        context_window: 262_144,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "k3",
        context_window: 1_048_576,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "k3-256k",
        context_window: 262_144,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    // DeepSeek direct API (api.deepseek.com)
    ModelInfo {
        id: "deepseek-v4-pro",
        context_window: 1_048_576,
        input_cost_per_million: 0.28,
        output_cost_per_million: 0.42,
        cache_read_cost_per_million: 0.028,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "deepseek-v4-flash",
        context_window: 1_048_576,
        input_cost_per_million: 0.14,
        output_cost_per_million: 0.28,
        cache_read_cost_per_million: 0.014,
        cache_write_cost_per_million: 0.0,
    },
    // Real, current id for DeepSeek-V4.1-Flash (shipped 2026-09-10); same
    // pricing as the legacy "deepseek-v4-flash" id above, which DeepSeek's
    // own docs say now temporarily routes to this same model.
    ModelInfo {
        id: "deepseek-flash",
        context_window: 1_048_576,
        input_cost_per_million: 0.14,
        output_cost_per_million: 0.28,
        cache_read_cost_per_million: 0.014,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "deepseek-chat",
        context_window: 128_000,
        input_cost_per_million: 0.28,
        output_cost_per_million: 0.42,
        cache_read_cost_per_million: 0.028,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "deepseek-reasoner",
        context_window: 128_000,
        input_cost_per_million: 0.55,
        output_cost_per_million: 2.19,
        cache_read_cost_per_million: 0.055,
        cache_write_cost_per_million: 0.0,
    },
    // Novita AI (novita.ai)
    ModelInfo {
        id: "novita/deepseek-v3",
        context_window: 131_000,
        input_cost_per_million: 0.14,
        output_cost_per_million: 0.28,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "novita/qwen3-coder-30b",
        context_window: 160_000,
        input_cost_per_million: 0.07,
        output_cost_per_million: 0.07,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    // SiliconFlow (siliconflow.cn)
    ModelInfo {
        id: "siliconflow/deepseek-v3",
        context_window: 128_000,
        input_cost_per_million: 0.27,
        output_cost_per_million: 0.41,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "siliconflow/qwen3-235b",
        context_window: 128_000,
        input_cost_per_million: 0.49,
        output_cost_per_million: 0.49,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    // Zhipu GLM (free tier)
    ModelInfo {
        id: "glm-4.7-flash",
        context_window: 200_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    // Local Ollama models — Qwen3.5 family (April 2026, pulled on M4 MacBook)
    ModelInfo {
        id: "ollama/qwen3.5:9b",
        context_window: 32_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "ollama/qwen3.5:4b",
        context_window: 32_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "ollama/qwen3.5:2b",
        context_window: 32_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "ollama/qwen3.5:0.8b",
        context_window: 32_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    // IBM Granite 4.1 — code-specialized, on-disk
    ModelInfo {
        id: "ollama/granite4.1:8b",
        context_window: 128_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "ollama/granite4.1:3b",
        context_window: 128_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    // Legacy Qwen3 entries (kept for backward compat)
    ModelInfo {
        id: "ollama/qwen3:14b",
        context_window: 128_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "ollama/qwen3:8b",
        context_window: 128_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "ollama/qwen3:1.7b",
        context_window: 32_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    // Gemma4 family (uses prompted_tools shim for function calling)
    ModelInfo {
        id: "ollama/gemma4:26b",
        context_window: 128_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "ollama/gemma4:e4b",
        context_window: 128_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "ollama/gemma4:e2b",
        context_window: 128_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    // TurboQuant local inference (3-bit KV cache compression, ~5x memory reduction)
    ModelInfo {
        id: "tq/qwen2.5-3b-turboquant",
        context_window: 32_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "tq/qwen2.5-7b-turboquant",
        context_window: 32_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "tq/qwen2.5-72b-turboquant",
        context_window: 48_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
    ModelInfo {
        id: "tq/llama3.3-70b-turboquant",
        context_window: 48_000,
        input_cost_per_million: 0.0,
        output_cost_per_million: 0.0,
        cache_read_cost_per_million: 0.0,
        cache_write_cost_per_million: 0.0,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_supports_vision_matches_the_allowlist_only() {
        assert!(model_supports_vision("deepseek-flash"));
        assert!(!model_supports_vision("unknown-model"));
        assert!(!model_supports_vision("deepseek-v4-pro"));
    }

    #[test]
    fn test_calculate_cost() {
        let model = "anthropic/claude-sonnet-4-6";
        // input: 3.0 / M, output: 15.0 / M
        let cost = calculate_cost(model, 1_000_000, 1_000_000);
        assert_eq!(cost, 18.0);

        let cost_half = calculate_cost(model, 500_000, 500_000);
        assert_eq!(cost_half, 9.0);

        let unknown = calculate_cost("unknown", 100, 100);
        assert_eq!(unknown, 0.0);
    }

    #[test]
    fn test_calculate_cost_with_cache() {
        let model = "anthropic/claude-sonnet-4-6";
        // 1M input (500K cached), 1M output
        // Fresh input: 500K * 3.0/M = 1.5
        // Cache read: 500K * 0.30/M = 0.15
        // Output: 1M * 15.0/M = 15.0
        let cost = calculate_cost_with_cache(model, 1_000_000, 1_000_000, 500_000, 0);
        assert!((cost - 16.65).abs() < 0.001);

        // All cached: 1M input all from cache
        // Fresh: 0, Cache read: 1M * 0.30/M = 0.30, Output: 1M * 15.0/M = 15.0
        let cost_all_cached = calculate_cost_with_cache(model, 1_000_000, 1_000_000, 1_000_000, 0);
        assert!((cost_all_cached - 15.30).abs() < 0.001);
    }

    #[test]
    fn normalize_maps_dated_anthropic_snapshot_to_canonical() {
        // The Messages API echoes a dated snapshot without the routing prefix.
        assert_eq!(
            normalize_model_id("claude-sonnet-4-6-20260514"),
            "anthropic/claude-sonnet-4-6"
        );
        // Bare, undated id still gains the prefix.
        assert_eq!(
            normalize_model_id("claude-opus-4"),
            "anthropic/claude-opus-4"
        );
        // Already-qualified ids are untouched.
        assert_eq!(
            normalize_model_id("anthropic/claude-sonnet-4-6"),
            "anthropic/claude-sonnet-4-6"
        );
        // A trailing non-date segment is not mistaken for a snapshot.
        assert_eq!(
            normalize_model_id("claude-haiku-4"),
            "anthropic/claude-haiku-4"
        );
        // Non-Anthropic bare ids pass through.
        assert_eq!(normalize_model_id("gpt-4.1-nano"), "gpt-4.1-nano");
    }

    #[test]
    fn fresh_install_defaults_are_priced() {
        for id in ["anthropic/claude-haiku-5.5", "openai/gpt-6-luna"] {
            let info = get_model_info(id).unwrap_or_else(|| panic!("{id} is not in the registry"));
            assert!(info.input_cost_per_million > 0.0, "{id}");
        }
    }

    #[test]
    fn get_model_info_resolves_dated_anthropic_snapshot() {
        // A raw snapshot the Anthropic API reports must price like the canonical
        // registry entry rather than falling through to the $0 unknown path.
        let info = get_model_info("claude-sonnet-4-6-20260514")
            .expect("dated snapshot should resolve via normalization");
        assert_eq!(info.id, "anthropic/claude-sonnet-4-6");
        assert_eq!(info.input_cost_per_million, 3.0);

        // And the cost helper agrees for the dated id.
        let cost = calculate_cost_with_cache("claude-sonnet-4-6-20260514", 1_000_000, 0, 0, 0);
        assert!((cost - 3.0).abs() < 0.001);

        // Genuinely unknown models still price at zero.
        assert!(get_model_info("totally-made-up-model").is_none());
    }

    #[test]
    fn get_model_info_resolves_provider_qualified_first_party_ids() {
        // `openai/gpt-5.4` must resolve to the bare `gpt-5.4` registry entry —
        // the provider prefix is stripped for first-party vendors.
        let gpt = get_model_info("openai/gpt-5.4")
            .expect("openai/gpt-5.4 should resolve to the bare gpt-5.4 entry");
        assert_eq!(gpt.id, "gpt-5.4");
        assert_eq!(gpt.input_cost_per_million, 2.50);
        assert_eq!(gpt.output_cost_per_million, 15.0);
        // Cost is non-zero for the qualified id.
        let cost = calculate_cost("openai/gpt-5.4", 1_000_000, 1_000_000);
        assert!((cost - 17.50).abs() < 0.001);

        // An already-canonical anthropic id resolves by exact match.
        let claude = get_model_info("anthropic/claude-sonnet-4-6")
            .expect("anthropic/claude-sonnet-4-6 is a registry key");
        assert_eq!(claude.id, "anthropic/claude-sonnet-4-6");
        assert_eq!(claude.input_cost_per_million, 3.0);

        // A google-qualified id resolves by exact match too.
        let gemini =
            get_model_info("google/gemini-2.5-pro").expect("google/gemini-2.5-pro is a key");
        assert_eq!(gemini.id, "google/gemini-2.5-pro");
    }

    #[test]
    fn get_model_info_reverse_resolves_bare_first_party_ids() {
        // The registry stores `openai/gpt-4.1` prefixed; a bare `gpt-4.1` query
        // must still resolve by adding the first-party prefix.
        let info =
            get_model_info("gpt-4.1").expect("bare gpt-4.1 should resolve to openai/gpt-4.1");
        assert_eq!(info.id, "openai/gpt-4.1");

        // A dated bare snapshot resolves through date-strip + prefix-add.
        let dated = get_model_info("gpt-5.4-20260101")
            .expect("dated bare gpt-5.4 snapshot should resolve to gpt-5.4");
        assert_eq!(dated.id, "gpt-5.4");
    }

    #[test]
    fn openrouter_community_prefixes_are_never_stripped() {
        // A community `vendor/model` id resolves only by its exact (prefixed) key.
        let m2 = get_model_info("minimax/minimax-m2.7").expect("community id resolves exactly");
        assert_eq!(m2.id, "minimax/minimax-m2.7");

        // Crucially, a community prefix must NOT be stripped: `minimax/gpt-5.4`
        // is unknown and must price at zero rather than aliasing to `gpt-5.4`.
        assert!(
            get_model_info("minimax/gpt-5.4").is_none(),
            "community vendor prefix must not be stripped into a first-party model"
        );
        assert_eq!(calculate_cost("minimax/gpt-5.4", 1_000, 1_000), 0.0);

        // An unknown model under a first-party prefix also stays zero.
        assert!(get_model_info("openai/nonexistent-model").is_none());
    }
}
