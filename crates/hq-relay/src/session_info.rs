//! Session-reset banner — builds a dynamic "starting fresh" notice for relay
//! chat surfaces, modelled on the Hermes gateway reset banner.
//!
//! Instead of a bare "Conversation reset." string, the banner surfaces the
//! live session configuration (model, provider, detected context window, the
//! configured fallback chain) plus a rotating usage tip, so a reset is also a
//! quick health snapshot of what the next turn will actually run on.

use hq_core::config::HqConfig;
use hq_llm::models::context_window;

/// Load `HqConfig`, tolerating a missing/unreadable file by returning the
/// default so the banner still renders rather than erroring the reset.
fn load_config_lenient() -> HqConfig {
    HqConfig::load().unwrap_or_default()
}

/// One entry in the resolved backend chain, for display + switching.
pub struct BackendInfo {
    pub name: String,
    pub kind: String,
    pub model: Option<String>,
    pub is_primary: bool,
}

/// The ordered chain (primary first, then fallbacks) as display records.
/// Unknown names referenced by the chain are included but flagged with no kind.
pub fn chain_listing(config: &HqConfig) -> Vec<BackendInfo> {
    config
        .backends
        .chain_order()
        .into_iter()
        .map(|name| {
            let entry = config.backends.backend(&name);
            BackendInfo {
                is_primary: name == config.backends.primary,
                kind: entry
                    .map(|e| format!("{:?}", e.kind))
                    .unwrap_or_else(|| "unknown".to_string()),
                model: entry.and_then(|e| e.model.clone()),
                name,
            }
        })
        .collect()
}

/// Render the backend chain as a `header` line followed by one marked line
/// per entry (◆ primary, ◦ fallback). Shared by `/backend` and `/model`'s
/// bare-listing forms so both surfaces agree on what "the model list" is.
pub fn render_chain_listing(config: &HqConfig, header: &str) -> String {
    let mut lines = vec![header.to_string()];
    for info in chain_listing(config) {
        let marker = if info.is_primary { "◆" } else { "◦" };
        let model = info.model.unwrap_or_else(|| "(default)".to_string());
        lines.push(format!("{marker} `{}` — {model}", info.name));
    }
    lines.join("\n")
}

pub use hq_core::config::model_switch::{ModelResolution, resolve_model_arg, set_primary_backend};

/// Rotating tips shown at the bottom of the reset banner.
///
/// Keep these short, actionable, and focused on relay-surface features
/// (things a chat user can actually do). Add freely — one is picked per reset.
const RESET_TIPS: &[&str] = &[
    "!status shows the active model and message count without leaving the chat.",
    "!model with no argument lists the backend chain, marking which one is active.",
    "/watch <minutes> <prompt> keeps checking something for you; /unwatch <id> stops it.",
    "resume lists interrupted background turns; resume <id> reruns one.",
    "The fallback chain engages automatically if the primary model errors — !status shows the active one.",
    "Use !reset whenever a conversation drifts off-topic — a fresh thread beats wrestling stale context.",
    "hq doctor from a terminal runs a full health sweep: keys, DB, ports, and backends.",
    "hq config backends.primary shows the primary backend driving chat turns.",
    "Long replies are auto-chunked to fit platform message limits — paste away.",
];

/// Format a context-window size for display, mirroring the Hermes banner's
/// compact style (`1.0M`, `200K`).
fn format_context(tokens: u32) -> String {
    if tokens >= 1_000_000 {
        format!("{:.1}M", tokens as f64 / 1_000_000.0)
    } else if tokens >= 1_000 {
        format!("{}K", tokens / 1_000)
    } else {
        tokens.to_string()
    }
}

/// Pick a tip for this banner.
///
/// Rotation is process-lifetime cheap: derive an index from the current time
/// so successive resets show different tips without persisting state.
fn pick_tip() -> &'static str {
    if RESET_TIPS.is_empty() {
        return "";
    }
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    RESET_TIPS[nanos % RESET_TIPS.len()]
}

/// Build the fallback-chain line from the configured backends.
///
/// Returns e.g. `copilot → kimi-k3-256k → deepseek → ollama` when an explicit
/// chain is configured, or `None` when running on the legacy provider list.
fn fallback_chain_display(config: &HqConfig) -> Option<String> {
    if !config.backends.is_configured() {
        return None;
    }
    let chain = config.backends.chain_order();
    if chain.is_empty() {
        return None;
    }
    Some(chain.join(" → "))
}

/// Resolve the model a fresh session will actually start on.
///
/// Thin re-export of [`hq_core::config::resolve_session_model`], the shared
/// resolver also used by hq-agent's `SessionConfig` construction, so both
/// surfaces agree on what a session actually runs on. Order: chain primary
/// model → explicit `relay.model` override → `default_model`.
pub fn resolve_session_model(config: &HqConfig) -> String {
    hq_core::config::resolve_session_model(config)
}

/// Build the full session-reset banner from the live runtime config.
///
/// Loads `HqConfig` itself and resolves the session model via
/// [`resolve_session_model`] (chain-primary first), so the banner always
/// reflects what a reset actually leaves behind. This is the common entry
/// point for chat surfaces that don't already hold the model string.
pub async fn reset_banner_from_config() -> String {
    let config = load_config_lenient();
    let model = resolve_session_model(&config);
    reset_banner(&model, &config).await
}

/// Build the full session-reset banner.
///
/// `model` is the model the next turn will run on (the relay's active model),
/// and `config` supplies provider/backend metadata. Both come from the live
/// runtime, so the banner always reflects what a reset actually leaves behind.
pub async fn reset_banner(model: &str, config: &HqConfig) -> String {
    // The provider label is the configured primary backend when an explicit
    // chain exists, else the flat default (matching how turns are routed).
    let provider = if config.backends.is_configured() {
        config.backends.primary.clone()
    } else {
        "default".to_string()
    };

    let mut ctx = context_window(model);
    // GitHub Copilot enforces its own per-model prompt-token cap, independent
    // of (and sometimes far smaller than) a model's native context window —
    // e.g. Gemini 3.8 Flash: 1M native vs Copilot's live-verified 200K. The
    // static table above can silently drift from Copilot's own cap, so this
    // banner — the user-visible "what will the next turn actually run on"
    // snapshot — prefers a live check when the model is Copilot-routed.
    if config.backends.model_routed_through_copilot(model)
        && let Some(live) = hq_llm::copilot::live_context_window(model).await
        && live > 0
    {
        ctx = live;
    }
    let ctx_display = format_context(ctx);

    let mut lines = vec![
        "✨ Session reset! Starting fresh.".to_string(),
        String::new(),
        format!("◆ Model: `{model}`"),
        format!("◆ Provider: {provider}"),
        format!("◆ Context: {ctx_display} tokens"),
    ];

    if let Some(chain) = fallback_chain_display(config) {
        lines.push(format!("◆ Fallbacks: {chain}"));
    }

    let tip = pick_tip();
    if !tip.is_empty() {
        lines.push(format!("✦ Tip: {tip}"));
    }

    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::{BackendEntry, BackendKind, BackendsConfig};

    fn config_with_chain() -> HqConfig {
        let backends = BackendsConfig {
            primary: "copilot".to_string(),
            fallbacks: vec!["kimi".to_string(), "deepseek".to_string()],
            backends: vec![
                BackendEntry {
                    name: "copilot".to_string(),
                    kind: BackendKind::GithubCopilotCli,
                    endpoint: None,
                    credential_env: None,
                    model: Some("claude-sonnet-5".to_string()),
                    effort: None,
                    wire: Default::default(),
                    enabled: true,
                },
                BackendEntry {
                    name: "kimi".to_string(),
                    kind: BackendKind::KimiCode,
                    endpoint: None,
                    credential_env: None,
                    model: Some("k3-256k".to_string()),
                    effort: None,
                    wire: Default::default(),
                    enabled: true,
                },
                BackendEntry {
                    name: "deepseek".to_string(),
                    kind: BackendKind::OpenaiCompatible,
                    endpoint: Some("https://api.deepseek.com/v1".to_string()),
                    credential_env: Some("DEEPSEEK_API_KEY".to_string()),
                    model: Some("deepseek-v4-pro".to_string()),
                    effort: None,
                    wire: Default::default(),
                    enabled: true,
                },
            ],
            ..Default::default()
        };
        HqConfig {
            backends,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn banner_includes_model_provider_context() {
        let cfg = config_with_chain();
        let out = reset_banner("claude-sonnet-5", &cfg).await;
        assert!(out.contains("Session reset! Starting fresh."));
        assert!(out.contains("Model: `claude-sonnet-5`"));
        assert!(out.contains("Provider: copilot"));
        assert!(out.contains("Context:"));
        assert!(out.contains("Fallbacks: copilot → kimi → deepseek"));
        assert!(out.contains("Tip:"));
    }

    #[test]
    fn chain_listing_marks_primary() {
        let cfg = config_with_chain();
        let listing = chain_listing(&cfg);
        assert_eq!(listing.len(), 3);
        assert!(listing[0].is_primary);
        assert_eq!(listing[0].name, "copilot");
        assert!(!listing[1].is_primary);
        assert_eq!(listing[1].model.as_deref(), Some("k3-256k"));
    }

    #[tokio::test]
    async fn chain_primary_model_wins_over_default() {
        let mut cfg = config_with_chain();
        cfg.default_model = "k3-256k".to_string();
        // The chain primary (copilot → claude-sonnet-5) must lead, not the
        // stale legacy default.
        assert_eq!(resolve_session_model(&cfg), "claude-sonnet-5");
        let out = reset_banner(&resolve_session_model(&cfg), &cfg).await;
        assert!(out.contains("Model: `claude-sonnet-5`"));
    }

    #[test]
    fn relay_alias_and_empty_fall_through_to_default_when_no_chain() {
        let mut cfg = HqConfig {
            default_model: "k3-256k".to_string(),
            ..Default::default()
        };
        cfg.relay.model = Some("relay".to_string()); // routing alias, not a model
        assert_eq!(resolve_session_model(&cfg), "k3-256k");
        cfg.relay.model = Some("k3".to_string()); // real override wins
        assert_eq!(resolve_session_model(&cfg), "k3");
    }

    #[tokio::test]
    async fn banner_omits_chain_when_unconfigured() {
        let cfg = HqConfig::default();
        let out = reset_banner("k3-256k", &cfg).await;
        assert!(out.contains("Provider: default"));
        assert!(!out.contains("Fallbacks:"));
    }

    #[test]
    fn context_formatting() {
        assert_eq!(format_context(1_000_000), "1.0M");
        assert_eq!(format_context(272_000), "272K");
        assert_eq!(format_context(200_000), "200K");
        assert_eq!(format_context(8_192), "8K");
    }

    #[test]
    fn resolve_model_arg_matches_backend_name() {
        let cfg = config_with_chain();
        match resolve_model_arg(&cfg.backends, "deepseek") {
            ModelResolution::Backend(name) => assert_eq!(name, "deepseek"),
            ModelResolution::NoMatch(_) => panic!("expected a backend match"),
        }
    }

    #[test]
    fn resolve_model_arg_matches_model_string_case_insensitive() {
        let cfg = config_with_chain();
        match resolve_model_arg(&cfg.backends, "DEEPSEEK-V4-PRO") {
            ModelResolution::Backend(name) => assert_eq!(name, "deepseek"),
            ModelResolution::NoMatch(_) => panic!("expected a backend match"),
        }
    }

    #[test]
    fn resolve_model_arg_reports_no_match() {
        let cfg = config_with_chain();
        match resolve_model_arg(&cfg.backends, "unknown-model") {
            ModelResolution::NoMatch(raw) => assert_eq!(raw, "unknown-model"),
            ModelResolution::Backend(_) => panic!("expected no match"),
        }
    }

    #[test]
    fn render_chain_listing_marks_primary_and_lists_models() {
        let cfg = config_with_chain();
        let out = render_chain_listing(&cfg, "**Available models:**");
        assert!(out.contains("**Available models:**"));
        assert!(out.contains("◆ `copilot` — claude-sonnet-5"));
        assert!(out.contains("◦ `kimi` — k3-256k"));
        assert!(out.contains("◦ `deepseek` — deepseek-v4-pro"));
    }
}
