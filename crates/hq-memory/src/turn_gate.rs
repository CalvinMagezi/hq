//! Cheap durable-text gates in front of memory writes, so chatter, machine
//! output, and self-referential reflections never pay for an LLM extraction call
//! or crowd out curated memories.

use hq_core::config::{DecisionMode, SITE_MEMORY_TURN};
use hq_core::redact::redact_secrets;
use hq_llm::decision::{DecisionRequest, Decisions};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use std::time::Instant;

const QUESTION_KEY: &str = "is_durable";
const EXCERPT_CHARS: usize = 200;
const HASH_HEX_CHARS: usize = 16;

/// A text is skipped only when the model is this sure it is not durable.
const DEFAULT_SKIP_BELOW: f64 = 0.15;

struct Rubric {
    site: &'static str,
    instructions: &'static str,
}

const TURN: Rubric = Rubric {
    site: SITE_MEMORY_TURN,
    instructions: "Does this conversation turn state a lasting fact, decision, or personal \
        preference that is worth remembering in future sessions? Greetings, status chatter, \
        transient task talk, and machine output are not.",
};

/// True when a conversation turn should go on to ingestion.
pub async fn admit(decisions: Option<&Arc<Decisions>>, source_id: &str, turn: &str) -> bool {
    gate(&TURN, decisions, source_id, turn).await
}

/// Fails open: a disabled gate, a shadow-mode site, or any provider error all admit.
async fn gate(
    rubric: &Rubric,
    decisions: Option<&Arc<Decisions>>,
    source_id: &str,
    text: &str,
) -> bool {
    let Some(decisions) = decisions else {
        return true;
    };
    let mode = decisions.mode(rubric.site);
    if mode == DecisionMode::Off {
        return true;
    }
    let redacted = redact_secrets(text);
    let request = DecisionRequest::new(&redacted).noul(QUESTION_KEY, rubric.instructions);
    let incumbent = json!({ "decision": "keep", "source": source_id });
    if mode == DecisionMode::Shadow {
        decisions.shadow(rubric.site, request, incumbent);
        return true;
    }
    let started = Instant::now();
    let Ok(response) = decisions.ask(&request).await else {
        return true;
    };
    let Ok(durable) = response.noul(QUESTION_KEY) else {
        return true;
    };
    let skip = durable < decisions.threshold(rubric.site, DEFAULT_SKIP_BELOW);
    let mut entry = json!({
        "incumbent": incumbent,
        "action": if skip { "skipped" } else { "admitted" },
        "score": durable,
        "model": response.model,
        "cost": response.usage.cost,
        "latency_ms": started.elapsed().as_millis() as u64,
    });
    if skip {
        entry["excerpt"] = json!(redacted.chars().take(EXCERPT_CHARS).collect::<String>());
        // With the source id, this locates the exact text for replay.
        entry["text_hash"] = json!(text_hash(text));
    }
    decisions.record(rubric.site, entry);
    !skip
}

fn text_hash(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
        .chars()
        .take(HASH_HEX_CHARS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::{DecisionSite, DecisionsConfig};
    use hq_llm::decision::{Answer, FakeDecisionProvider};
    use std::collections::BTreeMap;
    use std::path::Path;

    fn gate_for(
        site: &str,
        mode: DecisionMode,
        durable: Option<f64>,
        dir: &Path,
    ) -> Option<Arc<Decisions>> {
        let mut config = DecisionsConfig::default();
        config.sites.insert(
            site.to_string(),
            DecisionSite {
                mode,
                threshold: None,
            },
        );
        let fake = FakeDecisionProvider {
            answers: durable
                .map(|noul| BTreeMap::from([(QUESTION_KEY.to_string(), Answer::Noul { noul })]))
                .unwrap_or_default(),
            fail: durable.is_none(),
        };
        Some(Arc::new(Decisions::new(Arc::new(fake), config, dir)))
    }

    fn logged(dir: &Path) -> String {
        let log_dir = dir.join("_system/decision-shadow");
        std::fs::read_dir(log_dir)
            .ok()
            .and_then(|mut d| d.next())
            .and_then(|e| std::fs::read_to_string(e.ok()?.path()).ok())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn no_handle_admits_everything() {
        assert!(admit(None, "s", "hello").await);
    }

    #[tokio::test]
    async fn enforce_skips_only_confident_ephemeral_turns_and_logs_both_outcomes() {
        let dir = tempfile::tempdir().unwrap();
        let low = gate_for(SITE_MEMORY_TURN, DecisionMode::Enforce, Some(0.02), dir.path());
        assert!(!admit(low.as_ref(), "chat:1", "ok thanks, key sk-abcdefghijklmnopqrstuvwx").await);
        let log = logged(dir.path());
        assert!(log.contains("\"action\":\"skipped\""));
        assert!(log.contains("\"text_hash\""));
        assert!(!log.contains("sk-abcdefghijklmnopqrstuvwx"), "excerpt must be redacted");

        let ambiguous = gate_for(SITE_MEMORY_TURN, DecisionMode::Enforce, Some(0.4), dir.path());
        assert!(admit(ambiguous.as_ref(), "chat:1", "maybe a preference").await);
        let durable = gate_for(SITE_MEMORY_TURN, DecisionMode::Enforce, Some(0.9), dir.path());
        assert!(admit(durable.as_ref(), "chat:1", "I always deploy on Fridays").await);
        assert!(logged(dir.path()).contains("\"action\":\"admitted\""));
    }

    #[tokio::test]
    async fn provider_errors_and_non_enforce_modes_admit() {
        let dir = tempfile::tempdir().unwrap();
        let failing = gate_for(SITE_MEMORY_TURN, DecisionMode::Enforce, None, dir.path());
        assert!(admit(failing.as_ref(), "s", "anything").await);
        let shadow = gate_for(SITE_MEMORY_TURN, DecisionMode::Shadow, Some(0.0), dir.path());
        assert!(admit(shadow.as_ref(), "s", "anything").await);
        let off = gate_for(SITE_MEMORY_TURN, DecisionMode::Off, Some(0.0), dir.path());
        assert!(admit(off.as_ref(), "s", "anything").await);
    }
}
