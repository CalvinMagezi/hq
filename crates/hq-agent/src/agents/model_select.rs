//! Per-child model selection: which model a child runs on, and why.
//!
//! With a `backends:` chain configured, every provider handed to a child is
//! pinned to its backend's declared model, so writing an override into the
//! child's session config alone would never reach the wire. An override is
//! therefore resolved to a declared backend and that backend's own provider.

use std::sync::Arc;

use hq_llm::provider::LlmProvider;
use serde::{Deserialize, Serialize};

use super::types::ChildRequest;
use crate::backend::BackendRegistry;

const MAX_MODEL_ID_LEN: usize = 128;
const UNUSABLE_MODEL_LABEL: &str = "(unusable model id)";

/// Where a child's effective model came from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelSource {
    /// The parent session's model, unchanged.
    #[default]
    Inherited,
    /// The role's router alias (coder, planner, explorer, verifier).
    RoleAlias,
    /// An explicit per-child `model` argument.
    Override,
}

/// What to do when a requested model cannot be used. Never a silent switch:
/// the default rejects, and `Inherit` is recorded in `fallback_from`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnModelUnavailable {
    #[default]
    Reject,
    Inherit,
}

/// The model and backend a child actually ran on, for the tool result.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EffectiveModel {
    pub model: String,
    /// Declared backend name, or `None` when the legacy router picks.
    #[serde(default)]
    pub backend: Option<String>,
    pub source: ModelSource,
    /// The rejected request, set only when `Inherit` replaced it.
    #[serde(default)]
    pub fallback_from: Option<String>,
}

/// A resolved selection: what to report and what to apply to the child.
#[derive(Clone, Default)]
pub(super) struct ModelPlan {
    pub effective: EffectiveModel,
    /// Replaces the child session's model when set.
    pub config_model: Option<String>,
    /// Replaces the service provider when set (the override backend's own).
    pub provider: Option<Arc<dyn LlmProvider>>,
}

impl std::fmt::Debug for ModelPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelPlan")
            .field("effective", &self.effective)
            .field("config_model", &self.config_model)
            .finish()
    }
}

/// Reject ids that are oversized, contain whitespace/control characters, or
/// look like a credential. The id is never echoed back in the reason.
fn check_model_id(model: &str) -> Result<(), String> {
    let malformed = model.len() > MAX_MODEL_ID_LEN
        || model.chars().any(|c| c.is_whitespace() || c.is_control());
    if malformed {
        return Err("model id is malformed (whitespace, control characters or too long)".into());
    }
    if hq_core::redact::redact_secrets(model) != model {
        return Err("model id looks like a credential and was not used".into());
    }
    Ok(())
}

fn primary_label(registry: Option<&BackendRegistry>) -> Option<String> {
    registry.map(|r| r.primary_name().to_string())
}

fn inherited(parent_model: &str, registry: Option<&BackendRegistry>) -> ModelPlan {
    ModelPlan {
        effective: EffectiveModel {
            model: parent_model.to_string(),
            backend: primary_label(registry),
            source: ModelSource::Inherited,
            fallback_from: None,
        },
        ..Default::default()
    }
}

fn role_alias_plan(alias: &str, registry: Option<&BackendRegistry>) -> ModelPlan {
    ModelPlan {
        effective: EffectiveModel {
            model: alias.to_string(),
            backend: primary_label(registry),
            source: ModelSource::RoleAlias,
            fallback_from: None,
        },
        config_model: Some(alias.to_string()),
        provider: None,
    }
}

/// Resolve an explicit override against the configured backends.
fn override_plan(
    model: &str,
    registry: Option<&BackendRegistry>,
    external: Option<&str>,
) -> Result<ModelPlan, String> {
    let Some(registry) = registry else {
        // Legacy router: it resolves aliases and ids itself and cannot be
        // enumerated, so only the id's shape is checked before dispatch.
        return Ok(ModelPlan {
            effective: EffectiveModel {
                model: model.to_string(),
                backend: None,
                source: ModelSource::Override,
                fallback_from: None,
            },
            config_model: Some(model.to_string()),
            provider: None,
        });
    };
    let Some((name, backend)) = registry.resolve_model(model) else {
        return Err(format!(
            "model '{model}' is not served by any configured backend. Available: {}",
            registry.model_listing()
        ));
    };
    if let Some(ext) = external.filter(|e| *e != name) {
        return Err(format!(
            "model '{model}' is served by backend '{name}' but this child runs on '{ext}'; \
             name backend '{name}' or drop the model"
        ));
    }
    if !backend.capabilities().tools {
        return Err(format!(
            "backend '{name}' cannot drive a tool loop, so it cannot serve a child agent"
        ));
    }
    let Some(pinned) = backend.pinned_model() else {
        return Err(format!("backend '{name}' declares no model to select"));
    };
    Ok(ModelPlan {
        effective: EffectiveModel {
            model: pinned.clone(),
            backend: Some(name),
            source: ModelSource::Override,
            fallback_from: None,
        },
        config_model: Some(pinned),
        provider: if external.is_some() {
            None
        } else {
            backend.utility_provider()
        },
    })
}

fn select(
    requested: Option<&str>,
    role_alias: Option<&'static str>,
    parent_model: &str,
    registry: Option<&BackendRegistry>,
    external: Option<&str>,
) -> Result<ModelPlan, String> {
    let Some(model) = requested else {
        return Ok(match role_alias.filter(|_| external.is_none()) {
            Some(alias) => role_alias_plan(alias, registry),
            None => inherited(parent_model, registry),
        });
    };
    check_model_id(model)?;
    if crate::subagent::is_role_alias(model) && external.is_none() {
        return Ok(role_alias_plan(model, registry));
    }
    override_plan(model, registry, external)
}

/// Decide the child's model before dispatch. `external` is the backend name
/// when the child runs on an external backend (which picks its own model).
pub(super) fn resolve_child_model(
    req: &ChildRequest,
    parent_model: &str,
    registry: Option<&BackendRegistry>,
    external: Option<&str>,
) -> Result<ModelPlan, String> {
    let requested = req
        .model
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty());
    let role_alias = crate::subagent::role_to_model_alias(req.role());
    match select(requested, role_alias, parent_model, registry, external) {
        Ok(plan) => Ok(plan),
        Err(_) if req.on_model_unavailable == OnModelUnavailable::Inherit => {
            let mut plan = inherited(parent_model, registry);
            let shown = requested.filter(|m| check_model_id(m).is_ok());
            plan.effective.fallback_from = Some(shown.unwrap_or(UNUSABLE_MODEL_LABEL).to_string());
            Ok(plan)
        }
        Err(reason) => Err(hq_core::redact::redact_secrets(&reason)),
    }
}
