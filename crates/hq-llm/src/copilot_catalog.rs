//! The live model catalog of the Copilot subscription behind HQ, in full detail.
//!
//! `GET /models` lists every model the account can see, with limits, capability flags, the
//! endpoints each model speaks and its policy state. A model appearing here does not prove HQ can
//! call it: that also needs a backend entry in the config that serves it.

use crate::copilot::{
    COPILOT_BASE_URL, COPILOT_INTEGRATION_ID, EDITOR_VERSION, cached_session_token,
};
use crate::provider::LlmError;
use serde::Serialize;
use serde_json::{Map, Value};

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ModelLimits {
    pub context_window: Option<u64>,
    pub max_prompt: Option<u64>,
    pub max_output: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct CopilotModelInfo {
    pub id: String,
    pub name: Option<String>,
    pub vendor: Option<String>,
    pub family: Option<String>,
    pub version: Option<String>,
    /// `chat`, `embeddings` and so on, from `capabilities.type`.
    pub kind: Option<String>,
    pub preview: bool,
    pub model_picker_enabled: bool,
    pub policy_state: Option<String>,
    pub limits: ModelLimits,
    pub tokenizer: Option<String>,
    /// Capability flags such as `tool_calls`, `vision`, `streaming`, `parallel_tool_calls`.
    pub supports: Map<String, Value>,
    /// Wire endpoints the model accepts, for example `/chat/completions`, `/responses`, `/v1/messages`.
    pub supported_endpoints: Vec<String>,
    pub billing: Option<Value>,
}

fn text(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_string)
}

fn limits_of(entry: &Value) -> ModelLimits {
    let limits = entry.pointer("/capabilities/limits");
    let num = |k: &str| limits.and_then(|l| l.get(k)).and_then(Value::as_u64);
    ModelLimits {
        context_window: num("max_context_window_tokens"),
        max_prompt: num("max_prompt_tokens"),
        max_output: num("max_output_tokens"),
    }
}

/// One catalog entry in a stable shape. Missing fields stay `None` rather than failing the entry.
pub fn summarize_entry(entry: &Value) -> Option<CopilotModelInfo> {
    let id = text(entry, "id")?;
    let supports = entry
        .pointer("/capabilities/supports")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let supported_endpoints = entry
        .get("supported_endpoints")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Some(CopilotModelInfo {
        id,
        name: text(entry, "name"),
        vendor: text(entry, "vendor"),
        family: entry
            .pointer("/capabilities/family")
            .and_then(Value::as_str)
            .map(str::to_string),
        version: text(entry, "version"),
        kind: entry
            .pointer("/capabilities/type")
            .and_then(Value::as_str)
            .map(str::to_string),
        preview: entry
            .get("preview")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        model_picker_enabled: entry
            .get("model_picker_enabled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        policy_state: entry
            .pointer("/policy/state")
            .and_then(Value::as_str)
            .map(str::to_string),
        limits: limits_of(entry),
        tokenizer: entry
            .pointer("/capabilities/tokenizer")
            .and_then(Value::as_str)
            .map(str::to_string),
        supports,
        supported_endpoints,
        billing: entry.get("billing").cloned(),
    })
}

/// Every parseable entry of a `GET /models` body, in the order Copilot returned them.
pub fn summarize_catalog(body: &Value) -> Vec<CopilotModelInfo> {
    body.get("data")
        .and_then(Value::as_array)
        .map(|items| items.iter().filter_map(summarize_entry).collect())
        .unwrap_or_default()
}

/// Fetch the live catalog, both the summaries and the raw entries (for full detail on request).
pub async fn fetch_live_catalog() -> Result<(Vec<CopilotModelInfo>, Vec<Value>), LlmError> {
    let session = cached_session_token(false).await?;
    let api_base = session.api_base.as_deref().unwrap_or(COPILOT_BASE_URL);
    let resp = crate::http::SHARED_HTTP_CLIENT
        .get(format!("{api_base}/models"))
        .header("Authorization", format!("Bearer {}", session.jwt))
        .header("Editor-Version", EDITOR_VERSION)
        .header("Copilot-Integration-Id", COPILOT_INTEGRATION_ID)
        .send()
        .await
        .map_err(|e| LlmError::Network(format!("Copilot /models request failed: {e}")))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(LlmError::Other(anyhow::anyhow!(
            "Copilot /models returned HTTP {status}"
        )));
    }
    let body: Value = resp
        .json()
        .await
        .map_err(|e| LlmError::Other(anyhow::anyhow!("Copilot /models body was not JSON: {e}")))?;
    let raw = body
        .get("data")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok((summarize_catalog(&body), raw))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn body() -> Value {
        json!({"data": [
            {
                "id": "gpt-6-astra", "name": "GPT-6 Astra", "vendor": "OpenAI", "version": "gpt-6-astra",
                "preview": false, "model_picker_enabled": true,
                "policy": {"state": "enabled"},
                "billing": {"multiplier": 1.0},
                "capabilities": {
                    "family": "gpt-6", "type": "chat", "tokenizer": "o200k_base",
                    "limits": {"max_context_window_tokens": 400000, "max_prompt_tokens": 272000, "max_output_tokens": 128000},
                    "supports": {"tool_calls": true, "vision": true, "streaming": true}
                },
                "supported_endpoints": ["/responses", "/chat/completions"]
            },
            {"id": "text-embedding-3-small", "capabilities": {"type": "embeddings"}},
            {"object": "model"}
        ]})
    }

    #[test]
    fn a_full_entry_keeps_limits_flags_endpoints_and_policy() {
        let all = summarize_catalog(&body());
        assert_eq!(all.len(), 2, "an entry without an id is skipped");
        let m = &all[0];
        assert_eq!(m.id, "gpt-6-astra");
        assert_eq!(m.limits.max_prompt, Some(272000));
        assert_eq!(m.limits.context_window, Some(400000));
        assert_eq!(m.supports.get("tool_calls"), Some(&json!(true)));
        assert_eq!(
            m.supported_endpoints,
            vec!["/responses", "/chat/completions"]
        );
        assert_eq!(m.policy_state.as_deref(), Some("enabled"));
        assert!(m.model_picker_enabled);
    }

    #[test]
    fn a_sparse_entry_still_summarizes_with_defaults() {
        let m = &summarize_catalog(&body())[1];
        assert_eq!(m.kind.as_deref(), Some("embeddings"));
        assert!(!m.preview && !m.model_picker_enabled);
        assert!(m.supports.is_empty() && m.supported_endpoints.is_empty());
        assert_eq!(m.limits.max_prompt, None);
    }

    #[test]
    fn a_body_without_data_is_empty_not_an_error() {
        assert!(summarize_catalog(&json!({"error": "x"})).is_empty());
    }
}
