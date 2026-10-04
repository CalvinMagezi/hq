//! Keeps low-value email FYIs (promotions, newsletters, digests) out of the
//! owner's chat. Only the FYI branch of `email_triage::handle` asks; a reply-needed
//! email never reaches this gate.

use hq_core::config::{DecisionMode, SITE_EMAIL_FYI};
use hq_core::redact::redact_secrets;
use hq_llm::decision::{DecisionRequest, Decisions};
use serde_json::json;
use std::sync::Arc;
use std::time::Instant;

/// Calibrated on real mail with `typesafe/jev-1.13`: promotions and newsletters
/// scored 0.05 to 0.16, mail that mattered scored 0.42 and up. Kept well below the
/// lowest borderline case because scores drift about 0.02 between identical calls.
const SUPPRESS_BELOW: f64 = 0.30;
const WORTH_KEY: &str = "worth_seeing";
const CATEGORY_KEY: &str = "category";
const FIELD_CHARS: usize = 120;

/// A message from a real person or about an account is never hidden, whatever the score.
const PROTECTED_CATEGORIES: [&str; 2] = ["personal_human", "account_or_security"];

const WORTH_INSTRUCTIONS: &str = "Does this email need the reader's attention: an action, a \
    decision, or news about their money, accounts, security, deadlines, or work? Marketing, \
    newsletters, digests, promotions, social notifications and routine automated updates do not.";

const CATEGORY_INSTRUCTIONS: &str = "What kind of email is this?";
const CATEGORIES: [(&str, &str); 6] = [
    (
        "promo_newsletter",
        "marketing, newsletters, digests, promotions, social or content recommendations",
    ),
    (
        "transaction_receipt",
        "order confirmations, receipts, delivery or refund notices",
    ),
    (
        PROTECTED_CATEGORIES[1],
        "sign-in, password, security, billing or subscription notices for an account",
    ),
    (
        "work_or_admin",
        "project tools, deadlines, tax, legal, government or organisational admin",
    ),
    (
        PROTECTED_CATEGORIES[0],
        "a message written by a real person to the reader",
    ),
    ("other", "anything else"),
];

/// True when this FYI should not be forwarded. Fails open: anything short of a
/// confident, enforced "not worth seeing" forwards the email as before.
///
/// The model sees only the sender and subject line, so a terse subject on an
/// important email is the failure mode; the category veto and the low threshold
/// exist for that reason.
pub async fn suppress_fyi(decisions: Option<&Arc<Decisions>>, from: &str, subject: &str) -> bool {
    let Some(decisions) = decisions else {
        return false;
    };
    let mode = decisions.mode(SITE_EMAIL_FYI);
    if mode == DecisionMode::Off {
        return false;
    }
    let (from, subject) = (clip(&redact_secrets(from)), clip(&redact_secrets(subject)));
    let request = DecisionRequest::new(&format!("From: {from}\nSubject: {subject}"))
        .noul(WORTH_KEY, WORTH_INSTRUCTIONS)
        .choice(CATEGORY_KEY, CATEGORY_INSTRUCTIONS, &CATEGORIES);
    let incumbent = json!({ "decision": "forward", "from": from, "subject": subject });
    if mode == DecisionMode::Shadow {
        decisions.shadow(SITE_EMAIL_FYI, request, incumbent);
        return false;
    }
    let started = Instant::now();
    let Ok(response) = decisions.ask(&request).await else {
        return false;
    };
    let (Ok(worth), Ok(category)) = (response.noul(WORTH_KEY), response.choice(CATEGORY_KEY))
    else {
        return false;
    };
    let low = worth < decisions.threshold(SITE_EMAIL_FYI, SUPPRESS_BELOW);
    let suppress = low && !PROTECTED_CATEGORIES.contains(&category);
    decisions.record(
        SITE_EMAIL_FYI,
        json!({
            "incumbent": incumbent,
            "action": if suppress { "suppressed" } else { "forwarded" },
            "score": worth,
            "category": category,
            "model": response.model,
            "cost": response.usage.cost,
            "latency_ms": started.elapsed().as_millis() as u64,
        }),
    );
    suppress
}

fn clip(text: &str) -> String {
    hq_core::text::truncate_chars(text, FIELD_CHARS).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hq_core::config::{DecisionSite, DecisionsConfig};
    use hq_llm::decision::{Answer, FakeDecisionProvider};
    use std::collections::BTreeMap;
    use std::path::Path;

    fn decisions(
        mode: DecisionMode,
        answer: Option<(f64, &str)>,
        dir: &Path,
    ) -> Option<Arc<Decisions>> {
        let mut config = DecisionsConfig::default();
        config.sites.insert(
            SITE_EMAIL_FYI.to_string(),
            DecisionSite {
                mode,
                threshold: None,
            },
        );
        let fake = FakeDecisionProvider {
            answers: answer
                .map(|(noul, category)| {
                    BTreeMap::from([
                        (WORTH_KEY.to_string(), Answer::Noul { noul }),
                        (
                            CATEGORY_KEY.to_string(),
                            Answer::Choice {
                                choice: category.to_string(),
                                probabilities: BTreeMap::new(),
                                confidence: 1.0,
                            },
                        ),
                    ])
                })
                .unwrap_or_default(),
            fail: answer.is_none(),
        };
        Some(Arc::new(Decisions::new(Arc::new(fake), config, dir)))
    }

    fn log(dir: &Path) -> String {
        std::fs::read_dir(dir.join("_system/decision-shadow"))
            .ok()
            .and_then(|mut d| d.next())
            .and_then(|e| std::fs::read_to_string(e.ok()?.path()).ok())
            .unwrap_or_default()
    }

    #[tokio::test]
    async fn confident_promo_is_suppressed_and_the_log_names_it() {
        let dir = tempfile::tempdir().unwrap();
        let d = decisions(DecisionMode::Enforce, Some((0.09, "promo_newsletter")), dir.path());
        assert!(suppress_fyi(d.as_ref(), "Glovo <no-reply@glovo.com>", "Top Sellers: PROMO").await);
        let line = log(dir.path());
        assert!(line.contains("\"action\":\"suppressed\""));
        assert!(line.contains("Glovo"));
        assert!(line.contains("Top Sellers"));
    }

    #[tokio::test]
    async fn people_accounts_and_worthwhile_mail_are_forwarded() {
        let dir = tempfile::tempdir().unwrap();
        let human = decisions(DecisionMode::Enforce, Some((0.05, "personal_human")), dir.path());
        assert!(!suppress_fyi(human.as_ref(), "A Person", "hi").await);
        let account = decisions(DecisionMode::Enforce, Some((0.1, "account_or_security")), dir.path());
        assert!(!suppress_fyi(account.as_ref(), "Bank", "New sign-in").await);
        let worthwhile = decisions(DecisionMode::Enforce, Some((0.84, "work_or_admin")), dir.path());
        assert!(!suppress_fyi(worthwhile.as_ref(), "", "Withholding Tax Credit Certificate").await);
        assert!(log(dir.path()).contains("\"action\":\"forwarded\""));
    }

    #[tokio::test]
    async fn errors_shadow_mode_and_missing_handle_all_forward() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!suppress_fyi(None, "x", "y").await);
        let failing = decisions(DecisionMode::Enforce, None, dir.path());
        assert!(!suppress_fyi(failing.as_ref(), "x", "y").await);
        let shadow = decisions(DecisionMode::Shadow, Some((0.0, "promo_newsletter")), dir.path());
        assert!(!suppress_fyi(shadow.as_ref(), "x", "y").await);
    }

    #[tokio::test]
    async fn secrets_in_the_subject_are_redacted_before_logging() {
        let dir = tempfile::tempdir().unwrap();
        let d = decisions(DecisionMode::Enforce, Some((0.02, "promo_newsletter")), dir.path());
        assert!(suppress_fyi(d.as_ref(), "x", "key sk-abcdefghijklmnopqrstuvwx").await);
        assert!(!log(dir.path()).contains("sk-abcdefghijklmnopqrstuvwx"));
    }
}
