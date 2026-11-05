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
use crate::cost::ProviderClass;
use crate::instrument::{InstrumentedProvider, Instruments};
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
/// set; otherwise the matching `HqConfig` key for the kind, unless the entry
/// names a variable of its own ([`BackendEntry::may_use_config_key`]). Local
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
        _ if !entry.may_use_config_key() => None,
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
    Arc::new(compat_provider(endpoint, api_key, effort, wire))
}

fn compat_provider(
    endpoint: &str,
    api_key: &str,
    effort: Option<String>,
    wire: WireApi,
) -> OpenRouterProvider {
    OpenRouterProvider::new_with_base(api_key, endpoint)
        .with_thinking_effort(effort)
        .with_responses_api(wire == WireApi::Responses)
}

/// The client for an entry whose endpoint is reached through its `unix_socket`.
#[cfg(unix)]
fn socket_client(entry: &BackendEntry) -> Result<reqwest::Client, String> {
    let path = entry
        .unix_socket
        .as_deref()
        .unwrap_or(std::path::Path::new(""));
    crate::http::unix_socket_client(path)
        .map_err(|e| format!("backend '{}': unix_socket client: {e}", entry.name))
}

#[cfg(not(unix))]
fn socket_client(entry: &BackendEntry) -> Result<reqwest::Client, String> {
    Err(format!(
        "backend '{}': unix_socket needs a unix machine",
        entry.name
    ))
}

/// Build the HTTP provider for an API-kind entry.
///
/// `Ok(None)`: a CLI kind, or a required credential is missing (skip, not
/// fatal). `Err`: a construction problem such as a missing endpoint.
pub fn api_provider(
    entry: &BackendEntry,
    creds: &ConfigCredentials,
) -> Result<Option<Arc<dyn LlmProvider>>, String> {
    Ok(build_api_provider(entry, creds)?.map(|provider| {
        InstrumentedProvider::wrap(
            provider,
            &entry.name,
            backend_class(entry),
            Instruments::global(),
        )
    }))
}

fn build_api_provider(
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
        BackendKind::OpenaiCompatible if entry.unix_socket.is_some() => {
            let endpoint = endpoint()?;
            let Some(key) = resolve_credential(entry, creds) else {
                return Ok(None);
            };
            let provider = compat_provider(&endpoint, &key, entry.effort.clone(), entry.wire)
                .with_http_client(socket_client(entry)?);
            Ok(Some(with_prompted_tools(entry, Arc::new(provider))))
        }
        BackendKind::Openrouter | BackendKind::KimiCode | BackendKind::OpenaiCompatible => {
            let endpoint = endpoint()?;
            Ok(resolve_credential(entry, creds).map(|key| {
                let provider =
                    openai_compat_provider(&endpoint, &key, entry.effort.clone(), entry.wire);
                with_prompted_tools(entry, provider)
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

/// The provider behind the prompted-tools shim when the entry asks for it.
fn with_prompted_tools(
    entry: &BackendEntry,
    provider: Arc<dyn LlmProvider>,
) -> Arc<dyn LlmProvider> {
    if entry.prompted_tools {
        Arc::new(crate::prompted_tools::PromptedToolsProvider::new(provider))
    } else {
        provider
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

fn backend_class(entry: &BackendEntry) -> ProviderClass {
    match entry.kind {
        BackendKind::GithubCopilotApi | BackendKind::GithubCopilotCli | BackendKind::KimiCode => {
            ProviderClass::Flat
        }
        _ => ProviderClass::of_endpoint(entry.resolved_endpoint().as_deref()),
    }
}

impl ChainProvider {
    /// The chain from `backends:`, primary first. `None` when no backend is
    /// usable. Entries without a model are skipped: an alias like "fast" is
    /// not a model any backend serves.
    pub fn from_config(config: &HqConfig) -> Option<Self> {
        let creds = ConfigCredentials::from_config(config);
        let backends = config.backends_in_force();
        let mut links = Vec::new();
        for name in backends.chain_order() {
            let Some(entry) = backends.backend(&name).filter(|e| e.enabled) else {
                continue;
            };
            let Some(model) = entry.model.clone().filter(|m| !m.trim().is_empty()) else {
                tracing::warn!(backend = %name, "backend chain: no model set, skipped for background calls");
                continue;
            };
            match api_provider(entry, &creds) {
                Ok(Some(provider)) => links.push(Link {
                    provider,
                    name,
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
                provider,
                name,
                model: None,
            })
            .collect();
        Self { links }
    }

    #[cfg(test)]
    pub(crate) fn from_links(
        links: Vec<(&str, Arc<dyn LlmProvider>, &str)>,
        instruments: Arc<Instruments>,
    ) -> Self {
        Self {
            links: links
                .into_iter()
                .map(|(name, provider, model)| Link {
                    name: name.to_string(),
                    provider: InstrumentedProvider::wrap(
                        provider,
                        name,
                        ProviderClass::Metered,
                        instruments.clone(),
                    ),
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

    fn is_instrumented(&self) -> bool {
        true
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

    #[cfg(unix)]
    mod over_a_socket {
        use super::*;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        const ANSWER: &str = r#"{"id":"x","model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"pong"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;
        const STREAM: &str = "data: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"pong\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"x\",\"object\":\"chat.completion.chunk\",\"created\":1,\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

        /// Answers each request on `socket` with `body` and records its request line and `Host`.
        async fn serve(
            listener: tokio::net::UnixListener,
            body: &'static str,
            kind: &'static str,
        ) -> Arc<Mutex<Vec<String>>> {
            let seen: Arc<Mutex<Vec<String>>> = Arc::default();
            let record = seen.clone();
            tokio::spawn(async move {
                while let Ok((mut sock, _)) = listener.accept().await {
                    let mut buf = vec![0u8; 64 * 1024];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    let line = head.lines().next().unwrap_or_default().to_string();
                    let host = head
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("host:"))
                        .unwrap_or_default()
                        .to_string();
                    record
                        .lock()
                        .unwrap()
                        .push(format!("{line} | {host} | {head}"));
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: {kind}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = sock.write_all(resp.as_bytes()).await;
                }
            });
            seen
        }

        fn entry(socket: &std::path::Path) -> BackendEntry {
            BackendEntry {
                name: "laptop".into(),
                kind: BackendKind::OpenaiCompatible,
                endpoint: Some("http://localhost/v1".into()),
                credential_env: None,
                model: Some("m".into()),
                effort: None,
                wire: WireApi::ChatCompletions,
                enabled: true,
                unix_socket: Some(socket.to_path_buf()),
                prompted_tools: false,
            }
        }

        fn request() -> ChatRequest {
            ChatRequest {
                model: "m".into(),
                messages: vec![ChatMessage {
                    image_parts: Vec::new(),
                    role: MessageRole::User,
                    content: "hi".into(),
                    tool_calls: vec![],
                    tool_call_id: None,
                    reasoning_content: None,
                }],
                tools: vec![],
                temperature: None,
                max_tokens: None,
            }
        }

        #[tokio::test]
        async fn a_backend_with_a_unix_socket_sends_every_request_through_it() {
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("llm.sock");
            let seen = serve(
                tokio::net::UnixListener::bind(&socket).unwrap(),
                ANSWER,
                "application/json",
            )
            .await;
            let provider = build_api_provider(&entry(&socket), &ConfigCredentials::default())
                .unwrap()
                .expect("a loopback endpoint needs no key");
            let answer = provider.chat(&request()).await.unwrap();
            assert_eq!(answer.message.content, "pong");
            let seen = seen.lock().unwrap().clone();
            assert_eq!(seen.len(), 1, "{seen:?}");
            assert!(seen[0].starts_with("POST /v1/chat/completions"), "{seen:?}");
            assert!(
                seen[0].to_ascii_lowercase().contains("host: localhost"),
                "{seen:?}"
            );
        }

        const TOOL_TAG_ANSWER: &str = r#"{"id":"x","model":"m","choices":[{"index":0,"message":{"role":"assistant","content":"Checking. <tool_call name=\"task_list\"><args>{\"limit\":3}</args></tool_call>"}}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;

        #[tokio::test]
        async fn prompted_tools_puts_the_tools_in_the_prompt_and_reads_the_tags_back() {
            use tokio_stream::StreamExt;
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("llm.sock");
            let seen = serve(
                tokio::net::UnixListener::bind(&socket).unwrap(),
                TOOL_TAG_ANSWER,
                "application/json",
            )
            .await;
            let mut with_shim = entry(&socket);
            with_shim.prompted_tools = true;
            let provider = build_api_provider(&with_shim, &ConfigCredentials::default())
                .unwrap()
                .unwrap();
            let mut asked = request();
            asked.tools = vec![hq_core::types::ToolDefinition {
                name: "task_list".into(),
                description: "List tasks".into(),
                parameters: serde_json::json!({"type": "object"}),
            }];
            let mut stream = provider.chat_stream(&asked).await.unwrap();
            let (mut text, mut name, mut args) = (String::new(), None, String::new());
            while let Some(chunk) = stream.next().await {
                match chunk.unwrap() {
                    StreamChunk::Text(t) => text.push_str(&t),
                    StreamChunk::ToolCallDelta {
                        index,
                        name: n,
                        arguments_delta,
                        ..
                    } => {
                        assert_eq!(index, 0, "one call");
                        name = n.or(name);
                        args.push_str(&arguments_delta);
                    }
                    _ => {}
                }
            }
            assert_eq!(text, "Checking.");
            assert_eq!(name.as_deref(), Some("task_list"));
            assert_eq!(args, r#"{"limit":3}"#);
            let seen = seen.lock().unwrap().clone();
            assert_eq!(seen.len(), 1, "{seen:?}");
            assert!(
                !seen[0].contains(r#""tools""#),
                "no native tools field: {seen:?}"
            );
            assert!(
                seen[0].contains("task_list"),
                "the tool is in the prompt: {seen:?}"
            );
        }

        #[tokio::test]
        async fn without_prompted_tools_the_tools_go_in_the_tools_field() {
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("llm.sock");
            let seen = serve(
                tokio::net::UnixListener::bind(&socket).unwrap(),
                ANSWER,
                "application/json",
            )
            .await;
            let provider = build_api_provider(&entry(&socket), &ConfigCredentials::default())
                .unwrap()
                .unwrap();
            let mut asked = request();
            asked.tools = vec![hq_core::types::ToolDefinition {
                name: "task_list".into(),
                description: "List tasks".into(),
                parameters: serde_json::json!({"type": "object"}),
            }];
            provider.chat(&asked).await.unwrap();
            assert!(seen.lock().unwrap()[0].contains(r#""tools""#));
        }

        #[tokio::test]
        async fn a_streamed_reply_comes_through_the_socket_too() {
            use tokio_stream::StreamExt;
            let dir = tempfile::tempdir().unwrap();
            let socket = dir.path().join("llm.sock");
            let seen = serve(
                tokio::net::UnixListener::bind(&socket).unwrap(),
                STREAM,
                "text/event-stream",
            )
            .await;
            let provider = build_api_provider(&entry(&socket), &ConfigCredentials::default())
                .unwrap()
                .unwrap();
            let mut stream = provider.chat_stream(&request()).await.unwrap();
            let mut text = String::new();
            while let Some(chunk) = stream.next().await {
                if let StreamChunk::Text(t) = chunk.unwrap() {
                    text.push_str(&t);
                }
            }
            assert_eq!(text, "pong");
            assert_eq!(seen.lock().unwrap().len(), 1);
        }
    }

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
                reasoning_tokens: 0,
                provider_cost_usd: None,
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
        let chain = ChainProvider::from_links(
            vec![
                ("luna", luna.clone(), "gpt-6-luna"),
                ("copilot", gemini.clone(), "gemini-3.8-flash"),
            ],
            Instruments::new(),
        );
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
        let chain = ChainProvider::from_links(
            vec![
                ("luna", luna.clone(), "gpt-6-luna"),
                ("copilot", gemini.clone(), "gemini-3.8-flash"),
            ],
            Instruments::new(),
        );
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
        let chain = ChainProvider::from_links(
            vec![
                ("luna", luna, "gpt-6-luna"),
                ("copilot", gemini.clone(), "gemini-3.8-flash"),
            ],
            Instruments::new(),
        );
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

    /// Opt-in check against a real model that ignores `tools`:
    /// `HQ_TEST_PROMPTED_MODEL=gemma4:e2b cargo test -p hq-llm prompted_tools_live -- --ignored`
    /// with Ollama on its default port.
    #[tokio::test]
    #[ignore = "needs Ollama on 127.0.0.1:11434 and HQ_TEST_PROMPTED_MODEL"]
    async fn prompted_tools_live() {
        let model = std::env::var("HQ_TEST_PROMPTED_MODEL").expect("HQ_TEST_PROMPTED_MODEL");
        let entry = BackendEntry {
            name: "local".into(),
            kind: BackendKind::OpenaiCompatible,
            endpoint: Some("http://127.0.0.1:11434/v1".into()),
            credential_env: None,
            model: Some(model.clone()),
            effort: None,
            wire: WireApi::ChatCompletions,
            enabled: true,
            unix_socket: None,
            prompted_tools: true,
        };
        let provider = build_api_provider(&entry, &ConfigCredentials::default())
            .unwrap()
            .unwrap();
        let request = ChatRequest {
            model,
            messages: vec![ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::User,
                content: "What is the weather in Kampala right now? Use the tool.".into(),
                tool_calls: vec![],
                tool_call_id: None,
                reasoning_content: None,
            }],
            tools: vec![hq_core::types::ToolDefinition {
                name: "get_weather".into(),
                description: "Current weather for a city".into(),
                parameters: serde_json::json!({
                    "type": "object",
                    "properties": {"city": {"type": "string"}},
                    "required": ["city"]
                }),
            }],
            temperature: Some(0.0),
            max_tokens: None,
        };
        let reply = provider.chat(&request).await.unwrap();
        let call = reply.message.tool_calls.first().expect("a tool call");
        assert_eq!(call.name, "get_weather", "{reply:?}");
        assert!(
            call.arguments
                .to_string()
                .to_lowercase()
                .contains("kampala"),
            "{reply:?}"
        );
    }

    #[test]
    fn a_backend_that_names_its_own_key_never_gets_the_generic_one() {
        let creds = ConfigCredentials {
            openai: Some("generic-openai-key".into()),
            openrouter: Some("generic-openrouter-key".into()),
            ..Default::default()
        };
        let entry = |kind, endpoint: &str, env: Option<&str>| BackendEntry {
            name: "b".into(),
            kind,
            endpoint: Some(endpoint.into()),
            credential_env: env.map(str::to_string),
            model: Some("m".into()),
            effort: None,
            wire: WireApi::default(),
            enabled: true,
            unix_socket: None,
            prompted_tools: false,
        };
        let unset = Some("HQ_TEST_UNSET_KEY_9C1D");
        let remote = "https://api.example.test/v1";
        let cases = [
            (BackendKind::OpenaiCompatible, remote, unset, None),
            (
                BackendKind::OpenaiCompatible,
                "http://127.0.0.1:47600/v1",
                unset,
                Some("local"),
            ),
            (
                BackendKind::OpenaiCompatible,
                "http://localhost/v1",
                unset,
                Some("local"),
            ),
            (BackendKind::Openrouter, remote, unset, None),
            (
                BackendKind::OpenaiCompatible,
                remote,
                None,
                Some("generic-openai-key"),
            ),
            (
                BackendKind::OpenaiCompatible,
                remote,
                Some("OPENAI_API_KEY"),
                Some("generic-openai-key"),
            ),
            (
                BackendKind::Openrouter,
                remote,
                Some("OPENROUTER_API_KEY"),
                Some("generic-openrouter-key"),
            ),
            (
                BackendKind::OpenaiCompatible,
                remote,
                Some(" "),
                Some("generic-openai-key"),
            ),
        ];
        for (kind, endpoint, env, want) in cases {
            let got = resolve_credential(&entry(kind, endpoint, env), &creds);
            assert_eq!(got.as_deref(), want, "{kind:?} {endpoint} {env:?}");
        }
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
