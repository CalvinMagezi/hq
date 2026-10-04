//! The vocabulary of a code review: severities, concerns, the prompts that ask
//! for them, and a parser tolerant of how models actually reply.
//!
//! This lives in `hq-core` rather than beside the critic that uses it because
//! two crates need it and they sit on opposite sides of a dependency edge:
//! `hq-agent` runs reviews through an `LlmProvider`, `hq-tools` asks a peer
//! agent for one, and `hq-agent` depends on `hq-tools`. Nothing here touches an
//! LLM, so nothing here needs to know which of the two is calling.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// Severity of a concern found by the adversarial critic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Must be fixed before proceeding. Triggers one rework loop.
    Blocker,
    /// Should be addressed but doesn't block. Reported to user.
    Warning,
    /// Minor suggestion. Reported to user.
    Nit,
    /// A severity the model invented ("critical", "major", "high"). Without
    /// this, one unrecognised value failed the whole `CriticResult`
    /// deserialization, and an unparseable review was treated as a clean one.
    #[serde(other)]
    Unknown,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Severity::Blocker => write!(f, "blocker"),
            Severity::Warning => write!(f, "warning"),
            Severity::Nit => write!(f, "nit"),
            Severity::Unknown => write!(f, "unknown"),
        }
    }
}

/// A single concern raised by the adversarial critic.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Concern {
    pub severity: Severity,
    pub category: String,
    pub description: String,
    pub suggestion: String,
}

impl Default for Concern {
    fn default() -> Self {
        Self {
            // A concern whose severity did not parse is not a nit to ignore.
            severity: Severity::Unknown,
            category: String::new(),
            description: String::new(),
            suggestion: String::new(),
        }
    }
}

/// Result of an adversarial critique.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CriticResult {
    pub concerns: Vec<Concern>,
}

impl CriticResult {
    pub fn blockers(&self) -> Vec<&Concern> {
        self.concerns
            .iter()
            .filter(|c| c.severity == Severity::Blocker)
            .collect()
    }

    pub fn warnings(&self) -> Vec<&Concern> {
        self.concerns
            .iter()
            .filter(|c| c.severity == Severity::Warning)
            .collect()
    }

    pub fn has_blockers(&self) -> bool {
        self.concerns
            .iter()
            .any(|c| c.severity == Severity::Blocker)
    }

    pub fn is_empty(&self) -> bool {
        self.concerns.is_empty()
    }

    /// Empty result (no concerns).
    pub fn clean() -> Self {
        Self {
            concerns: Vec::new(),
        }
    }
}

/// Truncate on a char boundary. Byte slicing LLM prose panics the moment a
/// multi-byte character straddles the cut.
pub fn clip(s: &str, max_bytes: usize) -> &str {
    &s[..s.floor_char_boundary(max_bytes)]
}

/// System prompt for reviewing an actual diff against acceptance criteria.
///
/// Reviewing the diff is the difference between "the agent
/// says it did the work" and "the work is in the tree".
pub const DIFF_CRITIC_SYSTEM: &str = r#"You are reviewing a code change against its acceptance criteria, as a senior engineer would review a pull request.

You are given: the acceptance criteria, the unified diff of what actually changed, and the results of any tests that were run.

Judge ONLY what the diff shows. Do not speculate about code you cannot see.

Rules:
- Raise a `blocker` only when an acceptance criterion is demonstrably NOT met, or the change introduces a clear defect (data loss, security hole, breakage).
- EVERY blocker MUST cite a concrete `path:line` from the diff in its description. A blocker without a citation cannot be acted on automatically and will be escalated to a human instead, so always cite one rather than downgrading a real blocker to a warning.
- Style preferences, naming, and speculative concerns are `nit` at most.
- If the criteria are met, return an empty concerns array. Approving good work is the correct outcome.

Respond with JSON only:
{"concerns":[{"severity":"blocker|warning|nit","category":"correctness|security|scope|tests","description":"what is wrong, citing path:line","suggestion":"what to do about it"}]}"#;

/// System prompt for reviewing business prose (reports, proposals, document
/// drafts) for tone and structure, not correctness. Deliberately does not
/// re-hunt banned words/em-dashes/emoji — `hq_core::prose_quality::SlopDetector`
/// already ran that mechanical pass; this dimension covers what regex can't:
/// tone, structural monotony, whether the voice fits the target brand.
pub const PROSE_CRITIC_SYSTEM: &str = "\
You are an adversarial editor reviewing business-document prose (reports, proposals, client-facing drafts).
A mechanical lint already removed banned AI-cliché words, em-dashes, and emoji — do NOT re-flag those.
ASSUME the draft has problems and find them. Be specific, quoting the offending sentence.

Focus on:
- Over-enthusiastic or apologetic tone (\"the over-eager assistant\")
- Structural monotony: bullets or sentences that all share the same length or grammatical shape
- Sweeping, unsupported claims (\"this will transform...\", \"industry-leading...\")
- Passive-voice hedging that obscures who decided or did what
- Voice that doesn't fit a professional client deliverable (too casual, too stiff, generic corporate boilerplate)
- Padding: sentences that restate the previous one without adding information

Respond ONLY with this JSON format (no other text):
{\"concerns\": [{\"severity\": \"warning\", \"category\": \"tone\", \"description\": \"Sentence 3 makes an unsupported claim: '...'\", \"suggestion\": \"State the specific, verifiable outcome instead\"}]}

Severity levels:
- blocker: makes the document unusable as-is (factually unsupported claims presented as fact, tone that would embarrass the sender)
- warning: should fix before sending (structural monotony, hedging, mild over-enthusiasm)
- nit: minor polish

Keep total response under 400 words. If the prose is solid, return {\"concerns\": []}.";

/// Parse the critic's JSON response. Handles markdown code blocks and raw JSON.
pub fn parse_critic_response(text: &str) -> Result<CriticResult> {
    // Try direct JSON parse
    if let Ok(result) = serde_json::from_str::<CriticResult>(text) {
        return Ok(result);
    }

    // Try extracting from ```json ... ``` block
    if let Some(start) = text.find("```json") {
        let after = &text[start + 7..];
        if let Some(end) = after.find("```")
            && let Ok(result) = serde_json::from_str::<CriticResult>(after[..end].trim())
        {
            return Ok(result);
        }
    }

    // Try extracting from ``` ... ``` block
    if let Some(start) = text.find("```") {
        let after = &text[start + 3..];
        let after = if let Some(nl) = after.find('\n') {
            &after[nl + 1..]
        } else {
            after
        };
        if let Some(end) = after.find("```")
            && let Ok(result) = serde_json::from_str::<CriticResult>(after[..end].trim())
        {
            return Ok(result);
        }
    }

    // Try finding JSON object in the text
    if let Some(start) = text.find("{\"concerns\"") {
        let candidate = &text[start..];
        // Find matching closing brace
        let mut depth = 0;
        let mut end_pos = 0;
        for (i, ch) in candidate.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end_pos = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        if end_pos > 0
            && let Ok(result) = serde_json::from_str::<CriticResult>(&candidate[..end_pos])
        {
            return Ok(result);
        }
    }

    // Last resort: scan every balanced object in the text. Models routinely
    // pretty-print, so the literal `{"concerns"` probe above misses
    // `{\n  "concerns": …}` embedded in prose.
    for (start, _) in text.char_indices().filter(|(_, c)| *c == '{') {
        let candidate = &text[start..];
        let mut depth = 0;
        for (i, ch) in candidate.char_indices() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        if let Ok(result) = serde_json::from_str::<CriticResult>(&candidate[..=i]) {
                            return Ok(result);
                        }
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    // Never claim a review was clean because it could not be read. Callers
    // treat an error as "the review did not happen", which for a code diff
    // means escalate — silently accepting an unreadable verdict is how a real
    // blocker ships.
    warn!("Could not parse critic response: {}", clip(text, 200));
    anyhow::bail!("critic response could not be parsed as a review verdict")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_clean_response() {
        let result = parse_critic_response(r#"{"concerns": []}"#).unwrap();
        assert!(result.is_empty());
        assert!(!result.has_blockers());
    }

    #[test]
    fn parse_response_with_concerns() {
        let json = r#"{"concerns": [
            {"severity": "blocker", "category": "security", "description": "SQL injection", "suggestion": "Use params"},
            {"severity": "warning", "category": "testing", "description": "No test for edge case", "suggestion": "Add test"},
            {"severity": "nit", "category": "style", "description": "Long function", "suggestion": "Split"}
        ]}"#;
        let result = parse_critic_response(json).unwrap();
        assert_eq!(result.concerns.len(), 3);
        assert_eq!(result.blockers().len(), 1);
        assert_eq!(result.warnings().len(), 1);
        assert!(result.has_blockers());
    }

    #[test]
    fn parse_response_from_markdown_block() {
        let md = "Here are my findings:\n```json\n{\"concerns\": [{\"severity\": \"warning\", \"category\": \"edge-case\", \"description\": \"Missing timeout\", \"suggestion\": \"Add 30s timeout\"}]}\n```";
        let result = parse_critic_response(md).unwrap();
        assert_eq!(result.concerns.len(), 1);
        assert_eq!(result.warnings().len(), 1);
    }

    #[test]
    fn an_unparseable_review_is_an_error_not_a_clean_verdict() {
        // Returning `clean()` here made an unreadable review indistinguishable
        // from an approving one, so a blocker the model actually raised could
        // ship. Callers treat the error as "the review did not happen".
        assert!(parse_critic_response("I think the code looks fine!").is_err());
    }

    #[test]
    fn a_pretty_printed_verdict_in_prose_still_parses() {
        let raw = "Here is my review:\n\n{\n  \"concerns\": [\n    {\n      \"severity\": \"blocker\",\n      \"category\": \"bug\",\n      \"description\": \"npe at src/a.rs:12\",\n      \"suggestion\": \"guard it\"\n    }\n  ]\n}\n\nHope that helps.";
        let result = parse_critic_response(raw).unwrap();
        assert_eq!(result.concerns.len(), 1);
        assert!(result.has_blockers());
    }

    #[test]
    fn an_invented_severity_does_not_discard_the_whole_review() {
        // One unrecognised value used to fail the entire deserialization,
        // which the old fallback then reported as clean.
        let raw = r#"{"concerns":[{"severity":"critical","category":"sec","description":"sqli at src/db.rs:88","suggestion":"bind params"}]}"#;
        let result = parse_critic_response(raw).unwrap();
        assert_eq!(result.concerns.len(), 1);
        assert_eq!(result.concerns[0].severity, Severity::Unknown);
    }

    #[test]
    fn parse_embedded_json_in_prose() {
        let text = "After careful review, {\"concerns\": [{\"severity\": \"blocker\", \"category\": \"bug\", \"description\": \"Off by one\", \"suggestion\": \"Fix loop\"}]} is my assessment.";
        let result = parse_critic_response(text).unwrap();
        assert_eq!(result.blockers().len(), 1);
    }

    #[test]
    fn severity_serialization() {
        let json = serde_json::to_string(&Severity::Blocker).unwrap();
        assert_eq!(json, "\"blocker\"");
        let parsed: Severity = serde_json::from_str("\"warning\"").unwrap();
        assert_eq!(parsed, Severity::Warning);
    }
}
