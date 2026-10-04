//! The list_skills and load_skill tools.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{Value, json};
use std::path::PathBuf;

use super::parse::*;
use crate::registry::HqTool;

pub struct ListSkillsTool {
    skills_dir: PathBuf,
}

impl ListSkillsTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self { skills_dir }
    }
}

#[async_trait]
impl HqTool for ListSkillsTool {
    fn name(&self) -> &str {
        "list_skills"
    }

    fn description(&self) -> &str {
        "List all available skills with name, description, auto-load status, and hint keywords. \
         Use hints to determine which skills are relevant to the current task — \
         if any hint keyword appears in the user's message, load that skill with load_skill."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn category(&self) -> &str {
        "skills"
    }

    async fn execute(&self, _args: Value) -> Result<Value> {
        let skills = list_skills(&self.skills_dir);
        Ok(json!({ "skills": skills, "count": skills.len() }))
    }
}

// ─── LoadSkillTool ──────────────────────────────────────────────

pub struct LoadSkillTool {
    skills_dir: PathBuf,
    /// Optional telemetry sink. When set, every successful `load_skill` call is
    /// logged to `skill_invocations`, keyed by `session_id`. Outcomes are
    /// backfilled later by callers with signal (AIDC, coordinator).
    ///
    /// NOTE: the default MCP registry instantiates `LoadSkillTool` once, ahead
    /// of any specific session, so it uses `new` (no telemetry). Wiring
    /// session-scoped telemetry requires either a per-session tool registry
    /// or passing `session_id` through the `execute` args; both are larger
    /// refactors tracked as follow-up work. Callers that already know the
    /// session id (e.g. coordinator sub-agents, research-loop harness) can
    /// construct this with `with_telemetry` to get invocation logging today.
    telemetry: Option<(std::sync::Arc<hq_db::Database>, String)>,
}

impl LoadSkillTool {
    pub fn new(skills_dir: PathBuf) -> Self {
        Self {
            skills_dir,
            telemetry: None,
        }
    }

    /// Construct a variant that records every successful load to
    /// `skill_invocations` under the given session id.
    pub fn with_telemetry(
        skills_dir: PathBuf,
        db: std::sync::Arc<hq_db::Database>,
        session_id: impl Into<String>,
    ) -> Self {
        Self {
            skills_dir,
            telemetry: Some((db, session_id.into())),
        }
    }
}

#[async_trait]
impl HqTool for LoadSkillTool {
    fn name(&self) -> &str {
        "load_skill"
    }

    /// Weak so vault-only relay sessions, which still get the catalog, keep it.
    fn tool_policy(&self) -> crate::registry::ToolPolicy {
        crate::registry::ToolPolicy::Weak
    }

    fn description(&self) -> &str {
        "Load a skill by name. Returns the full skill content (instructions)."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "name": { "type": "string", "description": "Skill name (directory name)" }
            },
            "required": ["name"]
        })
    }

    fn is_read_only(&self) -> bool {
        true
    }

    fn category(&self) -> &str {
        "skills"
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        {
            let name = args
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or_default();

            match parse_skill(&self.skills_dir, name) {
                Some(skill) => {
                    if let Some((db, session_id)) = &self.telemetry {
                        let _ = db.with_conn(|conn| {
                            hq_db::skill_invocations::log_invocation(
                                conn,
                                &skill.name,
                                session_id,
                                hq_db::skill_invocations::InvocationTrigger::LoadSkill,
                            )?;
                            Ok(())
                        });
                    }
                    Ok(json!({
                        "name": skill.name,
                        "description": skill.description,
                        "content": skill.content,
                        "auto_load": skill.auto_load,
                        "load_full": skill.load_full,
                        "next_skills": skill.next_skills,
                    }))
                }
                None => Ok(json!({ "error": format!("skill not found: {name}") })),
            }
        }
    }
}
