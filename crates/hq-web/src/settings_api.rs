//! View of the running HQ configuration for the web app's Settings page. Provider keys are
//! changed through `provider_keys`; this view stays read-only.
//!
//! The view is built field by field. Nothing is serialized wholesale, so a secret added
//! to `HqConfig` later cannot leak here by accident.

use crate::error::ApiError;
use crate::provider_keys;
use axum::response::IntoResponse;
use hq_core::config::{HqConfig, resolve_session_model};
use hq_core::setup_provider::SetupProvider;
use serde_json::{Value, json};

/// Host part of a URL, so an endpoint can be shown without its path or credentials.
fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    authority
        .rsplit('@')
        .next()
        .unwrap_or(authority)
        .to_string()
}

fn present(key: &Option<String>) -> bool {
    key.as_deref().is_some_and(|k| !k.trim().is_empty())
}

/// A provider the Settings page can change: `id` is what the replace endpoint takes, and
/// `source` says where the key in force comes from (`env` cannot be changed from the web).
fn editable_key(
    name: &str,
    provider: SetupProvider,
    key: &Option<String>,
    env_set: &dyn Fn(&str) -> bool,
) -> Value {
    let source = if provider_keys::env_lock(provider, env_set).is_some() {
        Some("env")
    } else {
        present(key).then_some("config")
    };
    json!({"name": name, "id": provider, "configured": present(key), "source": source})
}

fn key_presence(config: &HqConfig, env_set: &dyn Fn(&str) -> bool) -> Value {
    json!([
        editable_key("OpenRouter", SetupProvider::OpenRouter, &config.openrouter_api_key, env_set),
        editable_key("Anthropic", SetupProvider::Anthropic, &config.anthropic_api_key, env_set),
        editable_key("Google AI", SetupProvider::Google, &config.google_ai_api_key, env_set),
        {"name": "DeepSeek", "configured": present(&config.deepseek_api_key)},
        {"name": "OpenAI", "configured": present(&config.openai_api_key)},
        {"name": "Groq", "configured": present(&config.groq_api_key)},
        {"name": "Cerebras", "configured": present(&config.cerebras_api_key)},
        {"name": "Kimi Code", "configured": present(&config.kimi_code_api_key)},
        {"name": "Brave Search", "configured": present(&config.brave_api_key)},
    ])
}

fn backends_view(config: &HqConfig) -> Value {
    let chain = &config.backends;
    let entries: Vec<Value> = chain
        .backends
        .iter()
        .map(|b| {
            json!({
                "name": b.name,
                "kind": serde_json::to_value(b.kind).unwrap_or(Value::Null),
                "model": b.model,
                "effort": b.effort,
                "endpoint_host": b.endpoint.as_deref().map(host_of),
                "enabled": b.enabled,
                "primary": b.name == chain.primary,
                "fallback_position": chain.fallbacks.iter().position(|f| *f == b.name),
            })
        })
        .collect();
    json!({
        "configured": chain.is_configured(),
        "primary": chain.primary,
        "fallbacks": chain.fallbacks,
        "entries": entries,
    })
}

pub(crate) fn settings_view(config: &HqConfig, env_set: &dyn Fn(&str) -> bool) -> Value {
    let sandbox = serde_json::to_value(config.governance.bash.sandbox).unwrap_or(Value::Null);
    let hosts: Vec<&String> = config.agent_host.hosts.keys().collect();
    let remote_mcp: Vec<Value> = config
        .remote_mcp
        .iter()
        .map(|s| {
            json!({
                "name": s.name,
                "host": host_of(&s.url),
                "live_user_turn_only": s.live_user_turn_only,
            })
        })
        .collect();
    json!({
        "model": {
            "active": resolve_session_model(config),
            "default_model": config.default_model,
            "relay_override": config.relay.model,
            "local_only": config.local_only,
        },
        "backends": backends_view(config),
        "provider_keys": key_presence(config, env_set),
        "limits": {
            "chat_turn_timeout_secs": config.chat_turn_timeout_secs,
            "turn_ack_timeout_secs": config.relay.turn_ack_timeout_secs,
            "background_turn_max_days": config.relay.background_turn_max_days,
            "background_progress_secs": config.relay.background_progress_secs,
        },
        "safety": {
            "bash_sandbox": sandbox,
            "bash_network": config.governance.bash.network,
            "skills_write_approval": config.governance.skills_write_approval,
            "web_allowed_origins": config.web_allowed_origins,
        },
        "agent_host": {
            "default_host": config.agent_host.default_host,
            "hosts": hosts,
            "sandbox_mode": match config.agent_host.sandbox.mode {
                hq_core::config::SandboxMode::Process => "process",
                hq_core::config::SandboxMode::None => "none",
            },
            "sandbox_extra_domains": config.agent_host.sandbox.allow_domains.len(),
            "idle_reap_hours": config.agent_host.idle_reap_hours,
            "drive_new_watches": config.agent_host.drive_new_watches,
            "driver_checkin_minutes": config.agent_host.driver_checkin_minutes,
            "driver_nudge_budget": config.agent_host.nudge_budget(),
            "driver_no_progress_limit": config.agent_host.no_progress_limit(),
        },
        "integrations": {
            "remote_mcp": remote_mcp,
            "searxng_host": config.searxng_url.as_deref().map(host_of),
            "web_search_native": config.web_search_native,
            "disk_watchdog_enabled": config.disk_watchdog.enabled,
        },
    })
}

pub(crate) async fn settings_handler() -> Result<impl IntoResponse, ApiError> {
    let config = tokio::task::spawn_blocking(HqConfig::load)
        .await
        .map_err(|e| ApiError::internal(format!("settings task failed: {e}")))?
        .map_err(|e| ApiError::internal(format!("could not read the HQ config: {e}")))?;
    Ok(axum::Json(settings_view(
        &config,
        &provider_keys::operator_env_set,
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::RemoteMcpServer;

    fn no_env(_: &str) -> bool {
        false
    }

    #[test]
    fn the_view_never_contains_a_secret_value() {
        let mut config = HqConfig {
            openrouter_api_key: Some("sk-or-SECRETVALUE1".into()),
            anthropic_api_key: Some("sk-ant-SECRETVALUE2".into()),
            ..Default::default()
        };
        config.remote_mcp.push(RemoteMcpServer {
            name: "diagrams".into(),
            url: concat!(
                "https://user:",
                "SECRETVALUE3@mcp.example.com/api/mcp?key=SECRETVALUE4"
            )
            .into(),
            api_key: Some("SECRETVALUE5".into()),
            live_user_turn_only: true,
        });
        let text = settings_view(&config, &no_env).to_string();
        assert!(!text.contains("SECRETVALUE"), "{text}");
        assert!(text.contains("mcp.example.com"));
    }

    #[test]
    fn provider_keys_report_presence_only() {
        let config = HqConfig {
            openrouter_api_key: Some("sk-or-x".into()),
            groq_api_key: Some("   ".into()),
            ..Default::default()
        };
        let view = settings_view(&config, &no_env);
        let keys = view["provider_keys"].as_array().unwrap();
        let configured =
            |name: &str| keys.iter().find(|k| k["name"] == name).unwrap()["configured"].clone();
        assert_eq!(configured("OpenRouter"), json!(true));
        assert_eq!(configured("Groq"), json!(false));
        assert_eq!(configured("Anthropic"), json!(false));
    }

    #[test]
    fn editable_keys_say_whether_the_config_or_the_environment_sets_them() {
        let config = HqConfig {
            openrouter_api_key: Some("sk-or-x".into()),
            anthropic_api_key: Some("sk-ant-x".into()),
            ..Default::default()
        };
        let env = |n: &str| n == "ANTHROPIC_API_KEY";
        let view = settings_view(&config, &env);
        let row = |name: &str| {
            view["provider_keys"]
                .as_array()
                .unwrap()
                .iter()
                .find(|k| k["name"] == name)
                .unwrap()
                .clone()
        };
        assert_eq!(row("OpenRouter")["id"], json!("open_router"));
        assert_eq!(row("OpenRouter")["source"], json!("config"));
        assert_eq!(row("Anthropic")["source"], json!("env"));
        assert_eq!(row("Google AI")["source"], Value::Null);
        assert!(row("Groq").get("source").is_none());
    }

    #[test]
    fn the_active_model_follows_the_session_resolution_rule() {
        let mut config = HqConfig {
            default_model: "model-a".into(),
            ..Default::default()
        };
        assert_eq!(
            settings_view(&config, &no_env)["model"]["active"],
            json!("model-a")
        );
        config.relay.model = Some("model-b".into());
        assert_eq!(
            settings_view(&config, &no_env)["model"]["active"],
            json!("model-b")
        );
    }

    #[test]
    fn each_backend_says_whether_it_is_switched_off() {
        let entry =
            |yaml: &str| serde_yaml::from_str::<hq_core::config::BackendEntry>(yaml).unwrap();
        let mut config = HqConfig::default();
        config.backends.primary = "a".into();
        config.backends.fallbacks = vec!["b".into()];
        config.backends.backends = vec![
            entry(
                "name: a\nkind: openai-compatible\nendpoint: https://api.example.com/v1\nenabled: false\n",
            ),
            entry("name: b\nkind: openai-compatible\nendpoint: https://api.example.org/v1\n"),
        ];
        let view = settings_view(&config, &no_env);
        let entries = view["backends"]["entries"].as_array().unwrap();
        let enabled =
            |name: &str| entries.iter().find(|e| e["name"] == name).unwrap()["enabled"].clone();
        assert_eq!(enabled("a"), json!(false));
        assert_eq!(enabled("b"), json!(true));
    }

    #[test]
    fn host_of_drops_credentials_path_and_query() {
        assert_eq!(
            host_of("https://u:p@host.example:8443/a/b?k=v"),
            "host.example:8443"
        );
        assert_eq!(host_of("http://127.0.0.1:8080"), "127.0.0.1:8080");
    }
}
