//! Pre-bound agent tools the local model can call as "extra intelligence".
//!
//! Unlike the unified `spawn_subagents` tool (which takes `role`, `backend`, and
//! `model` as runtime arguments), each `AgentCallableTool` is registered once
//! with a fixed role and descriptive name. The local model picks one by
//! name (e.g. `call_code_reasoner`) rather than
//! reasoning about the role vocabulary.
//!
//! Under the hood every entry now routes through the same unified
//! [`AgentService`](crate::agents::AgentService) path as `spawn_subagents`: each
//! catalog entry is effectively a pre-bound single-child
//! [`ChildRequest`](crate::agents::ChildRequest) with a fixed role, run as a
//! [`ChildMode::Single`](crate::agents::ChildMode) plan. The model comes from
//! the role's router alias (see `subagent::role_to_model_alias`), so the
//! operator's router config decides which provider serves each specialist.

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use hq_core::types::{SubagentType, ToolResult, ToolResultContent};
use serde_json::{Value, json};

use crate::agents::{AgentService, ChildExecContext, ChildPlan, ChildRequest, ChildStatus};
use crate::tools::AgentTool;

/// Map a catalog role string to the shared [`SubagentType`] vocabulary.
fn role_to_agent_type(role: &str) -> SubagentType {
    match role {
        "coder" => SubagentType::Coder,
        "explorer" => SubagentType::Explorer,
        "planner" => SubagentType::Planner,
        "verifier" => SubagentType::Verifier,
        _ => SubagentType::General,
    }
}

/// A pre-bound agent-as-tool entry. Thin wrapper that builds a single-child
/// [`ChildPlan`] with role and model injected, and runs it through
/// [`AgentService`].
pub struct AgentCallableTool {
    pub name: &'static str,
    pub description: String,
    pub role: &'static str,
    pub bound_model: String,
    pub service: Arc<AgentService>,
    /// Parent correlation context (run id + envelope sink) so the specialist's
    /// events interleave with the parent run, matching `spawn_subagents`.
    pub ctx: ChildExecContext,
}

impl AgentCallableTool {
    /// Build the pre-bound single-child request: fixed role + model alias,
    /// in-process. `backend` is pinned explicitly to the in-process ("hq")
    /// backend. Without this, a role like "planner" that doesn't need local
    /// tools would hit `AgentService`'s auto-external policy and run on
    /// whatever default external backend is configured, silently discarding
    /// `bound_model` (model overrides only apply on the in-process path).
    fn build_request(&self, prompt: &str, context: &str) -> ChildRequest {
        let mut req = ChildRequest::new(self.name, prompt);
        req.agent_type = role_to_agent_type(self.role);
        req.backend = Some(crate::agents::INPROCESS_BACKEND.to_string());
        req.model = Some(self.bound_model.clone());
        if !context.is_empty() {
            req.context = Some(context.to_string());
        }
        req
    }
}

#[async_trait]
impl AgentTool for AgentCallableTool {
    fn name(&self) -> &str {
        self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["prompt"],
            "properties": {
                "prompt": {
                    "type": "string",
                    "description": "The task instruction for the specialist agent."
                },
                "context": {
                    "type": "string",
                    "description": "Optional extra context to pass to the specialist."
                }
            }
        })
    }

    async fn execute(&self, _id: &str, args: Value) -> Result<ToolResult> {
        let prompt = args
            .get("prompt")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required 'prompt' parameter"))?;
        let context = args.get("context").and_then(|v| v.as_str()).unwrap_or("");

        let req = self.build_request(prompt, context);
        let plan = ChildPlan::single(req);
        let mut outcomes = self.service.execute(plan, self.ctx.clone()).await;
        let outcome = outcomes.pop().ok_or_else(|| {
            anyhow::anyhow!("agent service returned no outcome for '{}'", self.name)
        })?;

        let details = json!({
            "specialist": self.name,
            "role": self.role,
            "model": self.bound_model,
            "backend": outcome.resolved_backend,
            "status": format!("{:?}", outcome.status).to_lowercase(),
        });

        let text = if outcome.status == ChildStatus::Completed {
            outcome.output
        } else {
            let reason = outcome
                .error
                .clone()
                .unwrap_or_else(|| outcome.output.clone());
            format!("Specialist ({}) did not complete: {reason}", self.name)
        };

        Ok(ToolResult {
            content: vec![ToolResultContent {
                r#type: "text".to_string(),
                text,
            }],
            details: Some(details),
            context_modifier: None,
        })
    }

    fn is_read_only(&self) -> bool {
        matches!(self.role, "explorer" | "verifier")
    }
}

/// One entry in the static catalog describing an escalation target.
struct CatalogEntry {
    name: &'static str,
    role: &'static str,
    /// Plain-language "when to use" text, stitched into the full description.
    when: &'static str,
}

/// The catalog. Intentionally small and curated.
const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        name: "call_code_reasoner",
        role: "coder",
        when: "tough refactors over 200 lines, tricky bugs, hairy type errors",
    },
    CatalogEntry {
        name: "call_planner_strong",
        role: "planner",
        when: "multi-step plans with constraints that need careful ordering",
    },
    CatalogEntry {
        name: "call_verifier_cheap",
        role: "verifier",
        when: "fast correctness checks and review passes; cheap, high volume ok",
    },
    CatalogEntry {
        name: "call_web_researcher",
        role: "explorer",
        when: "questions that need fresh information or recent events",
    },
];

/// Build the full set of pre-bound agent tools, sharing one [`AgentService`]
/// across all entries. The `ctx` threads parent correlation (run id + envelope
/// sink) into each specialist run.
pub fn build_catalog(service: Arc<AgentService>, ctx: ChildExecContext) -> Vec<Box<dyn AgentTool>> {
    CATALOG
        .iter()
        .map(|entry| {
            let description = format!(
                "Delegate to the {} specialist. Use when: {}.",
                entry.role, entry.when
            );
            Box::new(AgentCallableTool {
                name: entry.name,
                description,
                role: entry.role,
                bound_model: bound_model(entry.role),
                service: service.clone(),
                ctx: ctx.clone(),
            }) as Box<dyn AgentTool>
        })
        .collect()
}

/// Router alias for a catalog role. Every catalog role has one.
fn bound_model(role: &str) -> String {
    crate::subagent::role_to_model_alias(role)
        .unwrap_or("default")
        .to_string()
}

/// Names of every escalation tool in the catalog — useful for the ToolPolicy
/// filter when it needs to decide which tools are escalation-class vs. core.
pub fn catalog_tool_names() -> Vec<&'static str> {
    CATALOG.iter().map(|e| e.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use hq_llm::provider::{ChatRequest, ChatResponse, LlmError, LlmProvider, StreamChunk};
    use std::pin::Pin;
    use tokio_stream::Stream;

    /// Minimal `LlmProvider` used only to construct an `AgentService` for the
    /// backend-pinning regression test below; its `chat`/`chat_stream` are
    /// never actually invoked (the test only inspects `build_request`, it
    /// never calls `.execute()`).
    struct NoopProvider;

    #[async_trait]
    impl LlmProvider for NoopProvider {
        fn name(&self) -> &str {
            "noop"
        }

        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
            Err(LlmError::Auth {
                status: 500,
                message: "unused in this test".to_string(),
            }
            .into())
        }

        async fn chat_stream(
            &self,
            _request: &ChatRequest,
        ) -> anyhow::Result<Pin<Box<dyn Stream<Item = anyhow::Result<StreamChunk>> + Send>>>
        {
            Ok(Box::pin(tokio_stream::iter(vec![Ok(StreamChunk::Done)])))
        }
    }

    #[test]
    fn catalog_has_expected_entries() {
        let names: Vec<&str> = CATALOG.iter().map(|e| e.name).collect();
        assert!(names.contains(&"call_code_reasoner"));
        assert!(names.contains(&"call_planner_strong"));
        assert!(names.contains(&"call_verifier_cheap"));
        assert!(names.contains(&"call_web_researcher"));
    }

    #[test]
    fn every_catalog_role_has_a_router_alias() {
        for e in CATALOG {
            assert!(
                crate::subagent::role_to_model_alias(e.role).is_some(),
                "{} has no router alias for role {}",
                e.name,
                e.role
            );
        }
    }

    #[test]
    fn every_catalog_request_pins_the_in_process_backend() {
        // Regression: `req.backend` must always be explicit "hq" so a
        // no-local-tools role (currently only "planner") can never fall
        // through to `AgentService`'s auto-external policy, which would
        // silently discard `bound_model` and run on whatever default
        // external backend happens to be configured.
        for entry in CATALOG {
            let tool = AgentCallableTool {
                name: entry.name,
                description: entry.when.to_string(),
                role: entry.role,
                bound_model: bound_model(entry.role),
                service: Arc::new(AgentService::new(
                    Arc::new(NoopProvider),
                    std::env::temp_dir(),
                    vec![std::env::temp_dir()],
                    crate::session::SessionConfig::default(),
                    hq_core::types::SecurityProfile::Guarded,
                    0,
                    3,
                    std::time::Duration::from_secs(30),
                )),
                ctx: ChildExecContext::default(),
            };
            let req = tool.build_request("do the thing", "");
            assert_eq!(
                req.backend.as_deref(),
                Some(crate::agents::INPROCESS_BACKEND),
                "catalog entry '{}' must pin the in-process backend",
                entry.name
            );
            assert_eq!(req.model.as_deref(), Some(bound_model(entry.role).as_str()));
        }
    }

    #[test]
    fn catalog_names_are_kebab_safe() {
        for e in CATALOG {
            assert!(
                e.name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "tool name must be snake-case-safe: {}",
                e.name
            );
        }
    }

    #[test]
    fn catalog_names_list_matches_catalog_entries() {
        let names = catalog_tool_names();
        assert_eq!(names.len(), CATALOG.len());
        assert!(names.contains(&"call_code_reasoner"));
    }

    #[test]
    fn role_maps_to_expected_agent_type() {
        assert_eq!(role_to_agent_type("coder"), SubagentType::Coder);
        assert_eq!(role_to_agent_type("explorer"), SubagentType::Explorer);
        assert_eq!(role_to_agent_type("planner"), SubagentType::Planner);
        assert_eq!(role_to_agent_type("verifier"), SubagentType::Verifier);
        assert_eq!(role_to_agent_type("general"), SubagentType::General);
        assert_eq!(role_to_agent_type("nonsense"), SubagentType::General);
    }
}
