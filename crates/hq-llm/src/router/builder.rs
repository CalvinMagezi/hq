use std::sync::Arc;

use tracing::info;

use crate::cost::ProviderClass;
use crate::openai_compat::GEMINI_OPENAI_BASE_URL;
use crate::provider::LlmProvider;

use super::LlmRouter;
use super::health::ProviderHealth;
use super::types::CostTier;

const GROQ_BASE_URL: &str = "https://api.groq.com/openai/v1";
const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com/v1";
const SILICONFLOW_BASE_URL: &str = "https://api.siliconflow.cn/v1";
const NOVITA_BASE_URL: &str = "https://api.novita.ai/v3/openai";
const MOONSHOT_BASE_URL: &str = "https://api.moonshot.ai/v1";
const KIMI_CODE_BASE_URL: &str = "https://api.kimi.com/coding/v1";
const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";

/// Which key (if any) should activate the Kimi Code subscription provider.
/// Prefers a dedicated `KIMI_CODE_API_KEY`; otherwise accepts `KIMI_API_KEY`
/// only if it's actually an `sk-kimi-`-prefixed Kimi Code key rather than a
/// legacy Moonshot platform key, so a user who put their new Kimi Code key in
/// the old env var by mistake still gets routed to the right host instead of
/// silently landing on `api.moonshot.ai`. Pure and env-independent so it's
/// directly testable without mutating global process state.
fn select_kimi_code_key(kimi_code_env: Option<&str>, kimi_env: Option<&str>) -> Option<String> {
    kimi_code_env
        .filter(|k| !k.is_empty())
        .or_else(|| kimi_env.filter(|k| !k.is_empty() && k.starts_with("sk-kimi-")))
        .map(String::from)
}

/// The config.yaml key backing an env var in `CONFIG_BACKED_KEYS`.
/// Empty config values count as unset.
fn config_key(cfg: Option<&hq_core::config::HqConfig>, env: &str) -> Option<String> {
    let cfg = cfg?;
    let value = match env {
        "CEREBRAS_API_KEY" => &cfg.cerebras_api_key,
        "GROQ_API_KEY" => &cfg.groq_api_key,
        "DEEPSEEK_API_KEY" => &cfg.deepseek_api_key,
        "OPENAI_API_KEY" => &cfg.openai_api_key,
        "KIMI_CODE_API_KEY" => &cfg.kimi_code_api_key,
        _ => return None,
    };
    value.clone().filter(|k| !k.is_empty())
}

/// Env vars that `config_key` can back from config.yaml.
const CONFIG_BACKED_KEYS: &[&str] = &[
    "CEREBRAS_API_KEY",
    "GROQ_API_KEY",
    "DEEPSEEK_API_KEY",
    "OPENAI_API_KEY",
    "KIMI_CODE_API_KEY",
];

/// Copy config-only keys into the process env, never overriding a set var.
fn export_config_keys(cfg: Option<&hq_core::config::HqConfig>) {
    for &env in CONFIG_BACKED_KEYS {
        if std::env::var(env).is_ok() {
            continue;
        }
        if let Some(value) = config_key(cfg, env) {
            // Still read from env by Cerebras/GroqProvider::from_env, credential_env lookups and child processes.
            unsafe { std::env::set_var(env, value) };
        }
    }
}

/// How a provider's API key is picked out of the env/config lookup.
enum KeyRule {
    /// Any value, even an empty one.
    Any(&'static str),
    NonEmpty(&'static str),
    /// Legacy Moonshot platform keys only: an `sk-kimi-` key belongs to Kimi
    /// Code and would silently mis-route to api.moonshot.ai.
    Moonshot,
    /// See [`select_kimi_code_key`].
    KimiCode,
}

impl KeyRule {
    fn resolve(&self, key: &dyn Fn(&str) -> Option<String>) -> Option<String> {
        match self {
            KeyRule::Any(env) => key(env),
            KeyRule::NonEmpty(env) => key(env).filter(|k| !k.is_empty()),
            KeyRule::Moonshot => {
                key("KIMI_API_KEY").filter(|k| !k.is_empty() && !k.starts_with("sk-kimi-"))
            }
            KeyRule::KimiCode => select_kimi_code_key(
                key("KIMI_CODE_API_KEY").as_deref(),
                key("KIMI_API_KEY").as_deref(),
            ),
        }
    }
}

enum Client {
    Cerebras,
    OpenRouter,
    Base(&'static str),
}

/// One cloud provider: registered, with its own routes, when its key resolves.
struct ProviderSpec {
    name: &'static str,
    key: KeyRule,
    client: Client,
    daily_token_limit: u64,
    /// (pattern, model id, cost tier)
    routes: &'static [(&'static str, &'static str, CostTier)],
}

use CostTier::{Budget, Free, Premium, Standard};

/// Registered before the local providers, the rest after, so registration
/// order (and with it round-robin tie-breaks) stays as it has always been.
const FREE_TIER_PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        name: "cerebras",
        key: KeyRule::Any("CEREBRAS_API_KEY"),
        client: Client::Cerebras,
        daily_token_limit: 1_000_000,
        routes: &[
            ("cerebras/*", "", Free),
            ("cerebras/glm-4.7", "zai-glm-4.7", Free),
            ("cerebras/gemma", "gemma-4-31b", Free),
            ("cerebras/gpt-oss", "gpt-oss-120b", Free),
            ("notification", "gemma-4-31b", Free),
            ("nudge", "gemma-4-31b", Free),
        ],
    },
    ProviderSpec {
        name: "groq",
        key: KeyRule::Any("GROQ_API_KEY"),
        client: Client::Base(GROQ_BASE_URL),
        daily_token_limit: 500_000,
        routes: &[
            ("groq/*", "", Free),
            ("groq/llama-8b", "llama-3.1-8b-instant", Free),
            ("groq/llama-70b", "llama-3.3-70b-versatile", Free),
            ("groq/gpt-oss-120b", "openai/gpt-oss-120b", Free),
            ("groq/gpt-oss-20b", "openai/gpt-oss-20b", Free),
            ("notification", "llama-3.1-8b-instant", Free),
            ("nudge", "llama-3.1-8b-instant", Free),
        ],
    },
];

const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        name: "deepseek",
        key: KeyRule::NonEmpty("DEEPSEEK_API_KEY"),
        client: Client::Base(DEEPSEEK_BASE_URL),
        daily_token_limit: 0,
        routes: &[
            ("deepseek/*", "", Budget),
            ("deepseek/deepseek-coder", "deepseek-chat", Budget),
            ("deepseek/deepseek-chat", "deepseek-chat", Budget),
            ("deepseek/deepseek-reasoner", "deepseek-reasoner", Budget),
            // Bare ids so `default_model: deepseek-v4-pro` resolves without the prefix.
            ("deepseek-v4-pro", "deepseek-v4-pro", Budget),
            ("deepseek-v4-flash", "deepseek-v4-flash", Budget),
            // The real V4.1-Flash id; deepseek-v4-flash is only temporarily routed to it.
            ("deepseek-flash", "deepseek-flash", Budget),
        ],
    },
    ProviderSpec {
        name: "openrouter",
        key: KeyRule::NonEmpty("OPENROUTER_API_KEY"),
        client: Client::OpenRouter,
        daily_token_limit: 0,
        routes: &[
            ("openrouter/*", "", Standard),
            ("qwen/qwen3-coder", "qwen/qwen3-coder:free", Free),
            (
                "qwen/qwen3-coder-480b",
                "qwen/qwen3-coder-480b-a35b-instruct:free",
                Free,
            ),
            ("qwen/qwen3.6-plus", "qwen/qwen3.6-plus:free", Free),
            ("qwen/*", "", Free),
            (
                "deepseek/deepseek-r1-free",
                "deepseek/deepseek-r1:free",
                Free,
            ),
            (
                "google/gemini-2.0-flash-exp",
                "google/gemini-2.0-flash-exp:free",
                Free,
            ),
            (
                "meta-llama/llama-3.3-70b-free",
                "meta-llama/llama-3.3-70b-instruct:free",
                Free,
            ),
        ],
    },
    ProviderSpec {
        name: "gemini",
        key: KeyRule::NonEmpty("GEMINI_API_KEY"),
        client: Client::Base(GEMINI_OPENAI_BASE_URL),
        daily_token_limit: 0,
        routes: &[
            ("gemini/*", "", Budget),
            ("gemini/gemini-2.5-flash", "gemini-2.5-flash", Budget),
            ("gemini/gemini-2.5-pro", "gemini-2.5-pro", Standard),
        ],
    },
    // Moonshot's pay-per-token platform, not the Kimi Code subscription below.
    // Explicit routes only: a wildcard would forward invalid model names.
    ProviderSpec {
        name: "kimi",
        key: KeyRule::Moonshot,
        client: Client::Base(MOONSHOT_BASE_URL),
        daily_token_limit: 0,
        routes: &[
            ("kimi/moonshot-v1-8k", "moonshot-v1-8k", Standard),
            ("kimi/k2.5", "kimi-k2.5", Standard),
            ("kimi/k2", "kimi-k2", Standard),
            // OpenRouter-style ids so HQ_DEFAULT_MODEL=moonshotai/kimi-k2.5 resolves here.
            ("moonshotai/kimi-k2.5", "kimi-k2.5", Standard),
            ("moonshotai/kimi-k2", "kimi-k2", Standard),
        ],
    },
    // Kimi Code subscription over its OpenAI-compatible path, so it reuses
    // OpenRouterProvider unchanged.
    ProviderSpec {
        name: "kimi-code",
        key: KeyRule::KimiCode,
        client: Client::Base(KIMI_CODE_BASE_URL),
        daily_token_limit: 0,
        routes: &[
            ("kimi-code/kimi-for-coding", "kimi-for-coding", Standard),
            (
                "kimi-code/kimi-for-coding-highspeed",
                "kimi-for-coding-highspeed",
                Standard,
            ),
            ("kimi-code/k3", "k3", Standard),
            // Free tier because the subscription is flat-rate: it competes with
            // groq/cerebras on health and latency instead of losing on cost.
            ("notification", "kimi-for-coding", Free),
            ("nudge", "kimi-for-coding", Free),
        ],
    },
    ProviderSpec {
        name: "siliconflow",
        key: KeyRule::NonEmpty("SILICONFLOW_API_KEY"),
        client: Client::Base(SILICONFLOW_BASE_URL),
        daily_token_limit: 0,
        routes: &[
            ("siliconflow/*", "", Budget),
            ("siliconflow/deepseek-v3", "deepseek-ai/DeepSeek-V3", Budget),
            ("siliconflow/qwen3-235b", "Qwen/Qwen3-235B-A22B", Budget),
        ],
    },
    ProviderSpec {
        name: "novita",
        key: KeyRule::NonEmpty("NOVITA_API_KEY"),
        client: Client::Base(NOVITA_BASE_URL),
        daily_token_limit: 0,
        routes: &[
            ("novita/*", "", Budget),
            ("novita/deepseek-v3", "deepseek/deepseek_v3", Budget),
            ("novita/qwen3-coder-30b", "Qwen/Qwen3-Coder-30B-A3B", Budget),
        ],
    },
    ProviderSpec {
        name: "openai",
        key: KeyRule::NonEmpty("OPENAI_API_KEY"),
        client: Client::Base(OPENAI_BASE_URL),
        daily_token_limit: 0,
        routes: &[
            ("openai/*", "", Budget),
            ("openai/gpt-4.1-nano", "gpt-4.1-nano", Free),
            ("openai/gpt-5.4-nano", "gpt-5.4-nano", Budget),
            ("openai/gpt-4o-mini", "gpt-4o-mini", Budget),
            ("openai/gpt-4.1-mini", "gpt-4.1-mini", Budget),
            ("openai/gpt-5.4-mini", "gpt-5.4-mini", Standard),
            ("openai/o4-mini", "o4-mini", Standard),
            ("openai/o3-mini", "o3-mini", Standard),
            ("openai/gpt-5.4", "gpt-5.4", Premium),
            ("openai/gpt-4.1", "gpt-4.1", Standard),
        ],
    },
];

const OLLAMA_TOOL_MODEL: &str = "ornith:latest";
const LLAMA_70B_FREE: &str = "meta-llama/llama-3.3-70b-instruct:free";
/// Paid OpenRouter fallback for background aliases: the free Llama route above
/// is often unavailable, and an OpenRouter-only host has nothing else to try.
const OPENROUTER_BACKGROUND_FALLBACK: &str = "deepseek/deepseek-v4-flash";

/// (alias, provider, model id, cost tier), kept only for registered providers.
/// Ollama rows are local routes. Order matters for ties, so rows follow the
/// original registration order rather than grouping by provider.
const ALIAS_ROUTES: &[(&str, &str, &str, CostTier)] = &[
    // "fast"/"bulk": highest throughput. DeepSeek is the fallback for hosts
    // (like the VPS) with no free-tier key, and ranks below every Free route.
    ("fast", "cerebras", "gemma-4-31b", Free),
    ("bulk", "cerebras", "gemma-4-31b", Free),
    ("fast", "groq", "llama-3.1-8b-instant", Free),
    ("bulk", "groq", "llama-3.1-8b-instant", Free),
    ("fast", "openrouter", LLAMA_70B_FREE, Free),
    ("fast", "openrouter", OPENROUTER_BACKGROUND_FALLBACK, Budget),
    ("bulk", "openrouter", LLAMA_70B_FREE, Free),
    ("bulk", "openrouter", OPENROUTER_BACKGROUND_FALLBACK, Budget),
    ("fast", "openai", "gpt-4.1-nano", Budget),
    ("bulk", "openai", "gpt-4.1-nano", Budget),
    ("fast", "deepseek", "deepseek-chat", Budget),
    ("bulk", "deepseek", "deepseek-chat", Budget),
    // "dream"/"deep-sleep": memory consolidation. Separate from "fast"/"bulk"
    // because those serve tool-calling turns and this reply is plain text.
    ("dream", "cerebras", "gemma-4-31b", Free),
    ("deep-sleep", "cerebras", "gemma-4-31b", Free),
    ("dream", "groq", "llama-3.1-8b-instant", Free),
    ("deep-sleep", "groq", "llama-3.1-8b-instant", Free),
    ("dream", "openrouter", LLAMA_70B_FREE, Free),
    ("dream", "openrouter", OPENROUTER_BACKGROUND_FALLBACK, Budget),
    ("deep-sleep", "openrouter", LLAMA_70B_FREE, Free),
    ("deep-sleep", "openrouter", OPENROUTER_BACKGROUND_FALLBACK, Budget),
    ("dream", "openai", "gpt-4.1-nano", Budget),
    ("deep-sleep", "openai", "gpt-4.1-nano", Budget),
    ("dream", "deepseek", "deepseek-chat", Budget),
    ("deep-sleep", "deepseek", "deepseek-chat", Budget),
    ("mid", "groq", "llama-3.3-70b-versatile", Free),
    ("mid", "cerebras", "gemma-4-31b", Free),
    // Kimi K2.5 stays out of relay/premium/plan: it requires temperature=0.6
    // exactly and chat_session_config uses 0.3.
    ("relay", "deepseek", "deepseek-chat", Budget),
    ("premium", "deepseek", "deepseek-chat", Budget),
    ("relay", "siliconflow", "deepseek-ai/DeepSeek-V3", Budget),
    ("premium", "siliconflow", "deepseek-ai/DeepSeek-V3", Budget),
    ("relay", "novita", "deepseek/deepseek_v3", Budget),
    ("premium", "novita", "deepseek/deepseek_v3", Budget),
    ("relay", "cerebras", "zai-glm-4.7", Free),
    ("relay", "cerebras", "gpt-oss-120b", Free),
    ("premium", "cerebras", "zai-glm-4.7", Free),
    ("relay", "groq", "llama-3.3-70b-versatile", Free),
    ("premium", "groq", "llama-3.3-70b-versatile", Free),
    ("relay", "openrouter", "deepseek/deepseek-r1:free", Free),
    ("premium", "openrouter", "deepseek/deepseek-r1:free", Free),
    ("relay", "gemini", "gemini-2.5-flash", Budget),
    ("premium", "gemini", "gemini-2.5-flash", Budget),
    ("relay", "openai", "gpt-4o-mini", Budget),
    ("relay", "openai", "o3-mini", Standard),
    ("premium", "openai", "o4-mini", Standard),
    ("relay", "ollama", OLLAMA_TOOL_MODEL, Free),
    ("plan", "deepseek", "deepseek-chat", Budget),
    ("plan", "siliconflow", "deepseek-ai/DeepSeek-V3", Budget),
    ("plan", "novita", "deepseek/deepseek_v3", Budget),
    ("plan", "cerebras", "zai-glm-4.7", Free),
    ("plan", "groq", "llama-3.3-70b-versatile", Free),
    (
        "plan",
        "openrouter",
        "google/gemini-2.0-flash-exp:free",
        Free,
    ),
    ("plan", "gemini", "gemini-2.5-flash", Budget),
    ("plan", "openai", "o4-mini", Standard),
    ("code", "kimi", "kimi-k2.5", Standard),
    ("code", "deepseek", "deepseek-chat", Budget),
    ("code", "siliconflow", "Qwen/Qwen3-235B-A22B", Budget),
    ("code", "novita", "Qwen/Qwen3-Coder-30B-A3B", Budget),
    ("code", "groq", "llama-3.3-70b-versatile", Free),
    ("code", "cerebras", "zai-glm-4.7", Free),
    (
        "code",
        "openrouter",
        "qwen/qwen3-coder-480b-a35b-instruct:free",
        Free,
    ),
    ("code", "gemini", "gemini-2.5-flash", Budget),
    ("code", "openai", "o4-mini", Standard),
    ("code", "ollama", OLLAMA_TOOL_MODEL, Free),
    // "verify"/"critic": cheap models for sub-agent verification and review.
    // Cerebras is Standard, not Free: gemma-4-31b fences its JSON and truncates
    // at these budgets, so it stays only as Groq's fallback.
    ("verify", "openai", "gpt-4.1-nano", Budget),
    ("verify", "cerebras", "gemma-4-31b", Standard),
    ("verify", "groq", "llama-3.1-8b-instant", Free),
    ("verify", "deepseek", "deepseek-chat", Budget),
    ("verify", "ollama", OLLAMA_TOOL_MODEL, Free),
    ("critic", "openai", "gpt-4.1-nano", Budget),
    ("critic", "cerebras", "gemma-4-31b", Standard),
    ("critic", "groq", "llama-3.1-8b-instant", Free),
    ("critic", "deepseek", "deepseek-chat", Budget),
];

impl LlmRouter {
    /// Seed in-memory health windows from historical outcome data.
    /// Call at router startup with the last 24h of vault.db records to warm up
    /// provider scoring before any live traffic arrives.
    ///
    /// Each tuple is `(provider, task_hint_str, success, latency_ms)`.
    pub fn seed_from_outcomes(&mut self, outcomes: &[(&str, &str, bool, u64)]) {
        let mut health = self.health.lock().unwrap();
        for &(provider, task_hint_str, success, latency_ms) in outcomes {
            let task_hint = super::types::TaskHint::parse(task_hint_str);
            super::strategy::seed_health_entry(
                &mut health,
                provider,
                task_hint,
                &[(success, latency_ms)],
            );
        }
    }

    /// Ask `gate` before every provider attempt; a refusal stops that attempt.
    pub fn set_budget_gate(&mut self, gate: crate::budget::SharedGate) {
        self.instruments.set_gate(gate);
    }

    pub fn set_outcome_sink(&mut self, sink: crate::outcome_sink::SharedSink) {
        self.instruments.set_sink(sink);
    }

    /// A router whose providers report to `instruments` instead of a handle of their own.
    pub fn with_instruments(instruments: Arc<crate::instrument::Instruments>) -> Self {
        Self {
            instruments,
            ..Self::new()
        }
    }

    pub fn instruments(&self) -> Arc<crate::instrument::Instruments> {
        self.instruments.clone()
    }

    pub fn is_empty(&self) -> bool {
        self.providers.is_empty()
    }

    pub fn provider_count(&self) -> usize {
        self.providers.len()
    }

    pub fn route_count(&self) -> usize {
        self.routes.len()
    }

    pub fn add_provider(&mut self, name: &str, provider: Arc<dyn LlmProvider>) {
        self.add_provider_with_budget(name, provider, 0);
    }

    pub fn add_provider_with_budget(
        &mut self,
        name: &str,
        provider: Arc<dyn LlmProvider>,
        daily_token_limit: u64,
    ) {
        let provider = crate::instrument::InstrumentedProvider::wrap(
            provider,
            name,
            ProviderClass::of_name(name),
            self.instruments.clone(),
        );
        self.providers.push((name.to_string(), provider));
        let mut health = self.health.lock().unwrap();
        if !health.iter().any(|(n, _)| n == name) {
            let h = ProviderHealth {
                daily_token_limit,
                ..Default::default()
            };
            health.push((name.to_string(), h));
        }
    }

    pub fn add_route(
        &mut self,
        pattern: &str,
        provider: &str,
        model_id: &str,
        cost_tier: CostTier,
    ) {
        self.routes.push(super::types::RouteEntry {
            pattern: pattern.to_string(),
            provider: provider.to_string(),
            model_id: model_id.to_string(),
            cost_tier,
            is_local: false,
        });
    }

    pub fn add_local_route(&mut self, pattern: &str, provider: &str, model_id: &str) {
        self.routes.push(super::types::RouteEntry {
            pattern: pattern.to_string(),
            provider: provider.to_string(),
            model_id: model_id.to_string(),
            cost_tier: CostTier::Free,
            is_local: true,
        });
    }
}

impl LlmRouter {
    /// When `backends:` is configured, send every alias to that chain in its
    /// declared order, so background calls use the same primary and fallbacks
    /// as HQ's sessions instead of the built-in provider table.
    pub fn from_backends(config: &hq_core::config::HqConfig) -> Option<Self> {
        if !config.backends.is_configured() {
            return None;
        }
        let chain = crate::backend_chain::ChainProvider::from_config(config)?;
        info!(backends = ?chain.backend_names(), "LLM Router: routing all aliases through the backend chain");
        let mut router = Self::with_instruments(crate::instrument::Instruments::global());
        router.add_provider("backends", Arc::new(chain) as Arc<dyn LlmProvider>);
        router.add_route("*", "backends", "", CostTier::Budget);
        Some(router)
    }

    /// Build the default router from environment variables, with config.yaml
    /// keys exported to the env for the providers that have one.
    pub fn from_env() -> Self {
        let cfg = hq_core::config::HqConfig::load().ok();
        if let Some(cfg) = &cfg {
            // local_only skips every cloud provider.
            if cfg.local_only {
                return Self::from_env_local_only_with_aliases();
            }
            if let Some(router) = Self::from_backends(cfg) {
                return router;
            }
        }
        export_config_keys(cfg.as_ref());
        // Gated so a cloud-primary setup can keep Ollama out of the alias pool;
        // local_only above still reaches Ollama unconditionally.
        let local_ollama = cfg
            .as_ref()
            .map(|c| c.instance.features.local_ollama)
            .unwrap_or(true);
        Self::from_keys(&|name| std::env::var(name).ok(), local_ollama)
    }

    fn from_keys(key: &dyn Fn(&str) -> Option<String>, local_ollama: bool) -> Self {
        let mut router = Self::with_instruments(crate::instrument::Instruments::global());
        router.add_cloud_providers(FREE_TIER_PROVIDERS, key);
        #[cfg(feature = "turboquant")]
        router.add_turboquant();
        if local_ollama {
            router.add_hardware_ollama();
        }
        router.add_cloud_providers(PROVIDERS, key);

        // Model-agnostic aliases for whichever providers registered; score-based
        // selection picks among them.
        for &(alias, provider, model, tier) in ALIAS_ROUTES {
            if !router.providers.iter().any(|(n, _)| n == provider) {
                continue;
            }
            if provider == "ollama" {
                router.add_local_route(alias, provider, model);
            } else {
                router.add_route(alias, provider, model, tier);
            }
        }

        info!(
            "LLM Router: {} providers, {} routes (score-based routing enabled)",
            router.providers.len(),
            router.routes.len()
        );
        router
    }

    fn add_cloud_providers(
        &mut self,
        specs: &[ProviderSpec],
        key: &dyn Fn(&str) -> Option<String>,
    ) {
        for spec in specs {
            let Some(api_key) = spec.key.resolve(key) else {
                continue;
            };
            let provider: Arc<dyn LlmProvider> = match spec.client {
                Client::Cerebras => Arc::new(crate::cerebras::CerebrasProvider::new(&api_key)),
                Client::OpenRouter => {
                    Arc::new(crate::openai_compat::OpenRouterProvider::new(&api_key))
                }
                Client::Base(url) => Arc::new(
                    crate::openai_compat::OpenRouterProvider::new_with_base(&api_key, url),
                ),
            };
            self.add_provider_with_budget(spec.name, provider, spec.daily_token_limit);
            for &(pattern, model, tier) in spec.routes {
                self.add_route(pattern, spec.name, model, tier);
            }
            info!("LLM Router: {} provider added", spec.name);
        }
    }

    /// TurboQuant local inference, if its server is reachable.
    #[cfg(feature = "turboquant")]
    fn add_turboquant(&mut self) {
        let Ok(provider) = crate::turboquant::TurboQuantProvider::from_env() else {
            return;
        };
        let base = std::env::var("TURBOQUANT_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:14747/v1".to_string());
        if !crate::turboquant::is_available_sync(&base) {
            info!("LLM Router: TurboQuant server not reachable at {}", base);
            return;
        }
        let name = "turboquant";
        self.add_provider(name, Arc::new(provider) as Arc<dyn LlmProvider>);
        self.add_local_route("tq/*", name, "");
        self.add_local_route("local/*", name, "");
        for alias in ["fast", "bulk", "notification", "nudge"] {
            self.add_local_route(alias, name, "qwen2.5-3b-turboquant");
        }
        info!(
            "LLM Router: TurboQuant provider added (local, {}) with fast/bulk aliases",
            base
        );
    }

    /// Ollama with hardware-recommended models, if reachable.
    fn add_hardware_ollama(&mut self) {
        if !super::strategy::ollama_is_available_sync() {
            return;
        }
        let profile = hq_core::hardware::detect_hardware();
        let recommended = hq_core::hardware::recommend_models(&profile);
        if recommended.is_empty() {
            return;
        }
        let provider = Arc::new(crate::ollama::OllamaProvider::new()) as Arc<dyn LlmProvider>;
        self.add_provider("ollama", provider);
        self.add_local_route("ollama/*", "ollama", "");
        for rec in &recommended {
            self.add_local_route(&rec.alias, "ollama", &rec.ollama_model);
            if rec.alias == "fast" {
                self.add_local_route("notification", "ollama", &rec.ollama_model);
                self.add_local_route("nudge", "ollama", &rec.ollama_model);
            }
        }
        info!(
            "LLM Router: Ollama provider added ({} {}GB usable) with {} models: {}",
            match &profile.gpu_type {
                hq_core::hardware::GpuType::AppleSilicon { chip } => chip.clone(),
                _ => "CPU".into(),
            },
            profile.usable_for_models_gb,
            recommended.len(),
            recommended
                .iter()
                .map(|r| format!("{}={}", r.alias, r.ollama_model))
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    /// Build a router with only local inference providers (Ollama + TurboQuant).
    /// Use this when `local_only = true` in HqConfig.
    pub fn from_env_local_only() -> Self {
        let mut router = Self::with_instruments(crate::instrument::Instruments::global());

        // TurboQuant optional local inference server
        #[cfg(feature = "turboquant")]
        if let Ok(provider) = crate::turboquant::TurboQuantProvider::from_env() {
            let base = std::env::var("TURBOQUANT_BASE_URL")
                .unwrap_or_else(|_| "http://localhost:14747/v1".to_string());
            if crate::turboquant::is_available_sync(&base) {
                let provider = Arc::new(provider) as Arc<dyn LlmProvider>;
                router.add_provider("turboquant", provider);
                router.add_local_route("tq/*", "turboquant", "");
                router.add_local_route("local/*", "turboquant", "");
                info!("LLM Router (local-only): TurboQuant added at {}", base);
            }
        }

        // Ollama — register provider + wildcard only; no hardware-detected routes.
        // Explicit alias routes (relay/plan/code/fast/etc.) are added by builder.rs.
        if super::strategy::ollama_is_available_sync() {
            let provider = Arc::new(crate::ollama::OllamaProvider::new()) as Arc<dyn LlmProvider>;
            router.add_provider("ollama", provider);
            router.add_local_route("ollama/*", "ollama", "");
            info!("LLM Router (local-only): Ollama provider added");
        } else {
            info!("LLM Router (local-only): Ollama not reachable at localhost:11434");
        }

        router
    }

    /// Like `from_env_local_only()` but also adds the standard alias routes.
    /// Called by `from_env()` when local_only=true.
    pub fn from_env_local_only_with_aliases() -> Self {
        let mut router = Self::from_env_local_only();
        router.add_local_route("relay", "ollama", "granite4.1:8b");
        router.add_local_route("plan", "ollama", "granite4.1:8b");
        router.add_local_route("premium", "ollama", "granite4.1:8b");
        router.add_local_route("code", "ollama", "granite4.1:8b");
        router.add_local_route("mid", "ollama", "granite4.1:8b");
        router.add_local_route("fast", "ollama", "qwen3.5:4b");
        router.add_local_route("bulk", "ollama", "qwen3.5:4b");
        router.add_local_route("verify", "ollama", "qwen3.5:4b");
        router.add_local_route("critic", "ollama", "qwen3.5:4b");
        router.add_local_route("notification", "ollama", "qwen3.5:0.8b");
        router.add_local_route("nudge", "ollama", "qwen3.5:0.8b");
        info!("LLM Router (local-only): alias routes added (relay/plan/code/fast/etc. → Ollama)");
        router
    }
}

#[cfg(test)]
mod kimi_code_key_tests {
    use super::select_kimi_code_key;

    #[test]
    fn prefers_dedicated_kimi_code_env_var() {
        assert_eq!(
            select_kimi_code_key(Some("sk-kimi-abc123"), Some("legacy-moonshot-key")),
            Some("sk-kimi-abc123".to_string())
        );
    }

    #[test]
    fn falls_back_to_kimi_api_key_when_it_is_actually_a_kimi_code_key() {
        assert_eq!(
            select_kimi_code_key(None, Some("sk-kimi-xyz789")),
            Some("sk-kimi-xyz789".to_string())
        );
    }

    #[test]
    fn does_not_treat_a_legacy_moonshot_key_as_a_kimi_code_key() {
        // This is the exact mis-routing case the guard exists to prevent —
        // a legacy KIMI_API_KEY must never activate the Kimi Code provider.
        assert_eq!(
            select_kimi_code_key(None, Some("legacy-moonshot-key")),
            None
        );
    }

    #[test]
    fn returns_none_when_neither_env_var_is_set() {
        assert_eq!(select_kimi_code_key(None, None), None);
    }

    #[test]
    fn empty_strings_are_treated_as_unset() {
        assert_eq!(
            select_kimi_code_key(Some(""), Some("sk-kimi-real")),
            Some("sk-kimi-real".to_string())
        );
        assert_eq!(select_kimi_code_key(Some(""), Some("")), None);
    }
}
