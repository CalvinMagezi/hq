//! Skill hooks around a session turn: auto-loading, reminders, and review.

use anyhow::Result;
use hq_core::types::{SessionResult, ToolCall};

use super::AgentSession;

/// Argument bytes scanned per tool call for skill hints, so a huge payload stays cheap.
const MAX_HINT_SCAN_BYTES: usize = 2_000;
/// Calls that already are skill handling, so a hint in them is not a missed skill.
const SKILL_TOOLS: &[&str] = &["load_skill", "list_skills", "skill_manage"];

impl AgentSession {
    /// Nudge toward a catalog skill whose hint shows up in a tool call the
    /// agent made without loading it. At most once per skill per session.
    pub(super) fn skill_reminders(&mut self, tool_calls: &[ToolCall]) -> Vec<String> {
        let Some(index) = self.skill_index.clone() else {
            return Vec::new();
        };
        let mut reminders = Vec::new();
        for tc in tool_calls {
            if tc.name == "load_skill"
                && let Some(name) = tc.arguments.get("name").and_then(|v| v.as_str())
            {
                self.skills_in_play.insert(name.to_string());
            }
            if SKILL_TOOLS.contains(&tc.name.as_str()) {
                continue;
            }
            let args = tc.arguments.to_string();
            let end = args.floor_char_boundary(MAX_HINT_SCAN_BYTES);
            let text = format!("{} {}", tc.name, &args[..end]).to_lowercase();
            for name in index.matching(&text) {
                if self.skills_in_play.insert(name.to_string()) {
                    reminders.push(format!(
                        "Skill {name} covers this; call load_skill before continuing"
                    ));
                }
            }
        }
        reminders
    }

    /// Review the session for skill improvements: always after enough tool
    /// calls, and after a short turn when a skill was loaded recently.
    pub(super) fn review_skills_async(&mut self) {
        let since = self.tool_call_count.saturating_sub(self.skill_review_mark);
        let (agent, enabled) = (&self.config.agent_name, self.config.background_review);
        let full = crate::skill_review::due(agent, enabled, since);
        if !full && !crate::skill_review::light_due(agent, enabled, since) {
            return;
        }
        let Some(vault_path) = self.vault_path.clone() else {
            return;
        };
        // Only the full path resets, so skipped light reviews still add up to one.
        if full {
            self.skill_review_mark = self.tool_call_count;
        }
        let session_id = self.session_id.clone();
        let messages = self.messages.clone();
        let db = self.telemetry_db.clone();
        tokio::spawn(async move {
            match crate::skill_review::run(&vault_path, &session_id, &messages, db.as_deref(), full)
                .await
            {
                Ok(applied) => {
                    for a in applied {
                        tracing::info!(skill = %a.name, change = ?a.change, held = a.held, "skill review changed a skill");
                    }
                }
                Err(e) => tracing::warn!(%e, "skill review skipped"),
            }
        });
    }

    /// Score this session's skill loads by how it ended; a later review verdict overrides it.
    pub(super) fn record_skill_outcome(&self, result: &Result<SessionResult>) {
        let (Some(db), Ok(result)) = (self.telemetry_db.clone(), result) else {
            return;
        };
        let Some(score) = crate::skill_review::outcome_score(result) else {
            return;
        };
        let session_id = self.session_id.clone();
        tokio::spawn(async move {
            let source = crate::skill_review::OUTCOME_SOURCE_SESSION;
            let updated = db.with_conn(|c| {
                hq_db::skill_invocations::update_outcome_by_session(c, &session_id, score, source)
            });
            if let Err(e) = updated {
                tracing::warn!(%e, "failed to record skill outcome");
            }
        });
    }

    /// Auto-load skills whose hints match this instruction, once per session.
    ///
    /// Skill matching needs the user's instruction, which does not exist when
    /// `SessionBuilder` assembles the prompt — it passed an empty string, so
    /// the match loop was skipped and no skill had ever auto-loaded on this
    /// path. Running it here fixes that; latching after the first turn keeps
    /// the system prompt stable so the cached prefix survives.
    pub(super) fn enrich_with_matching_skills(&mut self, instruction: &str) {
        if self.skills_enriched || instruction.trim().is_empty() {
            return;
        }
        let (Some(index), Some(base)) = (self.skill_index.clone(), self.system_prompt.clone())
        else {
            return;
        };
        self.skills_enriched = true;

        let (enriched, loaded) = hq_tools::skills::enrich_system_prompt(
            &index,
            &base,
            instruction,
            None,
            Some(self.max_skill_tokens),
        );
        if loaded.is_empty() {
            return;
        }
        tracing::debug!(skills = ?loaded, "auto-loaded skills for this session");
        self.set_system_prompt(enriched);
        self.skills_in_play.extend(loaded.iter().cloned());

        // Record the auto-loads. Only explicit `load_skill` calls were ever
        // logged, so `skill_invocations` could not answer which skills were
        // actually earning their place in the catalog.
        if let Some(db) = self.telemetry_db.clone() {
            let session_id = self.session_id.clone();
            tokio::spawn(async move {
                for name in &loaded {
                    let result = db.with_conn(|conn| {
                        hq_db::skill_invocations::log_invocation(
                            conn,
                            name,
                            &session_id,
                            hq_db::skill_invocations::InvocationTrigger::AutoLoad,
                        )
                    });
                    if let Err(e) = result {
                        tracing::warn!(%e, skill = %name, "failed to log skill auto-load");
                    }
                }
            });
        }
    }
}
