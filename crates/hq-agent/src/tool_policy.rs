//! Per-model tool-visibility policy.
//!
//! The same session machinery powers both local Gemma and cloud premium
//! models, but they shouldn't see the same tool set. Local Gemma gets a
//! curated "escalation toolkit" — core read/write + pre-bound specialists.
//! Cloud models get everything. Nested sub-agents see only core tools so
//! they can't recursively escalate.
//!
//! Named agents (e.g. "telegram_guest") can additionally restrict
//! tools by category or name via `AgentProfile`.

use crate::callable_agents::catalog_tool_names;
use crate::tools::AgentTool;
use std::path::Path;

/// Capability profile for a named agent — loaded from vault config or hardcoded defaults.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentProfile {
    /// Tool categories to deny entirely (e.g. "browser", "computer_use").
    /// Category names are compared case-insensitively.
    pub deny_categories: Vec<String>,
    /// Specific tool names to deny (overrides category allow).
    pub deny_tools: Vec<String>,
    /// Specific tool names to always allow (overrides category deny).
    pub allow_tools: Vec<String>,
}

/// Which policy preset to apply for a given `(model, depth)` pair.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Preset {
    /// Local small models (Gemma, Phi, Qwen3 via Ollama). Core tools + the
    /// escalation catalog; no raw spawn_subagent (redundant with pre-bound
    /// specialists).
    LocalGemma,
    /// Cloud premium models. Full tool access including spawn_subagent.
    /// Drops escalate_to_* tools to avoid self-referential calls.
    Cloud,
    /// Nested sub-agents. Core tools only — no escalation machinery so a
    /// child can't loop back up.
    Subagent,
    /// Named agent with an explicit capability profile.
    Named(AgentProfile),
}

impl Preset {
    /// Pick a preset based on the model id and the current subagent depth.
    pub fn for_model(model: &str, depth: u32) -> Self {
        if depth > 0 {
            return Preset::Subagent;
        }
        if is_local_small_model(model) {
            Preset::LocalGemma
        } else {
            Preset::Cloud
        }
    }
}

/// Returns true for models that run locally via Ollama and have limited
/// context / capability relative to cloud models.  The `relay` alias routes
/// to qwen3:14b, so we need to catch both the bare name and the alias.
fn is_local_small_model(model: &str) -> bool {
    let m = model.to_lowercase();
    let m = m.strip_prefix("ollama/").unwrap_or(&m);
    m.starts_with("gemma") || m.starts_with("phi") || m.starts_with("qwen") || m == "relay"
}

/// Filter a tool list according to the preset. Consumes and returns the
/// vector so the caller keeps ownership.
pub fn filter(tools: Vec<Box<dyn AgentTool>>, preset: Preset) -> Vec<Box<dyn AgentTool>> {
    let escalation: Vec<&'static str> = catalog_tool_names();
    tools
        .into_iter()
        .filter(|t| match &preset {
            Preset::LocalGemma => {
                // Local Gemma sees core + escalation catalog. Drop the raw
                // spawn_subagent / coordinator duplicates and the unified
                // spawn_subagents surface.
                t.name() != "spawn_subagent"
                    && t.name() != "coordinate"
                    && t.name() != "spawn_subagents"
            }
            Preset::Cloud => {
                // Cloud premium sees everything except the pre-bound
                // escalation specialists (they'd just loop back into cloud
                // anyway; the raw spawn_subagent is kept so cloud can still
                // fan out). Note: core tools like `generate_image` from
                // hq-tools are NOT in the escalation list, so they stay.
                !escalation.contains(&t.name())
            }
            Preset::Subagent => {
                // Subagents only get core tools (no escalation, no nesting).
                t.name() != "spawn_subagent"
                    && t.name() != "coordinate"
                    && t.name() != "spawn_subagents"
                    && !escalation.contains(&t.name())
            }
            Preset::Named(profile) => {
                let name = t.name();
                let cat = t.category();
                // Explicit allow wins over category deny.
                if profile.allow_tools.iter().any(|a| a == name) {
                    return true;
                }
                // Explicit name deny.
                if profile.deny_tools.iter().any(|d| d == name) {
                    return false;
                }
                // Category deny — normalize both sides to avoid case mismatch.
                if profile
                    .deny_categories
                    .iter()
                    .any(|c| c.to_lowercase() == cat.to_lowercase())
                {
                    return false;
                }
                true
            }
        })
        .collect()
}

/// Hardcoded default capability profile for a named agent.
/// Returns None if the agent has no restrictions (full Cloud access).
pub fn default_profile_for_agent(name: &str) -> Option<AgentProfile> {
    match name {
        "telegram_guest" => Some(AgentProfile {
            // Remote MCP servers are excluded even though a guest message is
            // a live turn (see LiveUserTurn): they act on the operator's own
            // external accounts, which only the operator should drive.
            deny_categories: vec![
                "computer_use".into(),
                "audio".into(),
                "meetings".into(),
                hq_tools::remote_mcp::REMOTE_MCP_CATEGORY.into(),
            ],
            deny_tools: vec![
                "skill_manage".into(),
                "bash".into(),
                "shell_exec".into(),
                "write".into(),
                "write_file".into(),
                "edit".into(),
                "edit_file".into(),
                "file_write".into(),
                "file_edit".into(),
                "vault_write_note".into(),
                "spawn_subagent".into(),
                "spawn_subagents".into(),
                "coordinate".into(),
                "dev_start".into(),
                "dev_cancel".into(),
                "native_code".into(),
            ],
            allow_tools: vec![],
        }),
        _ => None,
    }
}

/// Load per-agent capability overrides from `.vault/_system/AGENT_PROFILES.yaml`.
/// Returns None if the file doesn't exist or the agent isn't listed.
///
/// Format: `agent_name: { deny_categories: [...], allow_tools: [...], deny_tools: [...] }`
pub fn load_vault_profile(vault_path: &Path, agent_name: &str) -> Option<AgentProfile> {
    let path = vault_path.join("_system").join("AGENT_PROFILES.yaml");
    let content = std::fs::read_to_string(&path).ok()?;

    let map: serde_yaml::Value = serde_yaml::from_str(&content).ok()?;
    let agent = map.get(agent_name)?;

    let deny_categories = agent
        .get("deny_categories")
        .and_then(|v| v.as_sequence())
        .map(|s| {
            s.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let deny_tools = agent
        .get("deny_tools")
        .and_then(|v| v.as_sequence())
        .map(|s| {
            s.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    let allow_tools = agent
        .get("allow_tools")
        .and_then(|v| v.as_sequence())
        .map(|s| {
            s.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();

    Some(AgentProfile {
        deny_categories,
        deny_tools,
        allow_tools,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use async_trait::async_trait;
    use hq_core::types::ToolResult;
    use serde_json::Value;

    struct StubTool(&'static str);

    #[async_trait]
    impl AgentTool for StubTool {
        fn name(&self) -> &str {
            self.0
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn parameters(&self) -> Value {
            serde_json::json!({})
        }
        async fn execute(&self, _id: &str, _args: Value) -> Result<ToolResult> {
            unreachable!()
        }
    }

    fn tools(names: &[&'static str]) -> Vec<Box<dyn AgentTool>> {
        names
            .iter()
            .map(|n| Box::new(StubTool(n)) as Box<dyn AgentTool>)
            .collect()
    }

    #[test]
    fn local_gemma_drops_spawn_subagent_keeps_escalation() {
        let t = tools(&[
            "bash",
            "spawn_subagent",
            "call_code_reasoner",
            "generate_image",
        ]);
        let filtered = filter(t, Preset::LocalGemma);
        let names: Vec<_> = filtered.iter().map(|t| t.name().to_string()).collect();
        assert!(names.contains(&"bash".into()));
        assert!(names.contains(&"call_code_reasoner".into()));
        // Core hq-tools (e.g. generate_image) stay visible to local Gemma.
        assert!(names.contains(&"generate_image".into()));
        assert!(!names.contains(&"spawn_subagent".into()));
    }

    #[test]
    fn cloud_drops_escalation_specialists() {
        let t = tools(&[
            "bash",
            "spawn_subagent",
            "call_code_reasoner",
            "generate_image",
        ]);
        let filtered = filter(t, Preset::Cloud);
        let names: Vec<_> = filtered.iter().map(|t| t.name().to_string()).collect();
        assert!(names.contains(&"bash".into()));
        assert!(names.contains(&"spawn_subagent".into()));
        // core hq-tools like generate_image are not escalation; they stay.
        assert!(names.contains(&"generate_image".into()));
        assert!(!names.contains(&"call_code_reasoner".into()));
    }

    #[test]
    fn subagent_gets_core_only() {
        let t = tools(&[
            "bash",
            "spawn_subagent",
            "call_code_reasoner",
            "generate_image",
        ]);
        let filtered = filter(t, Preset::Subagent);
        let names: Vec<_> = filtered.iter().map(|t| t.name().to_string()).collect();
        // Core hq-tools (e.g. generate_image) are not escalation; they stay.
        assert!(names.contains(&"bash".into()));
        assert!(names.contains(&"generate_image".into()));
        assert!(!names.contains(&"spawn_subagent".into()));
        assert!(!names.contains(&"call_code_reasoner".into()));
    }

    #[test]
    fn preset_for_model_detects_local_small_models() {
        assert_eq!(
            Preset::for_model("ollama/gemma4:e4b", 0),
            Preset::LocalGemma
        );
        assert_eq!(Preset::for_model("gemma2-9b", 0), Preset::LocalGemma);
        assert_eq!(Preset::for_model("ollama/qwen3:14b", 0), Preset::LocalGemma);
        assert_eq!(Preset::for_model("qwen3:14b", 0), Preset::LocalGemma);
        assert_eq!(Preset::for_model("relay", 0), Preset::LocalGemma);
        assert_eq!(Preset::for_model("moonshotai/kimi-k2", 0), Preset::Cloud);
        assert_eq!(Preset::for_model("ollama/gemma4:e4b", 1), Preset::Subagent);
    }

    struct CatTool {
        name: &'static str,
        cat: &'static str,
    }

    #[async_trait]
    impl AgentTool for CatTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "cat-tool stub"
        }
        fn parameters(&self) -> Value {
            serde_json::json!({})
        }
        fn category(&self) -> &str {
            self.cat
        }
        async fn execute(&self, _id: &str, _args: Value) -> Result<ToolResult> {
            unreachable!()
        }
    }

    fn cat_tools(pairs: &[(&'static str, &'static str)]) -> Vec<Box<dyn AgentTool>> {
        pairs
            .iter()
            .map(|(n, c)| Box::new(CatTool { name: n, cat: c }) as Box<dyn AgentTool>)
            .collect()
    }

    #[test]
    fn telegram_guest_profile_denies_bash() {
        let profile = default_profile_for_agent("telegram_guest").unwrap();
        let t = tools(&["bash", "vault_search"]);
        let filtered = filter(t, Preset::Named(profile));
        let names: Vec<_> = filtered.iter().map(|t| t.name().to_string()).collect();
        assert!(!names.contains(&"bash".into()));
        assert!(names.contains(&"vault_search".into()));
    }

    #[test]
    fn named_deny_category_blocks_tool() {
        let profile = AgentProfile {
            deny_categories: vec!["browser".into()],
            deny_tools: vec![],
            allow_tools: vec![],
        };
        let t = cat_tools(&[("navigate", "browser"), ("bash", "shell")]);
        let filtered = filter(t, Preset::Named(profile));
        let names: Vec<_> = filtered.iter().map(|t| t.name().to_string()).collect();
        assert!(
            !names.contains(&"navigate".into()),
            "browser tool should be filtered out"
        );
        assert!(
            names.contains(&"bash".into()),
            "shell tool should pass through"
        );
    }

    #[test]
    fn named_allow_tool_overrides_category_deny() {
        let profile = AgentProfile {
            deny_categories: vec!["browser".into()],
            deny_tools: vec![],
            allow_tools: vec!["safe_browser_tool".into()],
        };
        let t = cat_tools(&[
            ("safe_browser_tool", "browser"),
            ("dangerous_browser_tool", "browser"),
        ]);
        let filtered = filter(t, Preset::Named(profile));
        let names: Vec<_> = filtered.iter().map(|t| t.name().to_string()).collect();
        assert!(
            names.contains(&"safe_browser_tool".into()),
            "explicitly allowed tool must pass"
        );
        assert!(
            !names.contains(&"dangerous_browser_tool".into()),
            "non-allowed browser tool must be blocked"
        );
    }

    #[test]
    fn named_deny_tool_by_name() {
        let profile = AgentProfile {
            deny_categories: vec![],
            deny_tools: vec!["dangerous".into()],
            allow_tools: vec![],
        };
        let t = cat_tools(&[("dangerous", "shell"), ("safe", "shell")]);
        let filtered = filter(t, Preset::Named(profile));
        let names: Vec<_> = filtered.iter().map(|t| t.name().to_string()).collect();
        assert!(
            !names.contains(&"dangerous".into()),
            "deny-listed tool must be filtered out"
        );
        assert!(
            names.contains(&"safe".into()),
            "non-listed tool must pass through"
        );
    }

    #[test]
    fn vault_and_claude_code_tools_survive_cloud_filter() {
        // Cloud preset (used by Qwen3/relay at depth=0) must never strip vault
        // or claude_code tools — only escalation-catalog entries are dropped.
        let t = tools(&[
            "vault_search",
            "vault_read",
            "vault_context",
            "claude_code_spawn",
            "claude_code_status",
            "spawn_subagent",
            "call_code_reasoner",
        ]);
        let filtered = filter(t, Preset::Cloud);
        let names: Vec<_> = filtered.iter().map(|t| t.name().to_string()).collect();
        assert!(names.contains(&"vault_search".into()));
        assert!(names.contains(&"vault_read".into()));
        assert!(names.contains(&"vault_context".into()));
        assert!(names.contains(&"claude_code_spawn".into()));
        assert!(names.contains(&"claude_code_status".into()));
        assert!(names.contains(&"spawn_subagent".into()));
        assert!(!names.contains(&"call_code_reasoner".into()));
    }
}
