//! Providers for `backends:` config entries, and an ordered chain over them.
//!
//! HQ's own sessions drive the chain through `hq-agent`'s backend registry.
//! Background work (agent worker triage, memory summaries, reviews) goes
//! through [`crate::router::LlmRouter`]; when `backends:` is configured the
//! router sends every alias to a [`ChainProvider`] so both follow the same
//! explicit primary and fallbacks.

use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use hq_core::config::{BackendEntry, BackendKind, HqConfig, WireApi};
use tokio_stream::Stream;

use crate::anthropic::AnthropicProvider;
use crate::copilot::CopilotProvider;
use crate::openai_compat::OpenRouterProvider;
use crate::provider::{ChatRequest, ChatResponse, LlmError, LlmProvider, StreamChunk};

/// API keys from `HqConfig`, used when an entry's `credential_env` is unset.
#[derive(Debug, Clone, Default)]
pub struct ConfigCredentials {
    pub kimi_code: Option<String>,
    pub openrouter: Option<String>,
    pub anthropic: Option<String>,
    pub openai: Option<String>,
}

impl ConfigCredentials {
    pub fn from_config(config: &HqConfig) -> Self {
        let clean = |v: &Option<String>| {
            v.as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        Self {
            kimi_code: clean(&config.kimi_code_api_key),
            openrouter: clean(&config.openrouter_api_key),
            anthropic: clean(&config.anthropic_api_key),
            openai: clean(&config.openai_api_key),
        }
    }
}

fn is_local_endpoint(endpoint: &str) -> bool {
    endpoint.contains("://localhost") || endpoint.contains("://127.0.0.1")
}

/// Resolve an entry's credential: the named environment variable wins when
/// set; otherwise the matching `HqConfig` key for the kind. Local
/// OpenAI-compatible endpoints (Ollama and friends) accept any token, so they
/// get a placeholder. Never writes to the environment.
pub fn resolve_credential(entry: &BackendEntry, creds: &ConfigCredentials) -> Option<String> {
    let from_env = entry
        .credential_env
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .and_then(|name| std::env::var(name).ok())
        .filter(|v| !v.trim().is_empty());
    if from_env.is_some() {
        return from_env;
    }
    let fallback = match entry.kind {
        BackendKind::KimiCode => creds.kimi_code.clone(),
        BackendKind::Openrouter => creds.openrouter.clone(),
        BackendKind::AnthropicCompatible => creds.anthropic.clone(),
        BackendKind::OpenaiCompatible => creds.openai.clone(),
        BackendKind::GithubCopilotCli | BackendKind::GithubCopilotApi => None,
    };
    if fallback.is_some() {
        return fallback;
    }
    let local = entry.kind == BackendKind::OpenaiCompatible
        && entry
            .resolved_endpoint()
            .as_deref()
            .is_some_and(is_local_endpoint);
    local.then(|| "local".to_string())
}

/// The OpenAI-style HTTP provider shared by every OpenAI-compatible kind.
pub fn openai_compat_provider(
    endpoint: &str,
    api_key: &str,
    effort: Option<String>,
    wire: WireApi,
) -> Arc<dyn LlmProvider> {
    Arc::new(
        OpenRouterProvider::new_with_base(api_key, endpoint)
            .with_thinking_effort(effort)
            .with_responses_api(wire == WireApi::Responses),
    )
}

/// Build the HTTP provider for an API-kind entry.
///
/// `Ok(None)`: a CLI kind, or a required credential is missing (skip, not
/// fatal). `Err`: a construction problem such as a missing endpoint.
pub fn api_provider(
    entry: &BackendEntry,
    creds: &ConfigCredentials,
) -> Result<Option<Arc<dyn LlmProvider>>, String> {
    let endpoint = || {
        entry
            .resolved_endpoint()
            .ok_or_else(|| format!("backend '{}' has no endpoint", entry.name))
    };
    match entry.kind {
        BackendKind::GithubCopilotCli => Ok(None),
        // Auth resolves dynamically (env vars, else `gh auth token`).
        BackendKind::GithubCopilotApi => Ok(Some(Arc::new(CopilotProvider::new()))),
        BackendKind::Openrouter | BackendKind::KimiCode | BackendKind::OpenaiCompatible => {
            let endpoint = endpoint()?;
            Ok(resolve_credential(entry, creds).map(|key| {
                openai_compat_provider(&endpoint, &key, entry.effort.clone(), entry.wire)
            }))
        }
        BackendKind::AnthropicCompatible => {
            let endpoint = endpoint()?;
            Ok(resolve_credential(entry, creds).map(|key| {
                Arc::new(AnthropicProvider::new_with_base(&key, &endpoint)) as Arc<dyn LlmProvider>
            }))
        }
    }
}

struct Link {
    name: String,
    provider: Arc<dyn LlmProvider>,
    /// `None` passes the caller's model through.
    model: Option<String>,
}

/// Tries each configured backend in order until one answers. A link with a
/// configured model sends it, whatever alias the caller asked for.
pub struct ChainProvider {
    links: Vec<Link>,
}

/// [`LlmError::fails_over`], except that an unclassified error also moves on:
/// a background call has no user waiting on the precise failure.
fn should_fail_over(err: &anyhow::Error) -> bool {
    match err.downcast_ref::<LlmError>() {
        None | Some(LlmError::Other(_)) => true,
        Some(llm) => llm.fails_over(),
    }
}

impl ChainProvider {
    /// The chain from `backends:`, primary first. `None` when no backend is
    /// usable. Entries without a model are skipped: an alias like "fast" is
    /// not a model any backend serves.
    pub fn from_config(config: &HqConfig) -> Option<Self> {
        let creds = ConfigCredentials::from_config(config);
        let mut links = Vec::new();
        for name in config.backends.chain_order() {
            let Some(entry) = config.backends.backend(&name).filter(|e| e.enabled) else {
                continue;
            };
            let Some(model) = entry.model.clone().filter(|m| !m.trim().is_empty()) else {
                tracing::warn!(backend = %name, "backend chain: no model set, skipped for background calls");
                continue;
            };
            match api_provider(entry, &creds) {
                Ok(Some(provider)) => links.push(Link {
                    name,
                    provider,
                    model: Some(model),
                }),
                Ok(None) => {}
                Err(e) => tracing::warn!(backend = %name, "backend chain: {e}"),
            }
        }
        (!links.is_empty()).then_some(Self { links })
    }

    /// A chain over providers that already choose their own model.
    pub fn from_providers(providers: Vec<(String, Arc<dyn LlmProvider>)>) -> Self {
        let links = providers
            .into_iter()
            .map(|(name, provider)| Link {
                name,
                provider,
                model: None,
            })
            .collect();
        Self { links }
    }

    #[cfg(test)]
    fn from_links(links: Vec<(&str, Arc<dyn LlmProvider>, &str)>) -> Self {
        Self {
            links: links
                .into_iter()
                .map(|(name, provider, model)| Link {
                    name: name.to_string(),
                    provider,
                    model: Some(model.to_string()),
                })
                .collect(),
        }
    }

    pub fn backend_names(&self) -> Vec<&str> {
        self.links.iter().map(|l| l.name.as_str()).collect()
    }
}

#[async_trait]
impl LlmProvider for ChainProvider {
    fn name(&self) -> &str {
        "backends"
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let mut last_err = None;
        for link in &self.links {
            let request = ChatRequest {
                model: link.model.clone().unwrap_or_else(|| request.model.clone()),
                ..request.clone()
            };
            match link.provider.chat(&request).await {
                Ok(response) => return Ok(response),
                Err(e) if should_fail_over(&e) => {
                    tracing::warn!(backend = %link.name, error = %e, "backend chain: trying next backend");
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("backend chain is empty")))
    }

    async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
        let mut last_err = None;
        for link in &self.links {
            let request = ChatRequest {
                model: link.model.clone().unwrap_or_else(|| request.model.clone()),
                ..request.clone()
            };
            match link.provider.chat_stream(&request).await {
                Ok(stream) => return Ok(stream),
                Err(e) if should_fail_over(&e) => {
                    tracing::warn!(backend = %link.name, error = %e, "backend chain: trying next backend");
                    last_err = Some(e);
                }
                Err(e) => return Err(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("backend chain is empty")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::types::{ChatMessage, MessageRole};
    use std::sync::Mutex;

    /// Records the model it was asked for; fails when `error` is set.
    struct Fake {
        seen: Mutex<Vec<String>>,
        error: Option<fn() -> LlmError>,
    }

    impl Fake {
        fn new(error: Option<fn() -> LlmError>) -> Arc<Self> {
            Arc::new(Self {
                seen: Mutex::new(Vec::new()),
                error,
            })
        }
    }

    #[async_trait]
    impl LlmProvider for Fake {
        fn name(&self) -> &str {
            "fake"
        }

        async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse> {
            self.seen.lock().unwrap().push(request.model.clone());
            if let Some(make) = self.error {
                return Err(make().into());
            }
            Ok(ChatResponse {
                message: ChatMessage {
                    role: MessageRole::Assistant,
                    content: "ok".into(),
                    tool_calls: Vec::new(),
                    tool_call_id: None,
                    reasoning_content: None,
                    image_parts: Vec::new(),
                },
                input_tokens: 1,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                model: request.model.clone(),
            })
        }

        async fn chat_stream(
            &self,
            _request: &ChatRequest,
        ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamChunk>> + Send>>> {
            unimplemented!("not used by these tests")
        }
    }

    fn request(model: &str) -> ChatRequest {
        ChatRequest {
            model: model.into(),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn primary_answers_with_its_own_model() {
        let (luna, gemini) = (Fake::new(None), Fake::new(None));
        let chain = ChainProvider::from_links(vec![
            ("luna", luna.clone(), "gpt-6-luna"),
            ("copilot", gemini.clone(), "gemini-3.8-flash"),
        ]);
        let reply = chain.chat(&request("fast")).await.unwrap();
        assert_eq!(reply.model, "gpt-6-luna");
        assert!(gemini.seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn outage_moves_to_the_next_backend_in_order() {
        let luna = Fake::new(Some(|| LlmError::Auth {
            status: 400,
            message: "model_not_supported".into(),
        }));
        let gemini = Fake::new(None);
        let chain = ChainProvider::from_links(vec![
            ("luna", luna.clone(), "gpt-6-luna"),
            ("copilot", gemini.clone(), "gemini-3.8-flash"),
        ]);
        let reply = chain.chat(&request("bulk")).await.unwrap();
        assert_eq!(reply.model, "gemini-3.8-flash");
        assert_eq!(*luna.seen.lock().unwrap(), vec!["gpt-6-luna"]);
    }

    #[tokio::test]
    async fn context_overflow_does_not_fail_over() {
        let luna = Fake::new(Some(|| LlmError::ContextOverflow {
            message: "too long".into(),
        }));
        let gemini = Fake::new(None);
        let chain = ChainProvider::from_links(vec![
            ("luna", luna, "gpt-6-luna"),
            ("copilot", gemini.clone(), "gemini-3.8-flash"),
        ]);
        assert!(chain.chat(&request("fast")).await.is_err());
        assert!(gemini.seen.lock().unwrap().is_empty());
    }

    #[test]
    fn unclassified_errors_fail_over_but_overflow_does_not() {
        assert!(should_fail_over(&anyhow::anyhow!("socket closed")));
        let odd = LlmError::Other(anyhow::anyhow!("odd"));
        assert!(should_fail_over(&odd.into()));
        assert!(should_fail_over(&LlmError::Overloaded.into()));
        let overflow = LlmError::ContextOverflow {
            message: "too long".into(),
        };
        assert!(!should_fail_over(&overflow.into()));
    }

    #[test]
    fn entries_without_a_model_or_credential_are_skipped() {
        let config = HqConfig {
            backends: serde_yaml::from_str(
                r#"
primary: luna
fallbacks: [nomodel, nokey]
backends:
  - name: luna
    kind: github-copilot-api
    model: gpt-6-luna
  - name: nomodel
    kind: github-copilot-api
  - name: nokey
    kind: openai-compatible
    endpoint: https://api.example.test/v1
    credential_env: HQ_TEST_UNSET_KEY_4F2A
    model: m
"#,
            )
            .unwrap(),
            ..Default::default()
        };
        let chain = ChainProvider::from_config(&config).unwrap();
        assert_eq!(chain.backend_names(), vec!["luna"]);
    }
}
