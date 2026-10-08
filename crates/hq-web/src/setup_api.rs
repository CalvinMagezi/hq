//! First-run setup: lets a fresh install (typically a server nobody has a shell on yet)
//! take its first model key from the web UI.
//!
//! These routes write a credential to disk, so they refuse to run unless the instance has
//! a web token. There is deliberately no loopback exception: behind `tailscale serve` or a
//! reverse proxy every request arrives from 127.0.0.1. They also refuse once a key exists,
//! so setup cannot be used to swap keys later (that is Settings and `hq env`).
//!
//! OpenRouter only: the chat router consumes `openrouter_api_key` from config, but an
//! Anthropic or Google key written by setup would not reach it (those are read only by
//! explicit `backends:` chains), so offering them would strand a fresh server.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use hq_core::config::HqConfig;
use hq_core::setup_provider::SetupProvider;
use hq_core::types::{ChatMessage, MessageRole};
use hq_llm::OpenRouterProvider;
use hq_llm::provider::{ChatRequest, LlmProvider};
use serde::Deserialize;
use serde_json::json;

use crate::WsState;
use crate::error::ApiError;

const TEST_TIMEOUT: Duration = Duration::from_secs(15);
const TEST_MAX_TOKENS: u32 = 1;
const TEST_PROMPT: &str = "Reply with one word.";
const MAX_KEY_CHARS: usize = 512;
/// The router alias a config carries until someone picks a model; anything else is deliberate.
const STOCK_DEFAULT_MODEL: &str = "relay";

const PROVIDER: SetupProvider = SetupProvider::OpenRouter;

/// Serializes the "no key yet" check with the config write, so two setups cannot both pass it.
static SAVE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[derive(Deserialize)]
pub(crate) struct KeyBody {
    api_key: String,
}

fn token_configured(state: &WsState) -> bool {
    state
        .web_auth_token
        .as_deref()
        .is_some_and(|t| !t.trim().is_empty())
}

fn require_token(state: &WsState) -> Result<(), ApiError> {
    if token_configured(state) {
        return Ok(());
    }
    Err(ApiError::Forbidden(
        "first-run setup needs a web token (web_auth_token); use `hq env` on the server instead"
            .into(),
    ))
}

async fn load_config() -> Result<HqConfig, ApiError> {
    tokio::task::spawn_blocking(HqConfig::load)
        .await
        .map_err(ApiError::internal)?
        .map_err(|e| ApiError::internal(format!("could not read the HQ config: {e}")))
}

fn clean_key(raw: &str) -> Result<String, ApiError> {
    let key = raw.trim();
    if key.is_empty()
        || key.chars().count() > MAX_KEY_CHARS
        || key.chars().any(|c| c.is_whitespace() || c.is_control())
    {
        return Err(ApiError::bad_request("that does not look like an API key"));
    }
    Ok(key.to_string())
}

/// One-token completion. The error text is the provider's, with the key scrubbed.
async fn try_key(key: &str) -> Result<(), String> {
    let request = ChatRequest {
        model: PROVIDER.cheap_model().to_string(),
        messages: vec![ChatMessage {
            role: MessageRole::User,
            content: TEST_PROMPT.to_string(),
            tool_calls: Vec::new(),
            tool_call_id: None,
            reasoning_content: None,
            image_parts: Vec::new(),
        }],
        tools: Vec::new(),
        temperature: None,
        max_tokens: Some(TEST_MAX_TOKENS),
    };
    let llm = OpenRouterProvider::new(key);
    match tokio::time::timeout(TEST_TIMEOUT, llm.chat(&request)).await {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(e.to_string().replace(key, "[key]")),
        Err(_) => Err("the provider did not answer in time".to_string()),
    }
}

pub(crate) async fn status_handler(
    State(state): State<Arc<WsState>>,
) -> Result<impl IntoResponse, ApiError> {
    let config = load_config().await?;
    Ok(Json(json!({
        "needs_setup": !config.has_llm_key(),
        "available": token_configured(&state),
    })))
}

pub(crate) async fn test_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<KeyBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_token(&state)?;
    let key = clean_key(&body.api_key)?;
    Ok(Json(match try_key(&key).await {
        Ok(()) => json!({"ok": true}),
        Err(error) => json!({"ok": false, "error": error}),
    }))
}

pub(crate) async fn provider_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<KeyBody>,
) -> Result<impl IntoResponse, ApiError> {
    require_token(&state)?;
    let key = clean_key(&body.api_key)?;
    let _saving = SAVE_LOCK.lock().await;
    if load_config().await?.has_llm_key() {
        return Err(ApiError::Conflict(
            "a model key is already configured".into(),
        ));
    }
    tokio::task::spawn_blocking(move || {
        HqConfig::save_patch(|c| {
            PROVIDER.set_key(c, key);
            if c.default_model == STOCK_DEFAULT_MODEL || c.default_model.starts_with("ollama/") {
                c.default_model = PROVIDER.cheap_model().to_string();
            }
            c.apply_cloud_key_flip(true);
        })
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(|e| ApiError::internal(format!("could not save the config: {e}")))?;
    Ok(Json(json!({"ok": true})))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    const PROVIDER_ENV: [&str; 4] = [
        "OPENROUTER_API_KEY",
        "ANTHROPIC_API_KEY",
        "GOOGLE_AI_API_KEY",
        "GEMINI_API_KEY",
    ];

    fn router(token: Option<&str>, vault: &std::path::Path) -> axum::Router {
        let mut state = WsState::new(vault.to_path_buf(), None);
        state.web_auth_token = token.map(str::to_string);
        crate::create_router(Arc::new(state))
    }

    fn post(path: &str, body: &str) -> Request<Body> {
        Request::post(path)
            .header("content-type", "application/json")
            .header("authorization", "Bearer s3cret")
            .header("x-hq-client", "web")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn status_of(app: &axum::Router, req: Request<Body>) -> StatusCode {
        app.clone().oneshot(req).await.unwrap().status()
    }

    #[test]
    fn keys_are_trimmed_and_junk_is_refused() {
        assert_eq!(clean_key("  sk-abc \n").unwrap(), "sk-abc");
        for bad in [
            "",
            "   ",
            "two words",
            "a\tb",
            &"k".repeat(MAX_KEY_CHARS + 1),
        ] {
            assert!(clean_key(bad).is_err(), "{bad:?}");
        }
    }

    /// One test owns the process env (HQ_CONFIG_PATH and the provider key variables),
    /// so nothing else in the crate can race it.
    #[tokio::test]
    async fn setup_needs_a_token_writes_once_and_never_touches_the_real_config() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("config.yaml");
        unsafe {
            std::env::set_var("HQ_CONFIG_PATH", &config_path);
            for name in PROVIDER_ENV {
                std::env::remove_var(name);
            }
        }
        let body = r#"{"api_key":"sk-test-123"}"#;

        let no_token = router(None, dir.path());
        assert_eq!(
            status_of(&no_token, post("/api/setup/provider", body)).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            status_of(&no_token, post("/api/setup/test", body)).await,
            StatusCode::FORBIDDEN
        );
        assert!(!config_path.exists());

        let app = router(Some("s3cret"), dir.path());
        assert_eq!(
            status_of(&app, post("/api/setup/provider", r#"{"api_key":"a b"}"#)).await,
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            status_of(&app, post("/api/setup/provider", body)).await,
            StatusCode::OK
        );

        let saved = HqConfig::load_from_path(&config_path).unwrap();
        assert_eq!(saved.openrouter_api_key.as_deref(), Some("sk-test-123"));
        assert_eq!(saved.default_model, PROVIDER.cheap_model());
        assert!(!saved.local_only);

        assert_eq!(
            status_of(&app, post("/api/setup/provider", body)).await,
            StatusCode::CONFLICT
        );

        let status = app
            .clone()
            .oneshot(
                Request::get("/api/setup/status")
                    .header("authorization", "Bearer s3cret")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(status.into_body(), 4096)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v, json!({"needs_setup": false, "available": true}));

        // A model somebody chose on purpose survives setup.
        std::fs::write(&config_path, "default_model: my/custom-model\n").unwrap();
        assert_eq!(
            status_of(&app, post("/api/setup/provider", body)).await,
            StatusCode::OK
        );
        let kept = HqConfig::load_from_path(&config_path).unwrap();
        assert_eq!(kept.default_model, "my/custom-model");

        unsafe { std::env::remove_var("HQ_CONFIG_PATH") };
    }
}
