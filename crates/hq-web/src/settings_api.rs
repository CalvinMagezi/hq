//! Read-only view of the running HQ configuration for the web app's Settings page.
//!
//! The view is built field by field. Nothing is serialized wholesale, so a secret added
//! to `HqConfig` later cannot leak here by accident.

use crate::error::ApiError;
use axum::response::IntoResponse;
use hq_core::config::{HqConfig, resolve_session_model};
use serde_json::{Value, json};

/// Host part of a URL, so an endpoint can be shown without its path or credentials.
fn host_of(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let authority = rest.split(['/', '?', '#']).next().unwrap_or(rest);
    authority.rsplit('@').next().unwrap_or(authority).to_string()
}

fn key_presence(config: &HqConfig) -> Value {
    let present = |key: &Option<String>| key.as_deref().is_some_and(|k| !k.trim().is_empty());
    json!([
        {"name": "OpenRouter", "configured": present(&config.openrouter_api_key)},
        {"name": "Anthropic", "configured": present(&config.anthropic_api_key)},
        {"name": "Google AI", "configured": present(&config.google_ai_api_key)},
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

pub(crate) fn settings_view(config: &HqConfig) -> Value {
    let sandbox = serde_json::to_value(config.governance.bash.sandbox).unwrap_or(Value::Null);
    let hosts: Vec<&String> = config.herdr.hosts.keys().collect();
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
        "provider_keys": key_presence(config),
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
        "herdr": {
            "default_host": config.herdr.default_host,
            "hosts": hosts,
            "drive_new_watches": config.herdr.drive_new_watches,
            "driver_checkin_minutes": config.herdr.driver_checkin_minutes,
            "driver_nudge_budget": config.herdr.nudge_budget(),
            "driver_no_progress_limit": config.herdr.no_progress_limit(),
        },
        "integrations": {
            "remote_mcp": remote_mcp,
            "searxng_host": config.searxng_url.as_deref().map(host_of),
            "disk_watchdog_enabled": config.disk_watchdog.enabled,
        },
    })
}

pub(crate) async fn settings_handler() -> Result<impl IntoResponse, ApiError> {
    let config = tokio::task::spawn_blocking(HqConfig::load)
        .await
        .map_err(|e| ApiError::internal(format!("settings task failed: {e}")))?
        .map_err(|e| ApiError::internal(format!("could not read the HQ config: {e}")))?;
    Ok(axum::Json(settings_view(&config)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::RemoteMcpServer;

    #[test]
    fn the_view_never_contains_a_secret_value() {
        let mut config = HqConfig {
            openrouter_api_key: Some("sk-or-SECRETVALUE1".into()),
            anthropic_api_key: Some("sk-ant-SECRETVALUE2".into()),
            ..Default::default()
        };
        config.remote_mcp.push(RemoteMcpServer {
            name: "diagrams".into(),
            url: concat!("https://user:", "SECRETVALUE3@mcp.example.com/api/mcp?key=SECRETVALUE4").into(),
            api_key: Some("SECRETVALUE5".into()),
            live_user_turn_only: true,
        });
        let text = settings_view(&config).to_string();
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
        let view = settings_view(&config);
        let keys = view["provider_keys"].as_array().unwrap();
        let configured = |name: &str| keys.iter().find(|k| k["name"] == name).unwrap()["configured"].clone();
        assert_eq!(configured("OpenRouter"), json!(true));
        assert_eq!(configured("Groq"), json!(false));
        assert_eq!(configured("Anthropic"), json!(false));
    }

    #[test]
    fn the_active_model_follows_the_session_resolution_rule() {
        let mut config = HqConfig {
            default_model: "model-a".into(),
            ..Default::default()
        };
        assert_eq!(settings_view(&config)["model"]["active"], json!("model-a"));
        config.relay.model = Some("model-b".into());
        assert_eq!(settings_view(&config)["model"]["active"], json!("model-b"));
    }

    #[test]
    fn host_of_drops_credentials_path_and_query() {
        assert_eq!(host_of("https://u:p@host.example:8443/a/b?k=v"), "host.example:8443");
        assert_eq!(host_of("http://127.0.0.1:8080"), "127.0.0.1:8080");
    }
}
