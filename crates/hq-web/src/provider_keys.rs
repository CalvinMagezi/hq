//! Replacing or adding a model provider key from the web Settings page, after first-run setup.
//!
//! A key is checked with a one-token call before it is saved, replaces any earlier key for that
//! provider, and never appears in a response or a log line. A key the server's environment pins
//! is refused: a config write would be hidden behind it.
//!
//! The model choice stays as it is when a key already existed. Only a first key (nothing in the
//! config or the environment) gets setup's treatment of moving a stock default onto that
//! provider's cheap model, because otherwise the next turn would have nothing to answer with.

use std::path::Path;
use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::response::IntoResponse;
use hq_core::config::HqConfig;
use hq_core::setup_provider::SetupProvider;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::WsState;
use crate::error::ApiError;
use crate::setup_api::{SAVE_LOCK, clean_key, first_key_patch, scrub_key, token_configured};

const NO_WEB_TOKEN: &str = "Changing model keys from the web needs a web token (web_auth_token). Use `hq env` on the server instead.";
/// Variables under this prefix override config.yaml through figment, even when empty.
const HQ_ENV_PREFIX: &str = "HQ_";

#[derive(Deserialize)]
pub(crate) struct ReplaceBody {
    provider: SetupProvider,
    api_key: String,
}

/// Variables that set this provider's key ahead of config.yaml: the `HQ_` override of the config
/// field, and the conventional names the chat router prefers over the config value.
fn env_names(provider: SetupProvider) -> &'static [&'static str] {
    match provider {
        SetupProvider::OpenRouter => &["HQ_OPENROUTER_API_KEY", "OPENROUTER_API_KEY"],
        SetupProvider::Anthropic => &["HQ_ANTHROPIC_API_KEY", "ANTHROPIC_API_KEY"],
        // GOOGLE_AI_API_KEY is left out: the router prefers GEMINI_API_KEY, which HQ fills from config.
        SetupProvider::Google => &["HQ_GOOGLE_AI_API_KEY", "GEMINI_API_KEY"],
    }
}

/// The variable that pins this provider's key from the environment, if any.
pub(crate) fn env_lock(
    provider: SetupProvider,
    env_set: &dyn Fn(&str) -> bool,
) -> Option<&'static str> {
    env_names(provider).iter().copied().find(|n| env_set(n))
}

/// Whether the operator set `name` in HQ's environment. A copy HQ made itself from config.yaml
/// does not count, and neither does an empty conventional variable, which the router ignores.
pub(crate) fn operator_env_set(name: &str) -> bool {
    if hq_llm::router::exported_from_config(name) {
        return false;
    }
    let figment_override = name.starts_with(HQ_ENV_PREFIX);
    std::env::vars_os().any(|(k, v)| {
        k.to_str().is_some_and(|k| k.eq_ignore_ascii_case(name))
            && (figment_override || v.to_str().is_some_and(|v| !v.trim().is_empty()))
    })
}

/// Checks `raw_key` with `test` and saves it for `provider` in the config file at `config_path`.
async fn replace_key_at(
    config_path: &Path,
    provider: SetupProvider,
    raw_key: &str,
    env_set: &(dyn Fn(&str) -> bool + Sync),
    test: impl AsyncFn(SetupProvider, &str) -> Result<(), String>,
) -> Result<Value, ApiError> {
    if let Some(name) = env_lock(provider, env_set) {
        return Err(ApiError::Conflict(format!(
            "This key is set by the server's environment ({name}). Change it there."
        )));
    }
    let key = clean_key(raw_key)?;
    if let Err(error) = test(provider, &key).await {
        return Err(ApiError::bad_request(format!(
            "The provider did not accept that key: {}",
            scrub_key(&error, &key)
        )));
    }
    let _saving = SAVE_LOCK.lock().await;
    let path = config_path.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let first = !HqConfig::load_from_path(&path)?.has_llm_key();
        HqConfig::save_patch_to_path(&path, |c| {
            if first {
                return first_key_patch(c, provider, key);
            }
            provider.set_key(c, key);
            c.apply_cloud_key_flip(true);
        })
    })
    .await
    .map_err(ApiError::internal)?
    .map_err(|e| ApiError::internal(format!("could not save the config: {e}")))?;
    Ok(json!({"ok": true, "provider": provider}))
}

pub(crate) async fn replace_handler(
    State(state): State<Arc<WsState>>,
    Json(body): Json<ReplaceBody>,
) -> Result<impl IntoResponse, ApiError> {
    if !token_configured(&state) {
        return Err(ApiError::Forbidden(NO_WEB_TOKEN.into()));
    }
    let path = HqConfig::config_file_path();
    let saved = replace_key_at(
        &path,
        body.provider,
        &body.api_key,
        &operator_env_set,
        crate::setup_api::try_key,
    )
    .await?;
    Ok(Json(saved))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    const PROVIDERS: [SetupProvider; 3] = [
        SetupProvider::OpenRouter,
        SetupProvider::Anthropic,
        SetupProvider::Google,
    ];

    const PATH: &str = "/api/settings/provider-key";
    const OLD_KEY: &str = "sk-old-111";
    const NEW_KEY: &str = "sk-new-222";
    const CHOSEN_MODEL: &str = "my/chosen-model";

    fn no_env(_: &str) -> bool {
        false
    }

    async fn accepts(_: SetupProvider, _: &str) -> Result<(), String> {
        Ok(())
    }

    /// A provider error that quotes the key back, as some do.
    async fn refuses(_: SetupProvider, key: &str) -> Result<(), String> {
        Err(format!("401 invalid key {key}"))
    }

    fn config_with_old_key(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("config.yaml");
        std::fs::write(
            &path,
            format!("openrouter_api_key: {OLD_KEY}\ndefault_model: {CHOSEN_MODEL}\n"),
        )
        .unwrap();
        path
    }

    #[tokio::test]
    async fn a_working_key_replaces_the_old_one_and_leaves_the_model_alone() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_with_old_key(dir.path());

        let body = replace_key_at(&path, SetupProvider::OpenRouter, NEW_KEY, &no_env, accepts)
            .await
            .unwrap();

        assert_eq!(body, json!({"ok": true, "provider": "open_router"}));
        assert!(!body.to_string().contains(NEW_KEY));
        let saved = HqConfig::load_from_path(&path).unwrap();
        assert_eq!(saved.openrouter_api_key.as_deref(), Some(NEW_KEY));
        assert_eq!(saved.default_model, CHOSEN_MODEL);
    }

    #[tokio::test]
    async fn a_second_provider_is_added_beside_the_first() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_with_old_key(dir.path());

        replace_key_at(&path, SetupProvider::Anthropic, NEW_KEY, &no_env, accepts)
            .await
            .unwrap();

        let saved = HqConfig::load_from_path(&path).unwrap();
        assert_eq!(saved.openrouter_api_key.as_deref(), Some(OLD_KEY));
        assert_eq!(saved.anthropic_api_key.as_deref(), Some(NEW_KEY));
        assert_eq!(saved.default_model, CHOSEN_MODEL);
    }

    #[tokio::test]
    async fn a_refused_key_is_not_saved_and_not_echoed() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_with_old_key(dir.path());

        let err = replace_key_at(&path, SetupProvider::OpenRouter, NEW_KEY, &no_env, refuses)
            .await
            .unwrap_err();

        assert!(matches!(err, ApiError::BadRequest(_)), "{err:?}");
        assert!(!err.to_string().contains(NEW_KEY), "{err}");
        assert!(err.to_string().contains("[key]"), "{err}");
        let saved = HqConfig::load_from_path(&path).unwrap();
        assert_eq!(saved.openrouter_api_key.as_deref(), Some(OLD_KEY));
    }

    #[tokio::test]
    async fn a_key_pinned_by_the_environment_is_refused_before_any_check() {
        let dir = tempfile::tempdir().unwrap();
        let path = config_with_old_key(dir.path());
        let gemini_env = |n: &str| n == "GEMINI_API_KEY";

        let err = replace_key_at(&path, SetupProvider::Google, NEW_KEY, &gemini_env, refuses)
            .await
            .unwrap_err();

        assert!(matches!(&err, ApiError::Conflict(m) if m.contains("GEMINI_API_KEY")));
        assert!(
            replace_key_at(
                &path,
                SetupProvider::OpenRouter,
                NEW_KEY,
                &gemini_env,
                accepts
            )
            .await
            .is_ok(),
            "another provider's variable does not pin this one"
        );
        let saved = HqConfig::load_from_path(&path).unwrap();
        assert!(saved.google_ai_api_key.is_none());
    }

    #[test]
    fn each_provider_is_pinned_by_its_hq_override_too() {
        for provider in PROVIDERS {
            let first = env_names(provider)[0];
            assert!(first.starts_with(HQ_ENV_PREFIX), "{provider:?}");
            assert_eq!(env_lock(provider, &|n| n == first), Some(first));
            assert_eq!(env_lock(provider, &no_env), None);
        }
        let google_ai = |n: &str| n == "GOOGLE_AI_API_KEY";
        assert_eq!(env_lock(SetupProvider::Google, &google_ai), None);
    }

    fn router(token: Option<&str>, vault: &Path) -> axum::Router {
        let mut state = WsState::new(vault.to_path_buf(), None);
        state.web_auth_token = token.map(str::to_string);
        crate::create_router(Arc::new(state))
    }

    fn post(body: &str, client_header: bool) -> Request<Body> {
        let mut req = Request::post(PATH)
            .header("content-type", "application/json")
            .header("authorization", "Bearer s3cret");
        if client_header {
            req = req.header("x-hq-client", "web");
        }
        req.body(Body::from(body.to_string())).unwrap()
    }

    async fn status(app: axum::Router, req: Request<Body>) -> StatusCode {
        app.oneshot(req).await.unwrap().status()
    }

    /// Only refusals that answer before the config is read, so no test here touches it.
    #[tokio::test]
    async fn the_route_needs_a_web_token_the_client_header_and_a_real_key() {
        let dir = tempfile::tempdir().unwrap();
        let body = r#"{"provider":"anthropic","api_key":"sk-test-123"}"#;
        let no_token = router(None, dir.path());
        assert_eq!(
            status(no_token, post(body, true)).await,
            StatusCode::FORBIDDEN
        );

        let app = router(Some("s3cret"), dir.path());
        assert_eq!(
            status(app.clone(), post(body, false)).await,
            StatusCode::FORBIDDEN
        );
        let junk = r#"{"provider":"anthropic","api_key":"two words"}"#;
        let refused = app.oneshot(post(junk, true)).await.unwrap();
        // An Anthropic key in the test runner's own environment answers 409 first.
        assert!(
            [StatusCode::BAD_REQUEST, StatusCode::CONFLICT].contains(&refused.status()),
            "{}",
            refused.status()
        );
    }
}
