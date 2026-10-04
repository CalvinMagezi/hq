//! `prose_lint` — mechanical AI-slop check for a text draft.
//!
//! The callable half of the mechanical layer: `hq_core::prose_quality`
//! defines the rules, this exposes them as a tool the document-generation
//! skills (`~/.agents/skills/{docx,pptx,xlsx,pdf}/SKILL.md`) can call before
//! delivering a draft. The LLM-judged tone/structure layer this deliberately
//! does not re-check is `hq_core::critic::PROSE_CRITIC_SYSTEM`
//! (`hq_agent::adversarial::review_prose`), run in-process rather than as a
//! callable tool.

use anyhow::Result;
use async_trait::async_trait;
use hq_core::prose_quality::SlopDetector;
use serde_json::{Value, json};

use crate::registry::HqTool;

pub struct ProseLintTool;

#[async_trait]
impl HqTool for ProseLintTool {
    fn name(&self) -> &str {
        "prose_lint"
    }

    fn description(&self) -> &str {
        "Check text for mechanical AI-slop tells: banned words (leverage, robust, \
         seamless, ...), banned openers/closers, em-dashes, emoji/icons. Call this on \
         any draft before delivering a document (docx/pptx/xlsx/pdf, report, email). \
         Only catches mechanical tells; still re-read the draft yourself for tone and \
         structure issues regex can't catch."
    }

    fn parameters(&self) -> Value {
        json!({
            "type": "object",
            "required": ["text"],
            "properties": {
                "text": {
                    "type": "string",
                    "description": "The draft text to check."
                },
                "mode": {
                    "type": "string",
                    "enum": ["business", "fiction"],
                    "default": "business",
                    "description": "Rule set: 'business' for reports/proposals/documents, \
                                     'fiction' for prose/narrative writing."
                }
            }
        })
    }

    fn category(&self) -> &str {
        "quality"
    }

    fn is_read_only(&self) -> bool {
        true
    }

    async fn execute(&self, args: Value) -> Result<Value> {
        let text = args
            .get("text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required arg: text"))?;

        let mode = args
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("business");
        let detector = match mode {
            "fiction" => SlopDetector::fiction(),
            "business" => SlopDetector::business(),
            other => anyhow::bail!("invalid mode '{other}': use 'business' or 'fiction'"),
        };

        let violations = detector.detect(text);

        Ok(json!({
            "clean": violations.is_empty(),
            "violation_count": violations.len(),
            "violations": violations,
            "mode": mode,
        }))
    }
}

/// Create the prose-lint tool.
pub fn create_prose_lint_tools() -> Vec<Box<dyn HqTool>> {
    vec![Box::new(ProseLintTool)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn catches_a_banned_word() {
        let out = ProseLintTool
            .execute(json!({ "text": "We should leverage this." }))
            .await
            .unwrap();
        assert_eq!(out["clean"], false);
        assert!(out["violation_count"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn clean_text_reports_clean() {
        let out = ProseLintTool
            .execute(json!({ "text": "We shipped the feature on Tuesday." }))
            .await
            .unwrap();
        assert_eq!(out["clean"], true);
        assert_eq!(out["violation_count"], 0);
    }

    #[tokio::test]
    async fn rejects_unknown_mode() {
        let err = ProseLintTool
            .execute(json!({ "text": "x", "mode": "poetry" }))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("invalid mode"));
    }
}
