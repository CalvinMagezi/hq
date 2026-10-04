//! Diagnostic against the real endpoint, not a regression gate. Run with:
//! `OPENROUTER_API_KEY=... cargo test -p hq-llm --test decision_live -- --ignored`

use hq_core::config::DecisionRoute;
use hq_llm::decision::{DecisionProvider, DecisionRequest, HttpDecisionProvider};
use std::time::Duration;

#[tokio::test]
#[ignore = "needs OPENROUTER_API_KEY and network access"]
async fn live_route_answers_noul_and_choice() {
    let key = std::env::var("OPENROUTER_API_KEY").expect("OPENROUTER_API_KEY");
    let provider = HttpDecisionProvider::new(DecisionRoute::default(), key, Duration::from_secs(5));
    let request = DecisionRequest::new("Weekly newsletter: 5 product updates from a SaaS vendor.")
        .noul("should_interrupt", "Should the owner be interrupted right now?")
        .choice(
            "urgency",
            "How urgent is this message?",
            &[("routine", "newsletters"), ("actionable", "needs a reply"), ("critical", "outage")],
        );
    let response = provider.decide(&request).await.expect("live decision");
    assert!(response.noul("should_interrupt").unwrap() < 0.5);
    assert_eq!(response.choice("urgency").unwrap(), "routine");
}
