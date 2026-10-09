//! Backend, utility provider and session config for `SessionBuilder::build`.

use std::sync::Arc;

use anyhow::{Context, Result};
use hq_core::config::HqConfig;
use hq_llm::provider::LlmProvider;
use tracing::info;

use crate::backend::SessionBackend;
use crate::session::SessionConfig;

/// The turn-driving chain (when `backends:` is configured) and the utility
/// provider used for compaction and other side calls.
pub(super) struct Backends {
    pub(super) chain: Option<Arc<crate::backend::ProviderChain>>,
    pub(super) provider: Arc<dyn LlmProvider>,
    /// An explicit `.provider(...)` override also takes over turn execution.
    pub(super) has_override: bool,
}

pub(super) fn resolve_backends(
    config: &HqConfig,
    provider_override: Option<Arc<dyn LlmProvider>>,
    shared_db: Option<&Arc<hq_db::Database>>,
) -> Result<Backends> {
    // Every provider reports to the process-wide ledger and budget gate, so turns driven by a
    // configured backend chain are recorded as well as router calls.
    if let Some(db) = shared_db {
        crate::install_ledger(db.clone());
    }
    // Attached to the legacy router so every LLM call reports its outcome.
    let outcome_sink: Option<hq_llm::SharedSink> = shared_db
        .map(|db| crate::outcome_sink::DbOutcomeSink::new(db.clone()) as hq_llm::SharedSink);
    // Build the configured backend chain first (if any).
    //
    // An explicit `backends` chain drives turns; its API backend also
    // supplies the utility/compaction provider, so a standalone `backends`
    // config needs no legacy provider at all. A pure-CLI chain has no API
    // provider, in which case compaction is routed through the backend itself
    // via `BackendUtilityProvider`. Building the chain before the provider is
    // what lets a chain-only config skip legacy provider construction
    // entirely (which would otherwise fail with no providers configured).
    let configured_chain: Option<Arc<crate::backend::ProviderChain>> = if config
        .backends
        .is_configured()
    {
        match crate::backend::BackendRegistry::from_config(config)
            .and_then(|registry| registry.build_chain())
        {
            Ok(chain) => {
                info!(
                    backends = chain.len(),
                    "SessionBuilder: driving turns through configured backend chain"
                );
                Some(Arc::new(chain))
            }
            Err(e) => {
                // Non-fatal: fall back to the legacy adapter so a
                // misconfigured chain never bricks the session.
                tracing::warn!(
                    %e,
                    "backends config present but chain unavailable; using legacy provider adapter"
                );
                None
            }
        }
    } else {
        None
    };

    // Resolve the utility/compaction LLM provider.
    //
    // Priority: explicit `.provider(...)` override > a provider derived from
    // the configured chain (its API backend, or the chain itself for a
    // pure-CLI chain) > legacy multi-provider construction. This lets a
    // standalone `backends` config work with no legacy provider while
    // preserving provider overrides and the legacy path for configs without a
    // chain.
    //
    // An explicit override also takes over *turn execution* (see `build`): it
    // is adapted into a single `ApiBackend` and the configured chain is
    // ignored, so `.provider(...)` behaves as documented even when a
    // `backends` config is present.
    let has_provider_override = provider_override.is_some();
    let provider: Arc<dyn LlmProvider> = match provider_override {
        Some(p) => p,
        None => match &configured_chain {
            Some(chain) => chain.utility_provider().unwrap_or_else(|| {
                Arc::new(crate::backend::BackendUtilityProvider::new(chain.clone()))
            }),
            None => build_provider_from_config(config, outcome_sink.clone(), shared_db)
                .context("failed to build LLM provider from config")?,
        },
    };

    Ok(Backends {
        chain: configured_chain,
        provider,
        has_override: has_provider_override,
    })
}

pub(super) async fn resolve_session_config(
    config: &HqConfig,
    session_config_override: Option<SessionConfig>,
) -> SessionConfig {
    let mut session_config =
        session_config_override.unwrap_or_else(|| session_config_from_hq_config(config));
    // Derive the context window from the model catalog when the config is
    // still on the generic default: a 1M-context model (k3) should not be
    // budgeted (and compacted) as if it had 200K, and a 128K model should
    // not be over-budgeted. An explicit non-default override always wins.
    // Captured before either adjustment below, since the static lookup
    // can itself move the value off the default marker.
    let context_window_was_default =
        session_config.context_window == SessionConfig::default().context_window;
    if context_window_was_default
        && let Some(info) = hq_llm::models::get_model_info(&session_config.model)
    {
        let known = info.context_window as usize;
        if known > 0 && known != session_config.context_window {
            tracing::debug!(
                model = %session_config.model,
                context_window = known,
                "SessionBuilder: derived context window from model catalog"
            );
            session_config.context_window = known;
        }
    }

    // GitHub Copilot enforces its own per-model prompt-token cap,
    // independent of (and sometimes far smaller than) the model's native
    // context window — e.g. Gemini 3.8 Flash: 1M native vs Copilot's
    // live-verified 200K. The static table above can drift from this cap
    // silently (it did: Copilot's own catalog is the only source of
    // truth), so when this model is routed through a Copilot endpoint,
    // a live check refines whatever the static path just set.
    if context_window_was_default {
        let routed_through_copilot = config
            .backends
            .model_routed_through_copilot(&session_config.model);
        if routed_through_copilot
            && let Some(live) = hq_llm::copilot::live_context_window(&session_config.model).await
        {
            let live = live as usize;
            if live > 0 && live != session_config.context_window {
                tracing::debug!(
                    model = %session_config.model,
                    context_window = live,
                    "SessionBuilder: derived context window from Copilot's live model catalog"
                );
                session_config.context_window = live;
            }
        }
    }

    // A background, watch or sub-agent run with no cap of its own gets the owner's per-run
    // ceiling, so a runaway loop cannot drain a month.
    if !session_config.is_live_user_turn
        && let Some(ceiling) = config.budgets.background_run_usd
    {
        session_config.max_budget_usd =
            Some(session_config.max_budget_usd.map_or(ceiling, |own| own.min(ceiling)));
    }
    session_config
}

/// Build a multi-provider LLM chain from config.
///
/// Sources (in priority order):
/// 1. Config-driven providers (config.providers list — DeepSeek, Novita, SiliconFlow, etc.)
/// 2. Environment-detected providers (Cerebras, Groq, Ollama, TurboQuant via LlmRouter::from_env)
/// 3. Legacy OpenRouter key (config.openrouter_api_key — kept as fallback)
///
/// Any model not matched by explicit routes falls through round-robin to all registered providers.
pub(super) fn build_provider_from_config(
    config: &HqConfig,
    outcome_sink: Option<hq_llm::SharedSink>,
    db: Option<&Arc<hq_db::Database>>,
) -> Result<Arc<dyn LlmProvider>> {
    // Start with env-detected providers. When local_only, skip cloud providers
    // and hardware-recommended routes entirely — we add explicit alias routes below.
    let mut router = if config.local_only {
        hq_llm::router::LlmRouter::from_env_local_only()
    } else {
        hq_llm::router::LlmRouter::from_env()
    };
    if let Some(sink) = outcome_sink {
        router.set_outcome_sink(sink);
    }

    if config.local_only {
        // Explicit aliases for the on-disk model roster (M4 MacBook).
        // Overrides hardware-detection routes so we use what's actually pulled.
        router.add_local_route("relay", "ollama", "gemma4:e4b");
        router.add_local_route("plan", "ollama", "gemma4:e4b");
        router.add_local_route("premium", "ollama", "gemma4:e4b");
        router.add_local_route("code", "ollama", "granite4.1:8b");
        router.add_local_route("mid", "ollama", "qwen3.5:9b");
        router.add_local_route("fast", "ollama", "qwen3.5:4b");
        router.add_local_route("bulk", "ollama", "qwen3.5:4b");
        router.add_local_route("verify", "ollama", "qwen3.5:4b");
        router.add_local_route("critic", "ollama", "qwen3.5:4b");
        router.add_local_route("notification", "ollama", "qwen3.5:0.8b");
        router.add_local_route("nudge", "ollama", "qwen3.5:0.8b");
    } else {
        // Add config-driven providers (the primary path for cloud mode)
        for pc in &config.providers {
            if !pc.enabled {
                continue;
            }
            let api_key = std::env::var(&pc.api_key_env).unwrap_or_default();
            if api_key.is_empty() {
                info!(
                    provider = %pc.name,
                    env = %pc.api_key_env,
                    "provider skipped: no API key"
                );
                continue;
            }

            let provider = Arc::new(hq_llm::openai_compat::OpenRouterProvider::new_with_base(
                &api_key,
                &pc.api_base,
            ));
            router.add_provider(&pc.name, provider);

            let cost_tier = match pc.tier {
                0 => hq_llm::router::CostTier::Free,
                1 => hq_llm::router::CostTier::Budget,
                2 => hq_llm::router::CostTier::Standard,
                _ => hq_llm::router::CostTier::Premium,
            };
            for model in &pc.models {
                router.add_route(model, &pc.name, model, cost_tier);
            }

            info!(
                provider = %pc.name,
                tier = pc.tier,
                models = pc.models.len(),
                "provider registered from config"
            );
        }

        // Legacy: add OpenRouter if key is set (backward compat)
        let or_key = config
            .openrouter_api_key
            .clone()
            .filter(|k| !k.is_empty())
            .or_else(|| std::env::var("OPENROUTER_API_KEY").ok())
            .unwrap_or_default();

        if !or_key.is_empty() {
            let or_provider = Arc::new(hq_llm::openai_compat::OpenRouterProvider::new(&or_key));
            router.add_provider("openrouter", or_provider);
            info!("OpenRouter added as fallback provider");
        }
    }

    if router.is_empty() {
        anyhow::bail!(
            "No LLM providers available. Configure at least one API key (openrouter_api_key, \
             anthropic_api_key, deepseek_api_key, etc. — via config.yaml or its HQ_* env var) \
             or run a local Ollama instance (ollama serve)."
        );
    }

    // Warm up router health from the last 24h of outcome data in vault.db.
    // Non-fatal: if the DB is unavailable the router starts cold.
    if let Some(db) = db {
        let raw = db.with_conn(|conn| hq_db::task_outcomes::recent_seed_rows(conn, 86_400));
        if let Ok(rows) = raw
            && !rows.is_empty()
        {
            let tuples: Vec<(&str, &str, bool, u64)> = rows
                .iter()
                .map(|r| {
                    (
                        r.provider.as_str(),
                        r.task_hint.as_str(),
                        r.success,
                        r.latency_ms,
                    )
                })
                .collect();
            router.seed_from_outcomes(&tuples);
            tracing::debug!(count = rows.len(), "router health seeded from vault.db");
        }
    }

    info!(
        model = %config.default_model,
        "multi-provider chain ready"
    );

    Ok(Arc::new(router) as Arc<dyn LlmProvider>)
}

/// Derive a `SessionConfig` from `HqConfig`, including budget caps.
pub(super) fn session_config_from_hq_config(config: &HqConfig) -> SessionConfig {
    SessionConfig {
        model: hq_core::config::resolve_session_model(config),
        max_budget_usd: Some(config.budget.session_cap_usd),
        background_review: config.governance.background_review,
        ..SessionConfig::default()
    }
}
