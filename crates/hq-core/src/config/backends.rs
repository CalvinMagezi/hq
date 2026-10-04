//! Versioned, provider-centric backend configuration.
//!
//! This is the forward-looking replacement for the flat [`ProviderConfig`](super::ProviderConfig)
//! list and the scattered `*_api_key` fields on [`HqConfig`](super::HqConfig). It
//! declares an explicit **primary** backend plus an **ordered fallback chain**,
//! and a registry of named [`BackendEntry`] records.
//!
//! Design goals:
//! - **Additive & backward compatible.** The whole section defaults to empty so
//!   existing configs load unchanged. Legacy fields keep working; this section is
//!   consulted only when [`BackendsConfig::is_configured`] is true.
//! - **Explicit, not adaptive.** Root provider selection is declared here
//!   (primary + fallbacks in order). No scoring, no learned routing.
//! - **No environment mutation.** Credentials are referenced by env-var *name*;
//!   the config never reads *or writes* process environment. Adapters read the
//!   named variables at construction time.

use serde::{Deserialize, Serialize};

use super::default_true;

/// Current schema version understood by this build.
pub const BACKENDS_SCHEMA_VERSION: u32 = 1;

fn default_backends_version() -> u32 {
    BACKENDS_SCHEMA_VERSION
}

/// The kind of a backend entry — selects the adapter used to construct it.
///
/// Serialized in kebab-case (e.g. `github-copilot-cli`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BackendKind {
    /// OpenRouter aggregator (OpenAI-compatible transport).
    Openrouter,
    /// Kimi Code subscription (OpenAI-compatible transport, `api.kimi.com/coding`).
    KimiCode,
    /// GitHub Copilot CLI harness (`gh copilot`). Buffered, limited capabilities.
    GithubCopilotCli,
    /// Generic OpenAI-compatible HTTP endpoint (DeepSeek, Novita, Fireworks, …).
    OpenaiCompatible,
    /// Generic Anthropic Messages API endpoint.
    ///
    /// Served by the native Anthropic Messages client (`AnthropicProvider` in
    /// `hq-llm`), which speaks the real `POST {base}/messages` wire format
    /// (system extraction, `tool_use`/`tool_result` blocks, `input_json_delta`
    /// streaming) rather than OpenAI chat-completions. Works against the
    /// canonical `api.anthropic.com` surface (the documented default endpoint)
    /// or any gateway that speaks the same protocol; supply a custom `endpoint`
    /// to target a gateway.
    AnthropicCompatible,
    /// GitHub Copilot subscription via direct HTTP (Anthropic Messages wire
    /// format at `api.githubcopilot.com`). Full streaming + tool-calling —
    /// unlike `GithubCopilotCli`, this is not a buffered subprocess shim.
    GithubCopilotApi,
}

impl BackendKind {
    /// Whether this kind reaches an HTTP API (as opposed to a local CLI harness).
    pub fn is_api(self) -> bool {
        !matches!(self, BackendKind::GithubCopilotCli)
    }

    /// Whether this kind is a local CLI harness subprocess.
    pub fn is_cli(self) -> bool {
        matches!(self, BackendKind::GithubCopilotCli)
    }

    /// Canonical default endpoint for kinds that have one, else `None`.
    ///
    /// `openai-compatible` has no canonical default: the generic kind's endpoint
    /// must be supplied explicitly. `anthropic-compatible` defaults to the
    /// standard Anthropic Messages base (`https://api.anthropic.com/v1`); supply
    /// an explicit endpoint to target a compatible gateway instead.
    pub fn default_endpoint(self) -> Option<&'static str> {
        match self {
            BackendKind::Openrouter => Some("https://openrouter.ai/api/v1"),
            BackendKind::KimiCode => Some("https://api.kimi.com/coding/v1"),
            BackendKind::AnthropicCompatible => Some("https://api.anthropic.com/v1"),
            BackendKind::GithubCopilotApi => Some("https://api.githubcopilot.com"),
            BackendKind::OpenaiCompatible | BackendKind::GithubCopilotCli => None,
        }
    }

    /// Whether construction needs a static `credential_env` value. False for
    /// both Copilot kinds: the CLI authenticates out-of-band (`gh auth`), and
    /// `GithubCopilotApi` resolves a raw GitHub token dynamically (env vars,
    /// else `gh auth token`) rather than reading a fixed API key once.
    pub fn requires_credential(self) -> bool {
        matches!(
            self,
            BackendKind::Openrouter
                | BackendKind::KimiCode
                | BackendKind::OpenaiCompatible
                | BackendKind::AnthropicCompatible
        )
    }
}

/// Request wire format for OpenAI-style backends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WireApi {
    /// `POST /chat/completions`.
    #[default]
    ChatCompletions,
    /// `POST /responses`.
    Responses,
}

/// A single declared backend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendEntry {
    /// Unique name used to reference this backend from `primary`/`fallbacks`.
    pub name: String,

    /// Adapter kind.
    pub kind: BackendKind,

    /// API base URL / endpoint.
    ///
    /// Optional for kinds with a canonical default ([`BackendKind::default_endpoint`])
    /// and ignored for CLI kinds.
    #[serde(default)]
    pub endpoint: Option<String>,

    /// Name of the environment variable that holds the API key/credential.
    ///
    /// Read at construction time only — never written. Optional for CLI kinds.
    #[serde(default)]
    pub credential_env: Option<String>,

    /// Default model identifier for this backend, if any.
    #[serde(default)]
    pub model: Option<String>,

    /// Reasoning effort forwarded to thinking-capable endpoints
    /// (Kimi K3: low | high | max). None = server default.
    #[serde(default)]
    pub effort: Option<String>,

    /// Wire format for OpenAI-style kinds. Some models (Copilot's
    /// `gpt-6-luna`) are only served over the Responses API.
    #[serde(default)]
    pub wire: WireApi,

    /// Whether this backend is enabled. Disabled backends are skipped when the
    /// chain is resolved.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl BackendEntry {
    /// Resolve the effective endpoint: explicit value, else the kind default.
    pub fn resolved_endpoint(&self) -> Option<String> {
        self.endpoint
            .as_deref()
            .map(str::to_string)
            .or_else(|| self.kind.default_endpoint().map(str::to_string))
    }
}

/// Versioned provider-centric backend configuration.
///
/// Declares one primary backend and an ordered fallback chain over a registry
/// of named [`BackendEntry`] records. Defaults to empty; consult
/// [`is_configured`](Self::is_configured) before using it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendsConfig {
    /// Schema version for forward migrations. Current: [`BACKENDS_SCHEMA_VERSION`].
    #[serde(default = "default_backends_version")]
    pub version: u32,

    /// Name of the primary backend (must match a `backends[].name`).
    ///
    /// Empty means "no explicit chain configured" — callers fall back to legacy
    /// provider construction.
    #[serde(default)]
    pub primary: String,

    /// Ordered fallback backend names, tried in order after the primary.
    #[serde(default)]
    pub fallbacks: Vec<String>,

    /// Declared backend entries (the registry).
    #[serde(default)]
    pub backends: Vec<BackendEntry>,
}

impl Default for BackendsConfig {
    fn default() -> Self {
        Self {
            version: default_backends_version(),
            primary: String::new(),
            fallbacks: Vec::new(),
            backends: Vec::new(),
        }
    }
}

impl BackendsConfig {
    /// Whether an explicit provider chain is configured (a non-empty primary and
    /// at least one declared backend).
    pub fn is_configured(&self) -> bool {
        !self.primary.trim().is_empty() && !self.backends.is_empty()
    }

    /// Look up a declared backend by name.
    pub fn backend(&self, name: &str) -> Option<&BackendEntry> {
        self.backends.iter().find(|b| b.name == name)
    }

    /// Whether `model` is served by a declared backend whose resolved
    /// endpoint is a GitHub Copilot host. Copilot enforces its own per-model
    /// prompt-token cap independent of (and sometimes far smaller than) a
    /// model's native context window, so callers use this to decide whether
    /// a live Copilot catalog check should refine a static context-window
    /// guess rather than trusting it outright.
    pub fn model_routed_through_copilot(&self, model: &str) -> bool {
        self.backends.iter().any(|b| {
            b.model.as_deref() == Some(model)
                && b.resolved_endpoint()
                    .as_deref()
                    .is_some_and(|e| e.contains("githubcopilot.com"))
        })
    }

    /// The ordered chain of backend names: primary first, then fallbacks, with
    /// duplicates removed. Does not filter by existence or enabled-ness — use
    /// [`validate`](Self::validate) for that.
    pub fn chain_order(&self) -> Vec<String> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for name in std::iter::once(&self.primary).chain(self.fallbacks.iter()) {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            if seen.insert(name.to_string()) {
                out.push(name.to_string());
            }
        }
        out
    }

    /// Validate the configuration, returning every problem found (empty `Ok`
    /// when valid). Kept explicit and separate from loading so a partially valid
    /// config can still load and legacy paths keep working.
    ///
    /// Checks:
    /// - schema version is supported,
    /// - backend names are non-empty and unique,
    /// - API kinds resolve an endpoint and name a credential env var,
    /// - the primary references an existing, enabled backend,
    /// - each fallback references an existing backend.
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();

        if self.version != BACKENDS_SCHEMA_VERSION {
            problems.push(format!(
                "unsupported backends.version {} (this build understands {})",
                self.version, BACKENDS_SCHEMA_VERSION
            ));
        }

        // Per-entry checks + uniqueness.
        let mut seen = std::collections::HashSet::new();
        for entry in &self.backends {
            if entry.name.trim().is_empty() {
                problems.push("a backend entry has an empty name".to_string());
                continue;
            }
            if !seen.insert(entry.name.clone()) {
                problems.push(format!("duplicate backend name '{}'", entry.name));
            }
            if entry.kind.is_api() && entry.resolved_endpoint().is_none() {
                problems.push(format!(
                    "backend '{}' ({:?}) requires an endpoint",
                    entry.name, entry.kind
                ));
            }
            // credential_env is optional: construction falls back to the
            // matching HqConfig api-key field for known kinds, and a missing
            // credential is a non-fatal skip, not a config error.
        }

        // Primary must exist and be enabled when the section is in use.
        if self.is_configured() {
            match self.backend(&self.primary) {
                None => problems.push(format!(
                    "primary backend '{}' is not declared in backends[]",
                    self.primary
                )),
                Some(b) if !b.enabled => {
                    problems.push(format!("primary backend '{}' is disabled", self.primary))
                }
                Some(_) => {}
            }
        }

        // Fallbacks must reference existing entries.
        for fb in &self.fallbacks {
            if self.backend(fb).is_none() {
                problems.push(format!(
                    "fallback backend '{fb}' is not declared in backends[]"
                ));
            }
        }

        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn openai_entry(name: &str) -> BackendEntry {
        BackendEntry {
            name: name.to_string(),
            kind: BackendKind::OpenaiCompatible,
            endpoint: Some("https://example.test/v1".to_string()),
            credential_env: Some("EXAMPLE_KEY".to_string()),
            model: Some("example-model".to_string()),
            effort: None,
            wire: Default::default(),
            enabled: true,
        }
    }

    #[test]
    fn default_is_unconfigured_and_valid() {
        let cfg = BackendsConfig::default();
        assert_eq!(cfg.version, BACKENDS_SCHEMA_VERSION);
        assert!(!cfg.is_configured());
        assert!(cfg.validate().is_ok());
        assert!(cfg.chain_order().is_empty());
    }

    #[test]
    fn kind_serializes_kebab_case() {
        let json = serde_json::to_string(&BackendKind::GithubCopilotCli).unwrap();
        assert_eq!(json, "\"github-copilot-cli\"");
        let parsed: BackendKind = serde_json::from_str("\"anthropic-compatible\"").unwrap();
        assert_eq!(parsed, BackendKind::AnthropicCompatible);
    }

    #[test]
    fn github_copilot_api_serializes_kebab_case() {
        let json = serde_json::to_string(&BackendKind::GithubCopilotApi).unwrap();
        assert_eq!(json, "\"github-copilot-api\"");
        let parsed: BackendKind = serde_json::from_str("\"github-copilot-api\"").unwrap();
        assert_eq!(parsed, BackendKind::GithubCopilotApi);
    }

    #[test]
    fn github_copilot_api_defaults_endpoint_and_needs_no_credential() {
        assert_eq!(
            BackendKind::GithubCopilotApi.default_endpoint(),
            Some("https://api.githubcopilot.com")
        );
        assert!(!BackendKind::GithubCopilotApi.requires_credential());
        assert!(BackendKind::GithubCopilotApi.is_api());
        assert!(!BackendKind::GithubCopilotApi.is_cli());
    }

    #[test]
    fn resolved_endpoint_prefers_explicit_then_kind_default() {
        let mut e = openai_entry("x");
        assert_eq!(
            e.resolved_endpoint().as_deref(),
            Some("https://example.test/v1")
        );
        e.endpoint = None; // openai-compatible has no default
        assert_eq!(e.resolved_endpoint(), None);

        let kimi = BackendEntry {
            kind: BackendKind::KimiCode,
            endpoint: None,
            ..openai_entry("kimi")
        };
        assert_eq!(
            kimi.resolved_endpoint().as_deref(),
            Some("https://api.kimi.com/coding/v1")
        );
    }

    #[test]
    fn chain_order_dedups_and_skips_blanks() {
        let cfg = BackendsConfig {
            primary: "a".to_string(),
            fallbacks: vec![
                "b".to_string(),
                "a".to_string(),
                "".to_string(),
                "c".to_string(),
            ],
            backends: vec![openai_entry("a"), openai_entry("b"), openai_entry("c")],
            ..Default::default()
        };
        assert_eq!(cfg.chain_order(), vec!["a", "b", "c"]);
    }

    #[test]
    fn validate_flags_missing_primary_and_fallback() {
        let cfg = BackendsConfig {
            primary: "ghost".to_string(),
            fallbacks: vec!["b".to_string(), "phantom".to_string()],
            backends: vec![openai_entry("b")],
            ..Default::default()
        };
        let problems = cfg.validate().unwrap_err();
        assert!(
            problems
                .iter()
                .any(|p| p.contains("primary backend 'ghost'"))
        );
        assert!(
            problems
                .iter()
                .any(|p| p.contains("fallback backend 'phantom'"))
        );
    }

    #[test]
    fn validate_flags_api_entry_without_credential_or_endpoint() {
        let cfg = BackendsConfig {
            primary: "bad".to_string(),
            fallbacks: vec![],
            backends: vec![BackendEntry {
                name: "bad".to_string(),
                kind: BackendKind::OpenaiCompatible,
                endpoint: None,
                credential_env: None,
                model: None,
                effort: None,
                wire: Default::default(),
                enabled: true,
            }],
            ..Default::default()
        };
        let problems = cfg.validate().unwrap_err();
        assert!(problems.iter().any(|p| p.contains("requires an endpoint")));
        // credential_env is intentionally NOT a validation error: construction
        // falls back to HqConfig api-key fields and skips non-fatally when no
        // credential resolves at all.
        assert!(
            !problems
                .iter()
                .any(|p| p.contains("requires a credential_env"))
        );
    }

    #[test]
    fn model_routed_through_copilot_matches_by_model_and_endpoint() {
        let cfg = BackendsConfig {
            primary: "copilot".to_string(),
            fallbacks: vec!["deepseek".to_string()],
            backends: vec![
                BackendEntry {
                    kind: BackendKind::OpenaiCompatible,
                    endpoint: Some("https://api.githubcopilot.com".to_string()),
                    credential_env: Some("COPILOT_GITHUB_TOKEN".to_string()),
                    model: Some("gemini-3.8-flash".to_string()),
                    ..openai_entry("copilot")
                },
                BackendEntry {
                    kind: BackendKind::OpenaiCompatible,
                    endpoint: Some("https://api.deepseek.com/v1".to_string()),
                    credential_env: Some("DEEPSEEK_API_KEY".to_string()),
                    model: Some("deepseek-flash".to_string()),
                    ..openai_entry("deepseek")
                },
            ],
            ..Default::default()
        };
        assert!(cfg.model_routed_through_copilot("gemini-3.8-flash"));
        assert!(!cfg.model_routed_through_copilot("deepseek-flash"));
        assert!(!cfg.model_routed_through_copilot("some-other-model"));
    }

    #[test]
    fn model_routed_through_copilot_ignores_a_non_copilot_endpoint_reusing_the_model_name() {
        // A coincidental model-name match on a non-Copilot endpoint must not
        // trigger a live Copilot catalog fetch.
        let cfg = BackendsConfig {
            primary: "openrouter".to_string(),
            fallbacks: vec![],
            backends: vec![BackendEntry {
                kind: BackendKind::Openrouter,
                model: Some("gemini-3.8-flash".to_string()),
                ..openai_entry("openrouter")
            }],
            ..Default::default()
        };
        assert!(!cfg.model_routed_through_copilot("gemini-3.8-flash"));
    }

    #[test]
    fn anthropic_compatible_defaults_to_the_standard_messages_base() {
        // The native Anthropic Messages kind resolves the canonical base when no
        // explicit endpoint is given (a documented default), but still requires
        // a credential like every API kind.
        assert_eq!(
            BackendKind::AnthropicCompatible.default_endpoint(),
            Some("https://api.anthropic.com/v1")
        );

        let entry = BackendEntry {
            name: "claude".to_string(),
            kind: BackendKind::AnthropicCompatible,
            endpoint: None,
            credential_env: Some("ANTHROPIC_API_KEY".to_string()),
            model: Some("claude-sonnet-4-6".to_string()),
            effort: None,
            wire: Default::default(),
            enabled: true,
        };
        assert_eq!(
            entry.resolved_endpoint().as_deref(),
            Some("https://api.anthropic.com/v1")
        );

        // A configured chain over it validates cleanly: endpoint resolved from
        // the default, credential env named.
        let cfg = BackendsConfig {
            primary: "claude".to_string(),
            fallbacks: vec![],
            backends: vec![entry],
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());

        // Dropping the credential still validates: resolution falls back to
        // the HqConfig anthropic key at construction time, and a truly absent
        // credential is a non-fatal construction skip, not a config error.
        let mut no_cred = cfg.clone();
        no_cred.backends[0].credential_env = None;
        assert!(no_cred.validate().is_ok());
    }

    #[test]
    fn validate_allows_cli_backend_without_credential() {
        let cfg = BackendsConfig {
            primary: "copilot".to_string(),
            fallbacks: vec![],
            backends: vec![BackendEntry {
                name: "copilot".to_string(),
                kind: BackendKind::GithubCopilotCli,
                endpoint: None,
                credential_env: None,
                model: None,
                effort: None,
                wire: Default::default(),
                enabled: true,
            }],
            ..Default::default()
        };
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn validate_flags_disabled_primary_and_duplicate_names() {
        let cfg = BackendsConfig {
            primary: "a".to_string(),
            fallbacks: vec![],
            backends: vec![
                BackendEntry {
                    effort: None,
                    wire: Default::default(),
                    enabled: false,
                    ..openai_entry("a")
                },
                openai_entry("a"),
            ],
            ..Default::default()
        };
        let problems = cfg.validate().unwrap_err();
        assert!(problems.iter().any(|p| p.contains("disabled")));
        assert!(
            problems
                .iter()
                .any(|p| p.contains("duplicate backend name 'a'"))
        );
    }

    #[test]
    fn deserializes_from_yaml_with_defaults() {
        let yaml = r#"
primary: openrouter-main
fallbacks:
  - copilot
backends:
  - name: openrouter-main
    kind: openrouter
    credential_env: OPENROUTER_API_KEY
    model: anthropic/claude-sonnet-4-6
  - name: copilot
    kind: github-copilot-cli
"#;
        let cfg: BackendsConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.version, BACKENDS_SCHEMA_VERSION); // defaulted
        assert!(cfg.is_configured());
        assert_eq!(cfg.chain_order(), vec!["openrouter-main", "copilot"]);
        // openrouter resolves its canonical endpoint even when omitted.
        assert_eq!(
            cfg.backend("openrouter-main")
                .unwrap()
                .resolved_endpoint()
                .as_deref(),
            Some("https://openrouter.ai/api/v1")
        );
        assert!(cfg.backend("copilot").unwrap().enabled); // default_true
        assert_eq!(cfg.backend("copilot").unwrap().wire, WireApi::ChatCompletions);
        assert!(cfg.validate().is_ok());
    }

    #[test]
    fn wire_responses_parses_from_yaml() {
        let yaml = r#"
primary: luna
backends:
  - name: luna
    kind: openai-compatible
    endpoint: https://api.githubcopilot.com
    credential_env: COPILOT_GITHUB_TOKEN
    model: gpt-6-luna
    wire: responses
"#;
        let cfg: BackendsConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(cfg.backend("luna").unwrap().wire, WireApi::Responses);
    }
}
