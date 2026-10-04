//! Model control for HQ's own session: read the Copilot subscription's live catalog, and switch
//! the primary backend the way the chat `model` command does.

use anyhow::{Result, bail};
use async_trait::async_trait;
use hq_core::config::{HqConfig, copilot_active};
use hq_core::config::model_switch::{
    ModelResolution, declare_backend, resolve_model_arg, set_primary_backend,
};
use hq_core::config::{BackendEntry, BackendKind, WireApi, resolve_session_model};
use hq_llm::copilot_catalog::{CopilotModelInfo, fetch_live_catalog};
use serde_json::{Value, json};

use crate::copilot_credits;
use crate::registry::HqTool;

pub fn create_model_tools() -> Vec<Box<dyn HqTool>> {
    vec![
        Box::new(CopilotModelsTool),
        Box::new(CopilotCreditsTool),
        Box::new(ModelSwitchTool),
    ]
}

fn backend_view(entry: &BackendEntry, primary: &str) -> Value {
    json!({
        "name": entry.name,
        "kind": serde_json::to_value(entry.kind).unwrap_or(Value::Null),
        "model": entry.model,
        "effort": entry.effort,
        "enabled": entry.enabled,
        "primary": entry.name == primary,
    })
}

fn declared_backend<'a>(config: &'a HqConfig, model_id: &str) -> Option<&'a BackendEntry> {
    let id = model_id.to_lowercase();
    config
        .backends
        .backends
        .iter()
        .find(|b| b.model.as_deref().is_some_and(|m| m.to_lowercase() == id))
}

fn matches_filter(m: &CopilotModelInfo, model: Option<&str>, kind: Option<&str>) -> bool {
    let model_ok = model.is_none_or(|needle| {
        let n = needle.to_lowercase();
        m.id.to_lowercase().contains(&n)
            || m.name
                .as_deref()
                .is_some_and(|x| x.to_lowercase().contains(&n))
    });
    let kind_ok = kind.is_none_or(|k| m.kind.as_deref().is_some_and(|x| x.eq_ignore_ascii_case(k)));
    model_ok && kind_ok
}

pub struct CopilotModelsTool;

#[async_trait]
impl HqTool for CopilotModelsTool {
    fn name(&self) -> &str {
        "copilot_models"
    }

    fn description(&self) -> &str {
        "List the models the GitHub Copilot subscription behind HQ can see right now, fetched live \
         from Copilot's /models endpoint: id, name, vendor, family, kind, preview flag, policy \
         state, context and output limits, capability flags (tools, vision, streaming), supported \
         wire endpoints and billing. Each model is cross-checked against HQ's backend chain \
         (`declared_backend`): a model in the catalog can only be used by HQ once a backend entry \
         serves it. Filter with `model` (id or name substring) or `kind`; `detail` adds the raw \
         catalog entry."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "model": { "type": "string", "description": "Only models whose id or name contains this" },
                "kind": { "type": "string", "description": "Only this capability type, e.g. chat or embeddings" },
                "detail": { "type": "boolean", "description": "Include each model's raw catalog entry" }
            }
        })
    }

    fn category(&self) -> &str {
        "config"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let filter_model = args.get("model").and_then(Value::as_str);
        let filter_kind = args.get("kind").and_then(Value::as_str);
        let detail = args.get("detail").and_then(Value::as_bool).unwrap_or(false);
        let (summaries, raw) = match fetch_live_catalog().await {
            Ok(pair) => pair,
            Err(e) => bail!(
                "could not read the Copilot catalog: {e}. This needs a Copilot token \
                 (COPILOT_GITHUB_TOKEN, GH_TOKEN or GITHUB_TOKEN, or an authenticated gh) on the machine HQ runs on."
            ),
        };
        let config = HqConfig::load().unwrap_or_default();
        let primary = config.backends.primary.clone();
        let total = summaries.len();
        let models: Vec<Value> = summaries
            .iter()
            .zip(raw.iter().chain(std::iter::repeat(&Value::Null)))
            .filter(|(m, _)| matches_filter(m, filter_model, filter_kind))
            .map(|(m, entry)| {
                let backend = declared_backend(&config, &m.id);
                let mut view = serde_json::to_value(m).unwrap_or(Value::Null);
                view["declared_backend"] = json!(backend.map(|b| &b.name));
                view["backend_enabled"] = json!(backend.map(|b| b.enabled));
                view["is_primary"] = json!(backend.is_some_and(|b| b.name == primary));
                if detail {
                    view["raw"] = entry.clone();
                }
                view
            })
            .collect();
        Ok(json!({
            "source": "live GET /models on the Copilot API",
            "checked_at": chrono::Utc::now().to_rfc3339(),
            "catalog_size": total,
            "returned": models.len(),
            "active_model": resolve_session_model(&config),
            "backend_chain": config.backends.backends.iter().map(|b| backend_view(b, &primary)).collect::<Vec<_>>(),
            "models": models,
        }))
    }
}

pub struct CopilotCreditsTool;

#[async_trait]
impl HqTool for CopilotCreditsTool {
    fn name(&self) -> &str {
        "copilot_credits"
    }

    fn description(&self) -> &str {
        "Show the Copilot credit balance for the account HQ runs on: identity (login, plan, sku), \
         credits left, total, used, percent, reset date, and the burn rate (1h, 6h, 24h), projected \
         use at reset and projected exhaustion from stored samples. `refresh` (default true) reads \
         the live balance and stores a sample first. A `note` says when the token is not a metered \
         plan or the read failed; no numbers are invented in that case."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "refresh": { "type": "boolean", "description": "Fetch and store a fresh sample first (default true)" }
            }
        })
    }

    fn category(&self) -> &str {
        "config"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let refresh = args.get("refresh").and_then(Value::as_bool).unwrap_or(true);
        let config = HqConfig::load().unwrap_or_default();
        if !copilot_active(&config) {
            return Ok(json!({ "active": false, "note": "No Copilot backend is enabled in the backend chain." }));
        }
        let db = hq_db::Database::open(&config.db_path())?;
        let fetched = if refresh {
            copilot_credits::sample_now(&db).await
        } else {
            hq_llm::copilot_usage::fetch_quota().await.map_err(|e| anyhow::anyhow!("{e}"))
        };
        let quota = match fetched {
            Ok(q) => q,
            Err(e) => {
                return Ok(json!({ "active": true, "note": format!("Could not read the Copilot balance: {e}. No numbers are shown.") }));
            }
        };
        Ok(credits_json(&db, &quota)?)
    }
}

fn credits_json(db: &hq_db::Database, quota: &hq_llm::copilot_usage::CopilotQuota) -> Result<Value> {
    let identity = json!({ "login": quota.login, "plan": quota.plan, "sku": quota.sku });
    if !quota.is_metered() {
        return Ok(json!({ "active": true, "account": identity, "note": copilot_credits::NOT_METERED_NOTE }));
    }
    let view = copilot_credits::burn_view(db, quota, chrono::Utc::now(), 0)?;
    Ok(json!({
        "active": true,
        "account": identity,
        "credits_left": quota.remaining,
        "credits_total": quota.entitlement,
        "credits_used": quota.credits_used,
        "percent_remaining": quota.percent_remaining,
        "reset_at": quota.reset_at,
        "overage_permitted": quota.overage_permitted,
        "checked_at": quota.fetched_at,
        "burn": view.burn,
    }))
}

const COPILOT_HOST: &str = "githubcopilot.com";

fn is_copilot_backend(entry: &BackendEntry) -> bool {
    entry.kind == BackendKind::GithubCopilotApi
        || entry
            .endpoint
            .as_deref()
            .is_some_and(|e| e.contains(COPILOT_HOST))
}

/// Which wire a catalog model needs, from the endpoints Copilot says it accepts.
fn needed_wire(model: &CopilotModelInfo) -> NeededWire {
    let has = |p: &str| model.supported_endpoints.iter().any(|e| e.ends_with(p));
    if has("/v1/messages") {
        NeededWire::Messages
    } else if has("/responses") && !has("/chat/completions") {
        NeededWire::Responses
    } else {
        NeededWire::Chat
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum NeededWire {
    Messages,
    Responses,
    Chat,
}

/// A new backend for a catalog model, cloned from an existing Copilot-backed entry so it reuses that
/// entry's endpoint and credential. `None` when the chain has no Copilot entry that can carry it.
fn backend_for_catalog_model(
    chain: &[BackendEntry],
    model: &CopilotModelInfo,
) -> Option<BackendEntry> {
    let wire = needed_wire(model);
    let template = match wire {
        NeededWire::Messages => chain
            .iter()
            .find(|b| b.kind == BackendKind::GithubCopilotApi && b.enabled),
        NeededWire::Responses | NeededWire::Chat => {
            let want = if wire == NeededWire::Responses {
                WireApi::Responses
            } else {
                WireApi::ChatCompletions
            };
            let copilot = || {
                chain.iter().filter(|b| {
                    b.kind == BackendKind::OpenaiCompatible && is_copilot_backend(b) && b.enabled
                })
            };
            copilot()
                .find(|b| b.wire == want)
                .or_else(|| copilot().next())
        }
    }?;
    let mut entry = template.clone();
    let taken = chain.iter().any(|b| b.name == model.id);
    entry.name = if taken {
        format!("{}-copilot", model.id)
    } else {
        model.id.clone()
    };
    entry.model = Some(model.id.clone());
    entry.effort = None;
    entry.enabled = true;
    if entry.kind == BackendKind::OpenaiCompatible {
        entry.wire = if wire == NeededWire::Responses {
            WireApi::Responses
        } else {
            WireApi::ChatCompletions
        };
    }
    Some(entry)
}

/// Declare `name` from the live catalog when it is enabled there and the chain can carry it.
async fn declare_from_catalog(config: &HqConfig, name: &str) -> Result<BackendEntry> {
    let (models, _) = fetch_live_catalog()
        .await
        .map_err(|e| anyhow::anyhow!("no backend matches `{name}` and the Copilot catalog could not be read to declare one: {e}"))?;
    let needle = name.to_lowercase();
    let Some(model) = models.iter().find(|m| m.id.to_lowercase() == needle) else {
        bail!(
            "`{name}` is neither a declared backend nor in the live Copilot catalog. Run copilot_models to see what exists."
        );
    };
    if model
        .policy_state
        .as_deref()
        .is_some_and(|s| s != "enabled")
    {
        bail!(
            "`{}` is in the catalog but its policy state is {:?}, so this subscription cannot use it.",
            model.id,
            model.policy_state
        );
    }
    if model.kind.as_deref().is_some_and(|k| k != "chat") {
        bail!(
            "`{}` is a {:?} model, not a chat model.",
            model.id,
            model.kind
        );
    }
    backend_for_catalog_model(&config.backends.backends, model).ok_or_else(|| {
        anyhow::anyhow!(
            "`{}` is available (endpoints {:?}) but the backend chain has no Copilot entry to reuse for it. Declare one Copilot backend first.",
            model.id, model.supported_endpoints
        )
    })
}

pub struct ModelSwitchTool;

#[async_trait]
impl HqTool for ModelSwitchTool {
    fn name(&self) -> &str {
        "model_switch"
    }

    fn description(&self) -> &str {
        "Show HQ's backend chain, or switch the primary backend by backend name or model id (the \
         same as the chat `model <name>` command). The switch is global and applies from the next \
         message; switching back is the same call with the previous name. The model must already \
         be a declared backend, or a chat model that `copilot_models` lists as enabled: then a backend \
         is declared for it automatically by reusing an existing Copilot backend's endpoint and \
         credential (the config is backed up first). Only use it when the user asks for a model change."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "model": { "type": "string", "description": "Backend name or model id. Omit to list the chain." }
            }
        })
    }

    fn category(&self) -> &str {
        "config"
    }

    fn requires_live_user_turn(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let config = HqConfig::load().unwrap_or_default();
        let before = resolve_session_model(&config);
        let chain: Vec<Value> = config
            .backends
            .backends
            .iter()
            .map(|b| backend_view(b, &config.backends.primary))
            .collect();
        let Some(name) = args
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
        else {
            return Ok(
                json!({ "active_model": before, "primary": config.backends.primary, "chain": chain }),
            );
        };
        let mut declared = None;
        let backend = match resolve_model_arg(&config.backends, name) {
            ModelResolution::Backend(b) => b,
            ModelResolution::NoMatch(_) => {
                let entry = declare_from_catalog(&config, name).await?;
                declare_backend(&entry)?;
                declared = Some(json!({
                    "name": entry.name,
                    "kind": serde_json::to_value(entry.kind).unwrap_or(Value::Null),
                    "wire": serde_json::to_value(entry.wire).unwrap_or(Value::Null),
                    "endpoint": entry.endpoint,
                }));
                entry.name
            }
        };
        if backend == config.backends.primary {
            return Ok(json!({ "changed": false, "primary": backend, "active_model": before }));
        }
        let (model, _) = set_primary_backend(&backend)?;
        let after = resolve_session_model(&HqConfig::load().unwrap_or_default());
        Ok(json!({
            "changed": true,
            "previous_primary": config.backends.primary,
            "previous_model": before,
            "primary": backend,
            "model": model,
            "active_model": after,
            "applies": "globally, from the next message",
            "declared_new_backend": declared,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_llm::copilot_catalog::summarize_entry;

    fn info(id: &str, name: &str, kind: &str) -> CopilotModelInfo {
        summarize_entry(&json!({"id": id, "name": name, "capabilities": {"type": kind}})).unwrap()
    }

    #[test]
    fn filters_match_id_or_name_and_kind_case_insensitively() {
        let astra = info("gpt-6-astra", "GPT-6 Astra", "chat");
        let embed = info("text-embedding-3", "Embedding", "embeddings");
        assert!(matches_filter(&astra, Some("ASTRA"), None));
        assert!(matches_filter(&astra, Some("gpt-6"), Some("Chat")));
        assert!(!matches_filter(&embed, None, Some("chat")));
        assert!(!matches_filter(&astra, Some("claude"), None));
        assert!(matches_filter(&embed, None, None));
    }

    #[test]
    fn model_switch_lists_the_chain_when_given_no_model() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let out = rt.block_on(ModelSwitchTool.execute(json!({}))).unwrap();
        assert!(out.get("active_model").is_some() && out["chain"].is_array());
    }

    fn chain() -> Vec<BackendEntry> {
        let entry =
            |name: &str, kind: BackendKind, endpoint: Option<&str>, wire: WireApi, model: &str| {
                BackendEntry {
                    name: name.into(),
                    kind,
                    endpoint: endpoint.map(str::to_string),
                    credential_env: Some("COPILOT_GITHUB_TOKEN".into()),
                    model: Some(model.into()),
                    effort: Some("high".into()),
                    wire,
                    enabled: true,
                }
            };
        vec![
            entry(
                "copilot",
                BackendKind::GithubCopilotApi,
                None,
                WireApi::ChatCompletions,
                "claude-sonnet-5",
            ),
            entry(
                "luna",
                BackendKind::OpenaiCompatible,
                Some("https://api.githubcopilot.com"),
                WireApi::Responses,
                "gpt-6-luna",
            ),
            entry(
                "deepseek",
                BackendKind::OpenaiCompatible,
                Some("https://api.deepseek.com/v1"),
                WireApi::ChatCompletions,
                "deepseek-flash",
            ),
        ]
    }

    fn catalog_model(id: &str, endpoints: &[&str]) -> CopilotModelInfo {
        summarize_entry(
            &json!({"id": id, "supported_endpoints": endpoints, "capabilities": {"type": "chat"}}),
        )
        .unwrap()
    }

    #[test]
    fn a_responses_only_gpt_model_clones_the_copilot_openai_entry_with_the_responses_wire() {
        let astra = catalog_model("gpt-6-astra", &["/responses"]);
        let entry = backend_for_catalog_model(&chain(), &astra).unwrap();
        assert_eq!(entry.name, "gpt-6-astra");
        assert_eq!(entry.model.as_deref(), Some("gpt-6-astra"));
        assert_eq!(entry.kind, BackendKind::OpenaiCompatible);
        assert_eq!(entry.wire, WireApi::Responses);
        assert_eq!(
            entry.endpoint.as_deref(),
            Some("https://api.githubcopilot.com")
        );
        assert_eq!(
            entry.credential_env.as_deref(),
            Some("COPILOT_GITHUB_TOKEN")
        );
        assert_eq!(
            entry.effort, None,
            "the template's effort setting is not inherited"
        );
    }

    #[test]
    fn a_messages_model_clones_the_copilot_api_entry_and_never_a_non_copilot_one() {
        let sonnet = catalog_model("claude-opus-5", &["/v1/messages", "/chat/completions"]);
        let entry = backend_for_catalog_model(&chain(), &sonnet).unwrap();
        assert_eq!(entry.kind, BackendKind::GithubCopilotApi);
        let only_deepseek = vec![chain().remove(2)];
        assert!(
            backend_for_catalog_model(
                &only_deepseek,
                &catalog_model("gpt-6-astra", &["/responses"])
            )
            .is_none()
        );
    }

    #[test]
    fn a_name_that_is_taken_gets_a_suffix() {
        let mut existing = chain();
        existing[2].name = "gpt-6-astra".into();
        let entry =
            backend_for_catalog_model(&existing, &catalog_model("gpt-6-astra", &["/responses"]))
                .unwrap();
        assert_eq!(entry.name, "gpt-6-astra-copilot");
    }

    #[test]
    fn the_switch_tool_needs_a_live_user_turn_and_the_catalog_tool_is_read_only() {
        assert!(ModelSwitchTool.requires_live_user_turn());
        assert!(CopilotModelsTool.is_read_only());
        assert!(!ModelSwitchTool.is_read_only());
    }
}
