//! [`BackendRegistry`] — construct backends and the [`ProviderChain`] from config.
//!
//! The registry reads the versioned [`BackendsConfig`](hq_core::config::BackendsConfig):
//! it validates it, constructs each enabled [`BackendEntry`](hq_core::config::BackendEntry)
//! into a [`SessionBackend`], then resolves the explicit primary + ordered
//! fallbacks into a [`ProviderChain`].
//!
//! Reuse, don't rewrite: OpenAI-compatible kinds (OpenRouter, Kimi Code, and
//! the generic `openai-compatible`) wrap the existing
//! [`OpenRouterProvider`](hq_llm::openai_compat::OpenRouterProvider) client;
//! `anthropic-compatible` wraps the native
//! [`AnthropicProvider`](hq_llm::anthropic::AnthropicProvider) Messages client;
//! `github-copilot-api` wraps the native
//! [`CopilotProvider`](hq_llm::copilot::CopilotProvider), which speaks the
//! same Messages format directly against the Copilot subscription; the
//! GitHub Copilot CLI kind reuses [`CopilotCliBackend`]. Every API provider
//! shares the one HTTP client, so no new connection pool is introduced here.
//!
//! Credentials are read from the named environment variables **only** — the
//! registry never writes to the process environment. An API backend whose
//! credential variable is unset is skipped (recorded in [`BackendRegistry::skipped`])
//! rather than erroring the whole build; a chain still forms from what's
//! available. If the *primary* is unavailable, [`build_chain`](BackendRegistry::build_chain)
//! fails loudly.

use std::sync::Arc;

#[cfg(test)]
use hq_core::config::WireApi;
use hq_core::config::{BackendEntry, BackendKind, BackendsConfig, GitHubCopilotConfig, HqConfig};
#[cfg(test)]
use hq_llm::anthropic::AnthropicProvider;
#[cfg(test)]
use hq_llm::provider::LlmProvider;

use super::{ApiBackend, CopilotCliBackend, ProviderChain, SessionBackend};

/// Errors from building a [`BackendRegistry`] or its [`ProviderChain`].
#[derive(Debug, thiserror::Error)]
pub enum BackendRegistryError {
    /// The [`BackendsConfig`] failed validation.
    #[error("backends config is invalid: {}", .0.join("; "))]
    Invalid(Vec<String>),

    /// No explicit backend chain is configured (empty `primary`/`backends`).
    #[error("no explicit backend chain configured")]
    NotConfigured,

    /// The declared primary backend could not be constructed.
    #[error("primary backend '{name}' is unavailable: {reason}")]
    PrimaryUnavailable { name: String, reason: String },

    /// The chain resolved to zero usable backends.
    #[error("backend chain is empty after resolution")]
    EmptyChain,
}

/// A registry of constructed backends plus the resolved chain order.
pub struct BackendRegistry {
    /// Constructed, enabled backends, keyed by declared name (declaration order).
    entries: Vec<(String, Arc<dyn SessionBackend>)>,
    /// The chain order (primary first, then fallbacks) filtered to constructed
    /// backends.
    chain_order: Vec<String>,
    /// The declared primary name (may be absent from `entries` if unavailable).
    primary: String,
    /// Backends that were declared+enabled but could not be constructed, with a
    /// human-readable reason. Surfaced for diagnostics.
    skipped: Vec<(String, String)>,
}

impl std::fmt::Debug for BackendRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BackendRegistry")
            .field("backends", &self.names())
            .field("chain_order", &self.chain_order)
            .field("primary", &self.primary)
            .field("skipped", &self.skipped)
            .finish()
    }
}

impl BackendRegistry {
    /// Build from a full [`HqConfig`], reading the `backends` section and the
    /// `github_copilot` harness settings.
    pub fn from_config(config: &HqConfig) -> Result<Self, BackendRegistryError> {
        Self::from_parts(
            &config.backends,
            &config.github_copilot,
            &ConfigCredentials::from_config(config),
        )
    }

    /// Build from the [`BackendsConfig`] plus the CLI harness settings it needs.
    pub fn from_parts(
        cfg: &BackendsConfig,
        github_copilot: &GitHubCopilotConfig,
        credentials: &ConfigCredentials,
    ) -> Result<Self, BackendRegistryError> {
        if !cfg.is_configured() {
            return Err(BackendRegistryError::NotConfigured);
        }
        cfg.validate().map_err(BackendRegistryError::Invalid)?;

        let mut entries: Vec<(String, Arc<dyn SessionBackend>)> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();

        for entry in cfg.backends.iter().filter(|b| b.enabled) {
            match construct_backend(entry, github_copilot, credentials) {
                Ok(Some(backend)) => entries.push((entry.name.clone(), backend)),
                Ok(None) => skipped.push((
                    entry.name.clone(),
                    format!(
                        "no credential: env '{}' unset and no config-key fallback for {:?}",
                        entry.credential_env.as_deref().unwrap_or("<none>"),
                        entry.kind,
                    ),
                )),
                Err(reason) => skipped.push((entry.name.clone(), reason)),
            }
        }

        let available: std::collections::HashSet<&str> =
            entries.iter().map(|(n, _)| n.as_str()).collect();
        let chain_order: Vec<String> = cfg
            .chain_order()
            .into_iter()
            .filter(|n| available.contains(n.as_str()))
            .collect();

        Ok(Self {
            entries,
            chain_order,
            primary: cfg.primary.clone(),
            skipped,
        })
    }

    /// Look up a constructed backend by declared name.
    pub fn get(&self, name: &str) -> Option<Arc<dyn SessionBackend>> {
        self.entries
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, b)| b.clone())
    }

    /// The declared primary backend's name.
    pub fn primary_name(&self) -> &str {
        &self.primary
    }

    /// The backend named `arg`, else the one whose pinned model equals it.
    pub fn resolve_model(&self, arg: &str) -> Option<(String, Arc<dyn SessionBackend>)> {
        let by_name = self.entries.iter().find(|(n, _)| n == arg);
        let by_model = || {
            self.entries
                .iter()
                .find(|(_, b)| b.pinned_model().as_deref() == Some(arg))
        };
        by_name
            .or_else(by_model)
            .map(|(n, b)| (n.clone(), b.clone()))
    }

    /// `backend (model)` pairs for error messages.
    pub fn model_listing(&self) -> String {
        let pairs: Vec<String> = self
            .entries
            .iter()
            .filter_map(|(n, b)| Some(format!("{n} ({})", b.pinned_model()?)))
            .collect();
        if pairs.is_empty() {
            "none declared".to_string()
        } else {
            pairs.join(", ")
        }
    }

    /// The best default external backend to auto-select for a child that does
    /// not need local tools: the declared primary when it constructed, else the
    /// first available backend in declaration order. `None` when nothing
    /// resolved.
    pub fn default_external(&self) -> Option<(String, Arc<dyn SessionBackend>)> {
        if let Some(backend) = self.get(&self.primary) {
            return Some((self.primary.clone(), backend));
        }
        self.entries
            .first()
            .map(|(name, backend)| (name.clone(), backend.clone()))
    }

    /// Construct a registry directly from already-built backends, bypassing the
    /// config-driven construction path. Chain order follows entry order and the
    /// caller names the default/primary. Intended for the agent service and
    /// tests that inject backends without a full [`HqConfig`].
    pub fn from_backends(
        entries: Vec<(String, Arc<dyn SessionBackend>)>,
        primary: impl Into<String>,
    ) -> Self {
        let chain_order = entries.iter().map(|(n, _)| n.clone()).collect();
        Self {
            entries,
            chain_order,
            primary: primary.into(),
            skipped: Vec::new(),
        }
    }

    /// Names of the constructed backends (declaration order).
    pub fn names(&self) -> Vec<&str> {
        self.entries.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// Declared+enabled backends that could not be constructed, with reasons.
    pub fn skipped(&self) -> &[(String, String)] {
        &self.skipped
    }

    /// Resolve the explicit primary + ordered fallbacks into a [`ProviderChain`].
    ///
    /// Fails if the declared primary is unavailable, or if no backend resolved.
    pub fn build_chain(&self) -> Result<ProviderChain, BackendRegistryError> {
        if self.get(&self.primary).is_none() {
            let reason = self
                .skipped
                .iter()
                .find(|(n, _)| *n == self.primary)
                .map(|(_, r)| r.clone())
                .unwrap_or_else(|| "not declared or disabled".to_string());
            return Err(BackendRegistryError::PrimaryUnavailable {
                name: self.primary.clone(),
                reason,
            });
        }

        let backends: Vec<Arc<dyn SessionBackend>> = self
            .chain_order
            .iter()
            .filter_map(|n| self.get(n))
            .collect();

        if backends.is_empty() {
            return Err(BackendRegistryError::EmptyChain);
        }

        Ok(ProviderChain::new("provider-chain", backends))
    }
}

pub use hq_llm::backend_chain::ConfigCredentials;

/// Wrap the OpenAI-compatible provider as an [`ApiBackend`].
#[cfg(test)]
pub(crate) fn construct_api_backend(
    name: &str,
    endpoint: &str,
    api_key: &str,
    model: Option<String>,
    effort: Option<String>,
    wire: WireApi,
) -> ApiBackend {
    let provider = hq_llm::backend_chain::openai_compat_provider(endpoint, api_key, effort, wire);
    ApiBackend::new(name.to_string(), provider).with_model(model)
}

/// Wrap the native [`AnthropicProvider`] (Anthropic Messages API) as an
/// [`ApiBackend`]. Used for [`BackendKind::AnthropicCompatible`], which speaks
/// the real Messages wire format rather than OpenAI chat-completions.
#[cfg(test)]
pub(crate) fn construct_anthropic_backend(
    name: &str,
    endpoint: &str,
    api_key: &str,
    model: Option<String>,
) -> ApiBackend {
    let provider =
        Arc::new(AnthropicProvider::new_with_base(api_key, endpoint)) as Arc<dyn LlmProvider>;
    ApiBackend::new(name.to_string(), provider).with_model(model)
}

/// Construct one backend from its config entry.
///
/// - `Ok(Some(_))` — constructed.
/// - `Ok(None)` — a required credential is missing (skip, not fatal).
/// - `Err(reason)` — a construction problem that validation didn't catch.
fn construct_backend(
    entry: &BackendEntry,
    github_copilot: &GitHubCopilotConfig,
    credentials: &ConfigCredentials,
) -> Result<Option<Arc<dyn SessionBackend>>, String> {
    match entry.kind {
        BackendKind::GithubCopilotCli => {
            // CLI harness authenticates out-of-band; always constructible. The
            // entry's model overrides the harness default when set.
            let mut gh = github_copilot.clone();
            if entry.model.is_some() {
                gh.model = entry.model.clone();
            }
            let backend = CopilotCliBackend::from_config(&gh).labelled(entry.name.clone());
            Ok(Some(Arc::new(backend) as Arc<dyn SessionBackend>))
        }
        _ => {
            let Some(provider) = hq_llm::backend_chain::api_provider(entry, credentials)? else {
                return Ok(None);
            };
            let backend =
                ApiBackend::new(entry.name.clone(), provider).with_model(entry.model.clone());
            Ok(Some(Arc::new(backend) as Arc<dyn SessionBackend>))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::BackendKind;

    fn cli_entry(name: &str) -> BackendEntry {
        BackendEntry {
            name: name.to_string(),
            kind: BackendKind::GithubCopilotCli,
            endpoint: None,
            credential_env: None,
            model: None,
            effort: None,
            wire: Default::default(),
            enabled: true,
        }
    }

    fn api_entry(name: &str, cred_env: &str) -> BackendEntry {
        BackendEntry {
            name: name.to_string(),
            kind: BackendKind::OpenaiCompatible,
            endpoint: Some("https://example.test/v1".to_string()),
            credential_env: Some(cred_env.to_string()),
            model: Some("m".to_string()),
            effort: None,
            wire: Default::default(),
            enabled: true,
        }
    }

    fn config_with(backends: BackendsConfig) -> HqConfig {
        HqConfig {
            backends,
            ..HqConfig::default()
        }
    }

    #[test]
    fn unconfigured_reports_not_configured() {
        let err = BackendRegistry::from_config(&HqConfig::default()).unwrap_err();
        assert!(matches!(err, BackendRegistryError::NotConfigured));
    }

    #[test]
    fn invalid_config_reports_invalid() {
        let cfg = BackendsConfig {
            primary: "ghost".to_string(),
            fallbacks: vec![],
            backends: vec![cli_entry("real")],
            ..Default::default()
        };
        let err = BackendRegistry::from_config(&config_with(cfg)).unwrap_err();
        assert!(matches!(err, BackendRegistryError::Invalid(_)));
    }

    #[test]
    fn cli_primary_builds_a_single_backend_chain() {
        let cfg = BackendsConfig {
            primary: "copilot".to_string(),
            fallbacks: vec![],
            backends: vec![cli_entry("copilot")],
            ..Default::default()
        };
        let registry = BackendRegistry::from_config(&config_with(cfg)).unwrap();
        assert_eq!(registry.names(), vec!["copilot"]);
        let chain = registry.build_chain().unwrap();
        assert_eq!(chain.len(), 1);
    }

    #[test]
    fn api_fallback_without_key_is_skipped_but_chain_still_builds() {
        // Use an env var guaranteed to be unset.
        let cfg = BackendsConfig {
            primary: "copilot".to_string(),
            fallbacks: vec!["api".to_string()],
            backends: vec![
                cli_entry("copilot"),
                api_entry("api", "HQ_TEST_DEFINITELY_UNSET_KEY_9Z"),
            ],
            ..Default::default()
        };
        let registry = BackendRegistry::from_config(&config_with(cfg)).unwrap();
        assert_eq!(registry.names(), vec!["copilot"]);
        assert!(registry.skipped().iter().any(|(n, _)| n == "api"));
        let chain = registry.build_chain().unwrap();
        assert_eq!(chain.len(), 1); // only the CLI primary survived
    }

    #[test]
    fn api_primary_without_key_reports_primary_unavailable() {
        let cfg = BackendsConfig {
            primary: "api".to_string(),
            fallbacks: vec![],
            backends: vec![api_entry("api", "HQ_TEST_DEFINITELY_UNSET_KEY_9Z")],
            ..Default::default()
        };
        let registry = BackendRegistry::from_config(&config_with(cfg)).unwrap();
        let err = registry.build_chain().unwrap_err();
        match err {
            BackendRegistryError::PrimaryUnavailable { name, .. } => assert_eq!(name, "api"),
            other => panic!("expected PrimaryUnavailable, got {other:?}"),
        }
    }

    #[test]
    fn construct_api_backend_yields_full_api_capabilities() {
        let backend = construct_api_backend(
            "named",
            "https://example.test/v1",
            "dummy-key",
            Some("configured-model".to_string()),
            None,
            WireApi::default(),
        );
        assert_eq!(backend.name(), "named");
        assert!(backend.capabilities().streaming);
        assert!(backend.capabilities().tools);
        assert_eq!(backend.model_override(), Some("configured-model"));
    }

    #[test]
    fn anthropic_compatible_constructs_native_messages_provider() {
        // The anthropic-compatible kind must wrap the native Anthropic Messages
        // provider (not the OpenAI-compatible OpenRouterProvider), while still
        // honoring the per-entry model override.
        let backend = construct_anthropic_backend(
            "claude",
            "https://api.anthropic.com/v1",
            "dummy-key",
            Some("claude-sonnet-4-6".to_string()),
        );
        assert_eq!(backend.name(), "claude");
        assert_eq!(backend.provider().name(), "anthropic");
        assert!(backend.capabilities().streaming);
        assert!(backend.capabilities().tools);
        assert_eq!(backend.model_override(), Some("claude-sonnet-4-6"));
    }

    #[test]
    fn openai_compatible_kinds_stay_on_the_openrouter_provider() {
        let backend = construct_api_backend(
            "gw",
            "https://example.test/v1",
            "dummy-key",
            None,
            None,
            WireApi::default(),
        );
        assert_eq!(backend.provider().name(), "openrouter");
        assert_eq!(backend.model_override(), None);
    }

    #[test]
    fn cli_entry_model_overrides_harness_default() {
        let mut entry = cli_entry("copilot");
        entry.model = Some("claude-sonnet-4.6".to_string());
        let backend = construct_backend(
            &entry,
            &GitHubCopilotConfig::default(),
            &ConfigCredentials::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(backend.name(), "copilot");
    }

    fn api_copilot_entry(name: &str) -> BackendEntry {
        BackendEntry {
            name: name.to_string(),
            kind: BackendKind::GithubCopilotApi,
            endpoint: None,
            credential_env: None,
            model: Some("claude-haiku-4.5".to_string()),
            effort: None,
            wire: Default::default(),
            enabled: true,
        }
    }

    #[test]
    fn github_copilot_api_constructs_without_a_credential() {
        let backend = construct_backend(
            &api_copilot_entry("copilot"),
            &GitHubCopilotConfig::default(),
            &ConfigCredentials::default(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(backend.name(), "copilot");
        assert!(backend.capabilities().streaming);
        assert!(backend.capabilities().tools);
    }

    #[test]
    fn github_copilot_api_primary_builds_a_full_capability_chain() {
        // This is the regression test for the bug this feature exists to fix:
        // a CLI-based Copilot primary silently strips tool-calling from every
        // chat turn. An API-based primary must not.
        let cfg = BackendsConfig {
            primary: "copilot".to_string(),
            fallbacks: vec![],
            backends: vec![api_copilot_entry("copilot")],
            ..Default::default()
        };
        let registry = BackendRegistry::from_config(&config_with(cfg)).unwrap();
        assert_eq!(registry.names(), vec!["copilot"]);
        let chain = registry.build_chain().unwrap();
        assert_eq!(chain.len(), 1);
        assert!(chain.root_capabilities().tools);
        assert!(chain.root_capabilities().streaming);
    }

    #[test]
    fn disabled_backend_is_not_constructed() {
        let cfg = BackendsConfig {
            primary: "copilot".to_string(),
            fallbacks: vec!["off".to_string()],
            backends: vec![
                cli_entry("copilot"),
                BackendEntry {
                    effort: None,
                    wire: Default::default(),
                    enabled: false,
                    ..cli_entry("off")
                },
            ],
            ..Default::default()
        };
        let registry = BackendRegistry::from_config(&config_with(cfg)).unwrap();
        assert_eq!(registry.names(), vec!["copilot"]);
        assert!(registry.get("off").is_none());
    }
}
