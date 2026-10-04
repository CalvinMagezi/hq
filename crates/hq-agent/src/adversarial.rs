//! Adversarial critic agent — devil's advocate for plans and implementations.
//!
//! Uses FREE ultra-fast models (Cerebras 8b at 2200 tok/s, Groq 8b) to
//! automatically find flaws in plans and code before the human reviews.
//!
//! Design principles:
//! - Assumes flaws exist (counter-acts rubber-stamping tendency of small models)
//! - Structured JSON output (severity + category + description + suggestion)
//! - Non-fatal: if critic model is unavailable, pipeline continues
//! - Single rework loop for blockers (prevents cost runaway)

use anyhow::Result;
use tracing::debug;

use hq_core::types::{ChatMessage, MessageRole};
use hq_llm::provider::{ChatRequest, LlmProvider};

// The vocabulary moved to `hq-core` so `hq-tools` could reach it; re-exported
// here because four call sites import it from this path.
pub use hq_core::critic::{
    Concern, CriticResult, DIFF_CRITIC_SYSTEM, PROSE_CRITIC_SYSTEM, Severity, clip,
    parse_critic_response,
};

/// Review a real diff against acceptance criteria.
///
/// Reuses the same cheap `"critic"` model alias and tolerant JSON parsing as
/// the rest of this module.
pub async fn critique_diff(
    diff: &str,
    acceptance_criteria: &[String],
    test_results: &str,
    ledger: &str,
    provider: &dyn LlmProvider,
) -> Result<CriticResult> {
    let content = build_critique_prompt(diff, acceptance_criteria, test_results, ledger);
    run_critic(DIFF_CRITIC_SYSTEM, &content, provider).await
}

/// Runs the `/codereview` terminal command: critiques every uncommitted
/// change in `cwd` (staged, unstaged, and untracked files) and renders the
/// result via `review_verdict`. Returns `Ok(None)` when the working tree is
/// clean — there is nothing for the critic to read, so no verdict is formed.
///
/// Uses `LlmRouter::from_env()` on the cheap `"critic"` model alias.
pub async fn review_uncommitted_changes(cwd: &std::path::Path) -> Result<Option<(String, bool)>> {
    let diff = hq_tools::coding::git::uncommitted_diff(cwd).await?;
    if diff.trim().is_empty() {
        return Ok(None);
    }
    let provider = hq_llm::router::LlmRouter::from_env();
    let result = critique_diff(&diff, &[], "", "", &provider).await;
    Ok(Some(review_verdict(&result)))
}

/// Renders a critic result as a Markdown verdict section, with a bool for
/// whether it contains a blocker.
pub fn review_verdict(result: &anyhow::Result<CriticResult>) -> (String, bool) {
    let critique = match result {
        Ok(c) => c,
        Err(e) => {
            return (
                format!(
                    "## Review\n\nThe adversarial review could not be read ({e}), so this \
                     pull request opens as a draft.\n"
                ),
                true,
            );
        }
    };

    if critique.is_empty() {
        return (
            "## Review\n\nAn adversarial review of this diff raised no concerns.\n".to_string(),
            false,
        );
    }

    let mut out = String::from("## Review\n\n");
    for c in &critique.concerns {
        out.push_str(&format!(
            "- **{}** ({}): {}\n  Suggestion: {}\n",
            c.severity, c.category, c.description, c.suggestion,
        ));
    }
    (out, critique.has_blockers())
}

/// Review business-document prose (report/proposal/deliverable drafts) for
/// tone and structure. Callers should run `hq_core::prose_quality::SlopDetector`
/// first — this dimension deliberately skips banned-word/em-dash/emoji
/// checking, which the mechanical pass already covers.
pub async fn review_prose(draft: &str, provider: &dyn LlmProvider) -> Result<CriticResult> {
    let content = format!("## Draft\n{}", clip(draft, 6000));
    run_critic(PROSE_CRITIC_SYSTEM, &content, provider).await
}

/// Build the diff-review prompt. Split out from `critique_diff` so the ledger
/// section can be unit-tested without an `LlmProvider`.
fn build_critique_prompt(
    diff: &str,
    acceptance_criteria: &[String],
    test_results: &str,
    ledger: &str,
) -> String {
    const MAX_DIFF: usize = 24_000;
    let criteria = if acceptance_criteria.is_empty() {
        "(none stated — judge whether the change is coherent and complete)".to_string()
    } else {
        acceptance_criteria
            .iter()
            .map(|c| format!("- {c}"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let truncated: String = if diff.len() > MAX_DIFF {
        let cut = diff
            .char_indices()
            .map(|(i, _)| i)
            .take_while(|i| *i < MAX_DIFF)
            .last()
            .unwrap_or(0);
        format!("{}\n… diff truncated …", &diff[..cut])
    } else {
        diff.to_string()
    };
    let mut prompt = format!(
        "## Acceptance criteria\n{criteria}\n\n## Test results\n{}\n\n## Diff\n```diff\n{truncated}\n```",
        if test_results.trim().is_empty() {
            "(no tests were run)"
        } else {
            test_results
        }
    );
    // A caller with no ledger context passes "", which must not add an empty section.
    if !ledger.trim().is_empty() {
        prompt.push_str(&format!(
            "\n\n{ledger}\n\nThe step recorded the decisions above because its \
             instructions were silent. Judge whether the diff contains a \
             further decision of that kind that is not recorded here. If it \
             does, name it and cite the path:line where it appears.\n"
        ));
    }
    prompt
}

/// Core critic function. Calls the LLM and parses the response.
async fn run_critic(
    system_prompt: &str,
    content: &str,
    provider: &dyn LlmProvider,
) -> Result<CriticResult> {
    let request = ChatRequest {
        model: "critic".to_string(),
        messages: vec![
            ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::System,
                content: system_prompt.to_string(),
                tool_calls: Vec::new(),
                tool_call_id: None,
                reasoning_content: None,
            },
            ChatMessage {
                image_parts: Vec::new(),
                role: MessageRole::User,
                content: content.to_string(),
                tool_calls: Vec::new(),
                tool_call_id: None,
                reasoning_content: None,
            },
        ],
        temperature: Some(0.4), // Slightly higher for creative flaw-finding
        max_tokens: Some(1024),
        ..Default::default()
    };

    let response = provider.chat(&request).await?;
    debug!(
        tokens = response.input_tokens + response.output_tokens,
        "critic response received"
    );

    parse_critic_response(&response.message.content)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ledger_reaches_the_critic_prompt() {
        let prompt = build_critique_prompt(
            "diff --git a/x b/x",
            &["it compiles".to_string()],
            "tests passed",
            "## Assumptions\n\n- Decision: SQLite",
        );
        assert!(prompt.contains("SQLite"));
        assert!(prompt.contains("not recorded"));
        assert!(prompt.contains("The step recorded the decisions above"));
    }

    #[test]
    fn an_empty_ledger_does_not_add_an_empty_section() {
        let prompt = build_critique_prompt("d", &[], "t", "");
        // Anchored on text only the ledger branch emits, so deleting the guard fails this.
        assert!(!prompt.contains("The step recorded the decisions above"));
    }
}
